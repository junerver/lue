# Rust 渐进式重构路线图

本分支(`feat/rust-rewrite`)的目标:在不重写整个应用的前提下,用 Rust 逐步替换
lue 的性能热点,实现可量化的提速;每步只替换真正卡顿的组件,保留纯 Python
回退路径,确保任何一步出错都不会影响功能。

## 原则

1. **可选加速,绝不硬依赖**:所有替换点都走 `lue/_rust.py` 的 `lue_rs` 探针,
   扩展未安装时自动回退到 `content_parser`/`reader` 中保留的纯 Python 实现。
2. **行为等价,测试锁定**:`tests/test_rust_parity.py` 同时包含
   - characterization 用例(锁定公开 API 的可观察行为,不依赖任何后端);
   - parity 用例(Rust 扩展 vs 纯 Python 逐函数对比,含随机 fuzz)。
   `lue_rs` 未构建时 parity 自动跳过,characterization 依然全跑(回退路径)。
3. **先测后写**:Rust 实现以 Python 现有行为为准绳,不"改良"语义。

## 阶段 1(已完成):文本管线三件套

已用 PyO3 实现并接入(见 `rust/src/lib.rs`):

| lue_rs 函数 | 对应 Python | 说明 |
|---|---|---|
| `split_sentences` / `split_sentences_batch` | `content_parser.split_into_sentences` | 手写字符扫描复刻 2 次缩写/首字母保护 + 4 分支切分,无正则 |
| `clean_visual_text` | `content_parser.clean_visual_text` | 25 条正则 + Unicode 替换表 + 全角空白 trim,逐字对齐 Python 语义 |
| `clean_txt_doc_lines` | `_extract_content_txt` 逐行清洗 | 批量:一次 FFI 调用处理全书 |
| `analyze_cjk` | 中文占比/平均行长统计 | 批量统计 |
| `pick_toc_rule` | `_pick_toc_rule` | legado 13 规则的评分选择;规则 0 的「第X章」分支用纯 NFA 快速路径先行 |
| `collect_title_lines` | `_collect_title_lines` | fancy-regex 完整扫描,字节偏移二分复刻 Python 字符偏移 bisect |
| `split_long_text_paragraphs` | `_split_long_text_into_paragraphs` | 标点零宽切分 + 长度兜底 |
| `wrap_paragraph` | `rich Text.wrap`(纯文本路径) | cell 表二分 + 分词贪心 + 硬折 + rstrip/pad/set_cell_size 后处理,与 rich 逐字节一致 |
| `char_cell_width_py` / `cell_len_py` / `divide_line_py` / `expand_tabs_py` | 诊断导出 | 供 parity 调试与未来测试使用 |

配套改动:
- `lue/_rust.py` — 可选扩展探针(import 失败则 `lue_rs = None`)。
- `lue/content_parser.py` — 各热点入口改为「Rust 优先,失败回退 Python」。
- `lue/reader.py` — `total_sentences` 的整书统计走 `split_sentences_batch`。
- `tests/test_rust_parity.py` — characterization + parity 双向校验。
- `benchmarks/bench_parse.py` — 双实现基准(见下)。

### 阶段 1 基准(本机,Windows,Python 3.13,2.6 MB 中文小说 TXT)

| 任务 | Python | Rust | 提速 |
|---|---|---|---|
| `clean_visual_text`(全部行) | 2.4 s | 0.65 s | **3.7x** |
| `split_into_sentences`(全部段) | 0.76 s | 0.14 s | **5.4x** |
| `_pick_toc_rule`(512K 采样) | 8.7 ms | 3.1 ms | **2.8x** |
| `_extract_content_txt`(完整管线) | 2.49 s | 0.81 s | **3.1x** |

即打开一本 2.6 MB 的书,解析阶段从约 2.5 秒降到约 0.8 秒(约快 1.7 秒),
且全部结果与纯 Python 逐项相等(parity 测试覆盖)。

### 构建 / 安装 / 测试

```bash
# 构建并安装到当前 Python 环境(需要 Rust 工具链)
cd rust && python -m maturin build --release
pip install target/wheels/lue_rs-*.whl

# 测试(扩展不装也跑,只是 parity 部分跳过)
cd .. && python -m pytest tests/ -q

# 基准(需已安装扩展)
python benchmarks/bench_parse.py
```

## 阶段 2(已完成):文档布局 / 换行

`ui.update_document_layout` 中 O(全书) 的 `rich.Text.wrap` 是打开书时的最大
卡点(2.6 MB 书约 2.5 s)。`lue_rs.wrap_paragraph` 在 Rust 中逐语义复刻了 rich
的换行管线,与 rich 输出**逐字节一致**:

- `rich.cells.CELL_WIDTHS` 450 条宽度区间表(由 Python 侧生成 `cell_table.rs`,
  与 rich 同表二分查找);
- `Text.expand_tabs`(tab 按 cell 补齐到 8 的倍数);
- `rich._wrap.divide_line`(`\s*\S+\s*` 分词 + cell 贪心 + 超长词 `chop_cells`
  硬折,断点偏移随折叠推进);
- 每行后处理 `rstrip_end` → `truncate(width, pad=True)`(不足补空格;
  `set_cell_size` 把跨界宽字符替换为一个空格),对应 justify="left" 的行为。

接线点(均带回退,rich 路径完整保留):
- `ui.update_document_layout`(打开书/宽度变化时的整书 wrap);
- `reader.py` 点击跳转处的单段 wrap。

**注意**:`ui.get_visible_content` 每帧的高亮段落 wrap(`ui.py:423`)带样式
span,rich 的 divide 会把 span 分配到各行,Rust 纯文本版不能替换 —— 该路径
保留 rich(后续如需优化,应把断行缓存复用而非替换渲染)。

### 阶段 2 基准(同上书,72,000 段,宽 100)

| 任务 | rich | Rust | 提速 |
|---|---|---|---|
| `Text.wrap`(全部段落) | 2.47 s | 0.28 s | **~9x**(等价输出) |
| 句子切分 batch(布局输入) | 0.91 s(逐段) | 0.21 s | 4.3x |
| 句子→行双指针映射(Python) | 0.09 s | — | (阶段 3 候选) |

打开一本 2.6 MB 书的完整成本(解析 + 布局)约为
0.8 s + 0.6 s ≈ **1.4 s**,纯 Python 约 5.2 s,**~3.7x**。

等价性验证:
- `tests/test_rust_parity.py`:手工边角用例 + 300 段随机 fuzz × 3 种宽度;
- 整书 72,000 段 × 3 宽度 = 216,000 次 wrap 对比,**0 差异**;
- 布局级 parity:同一本书在 Rust 路径与 rich 回退路径下 `document_lines`
  与 `position_to_line` 完全一致。

## 阶段 3(已完成):进度锚点索引化

`reader._get_topmost_visible_sentence` 每次导航按键和每 5 秒的进度保存都会
对 `position_to_line`(全书每个句子一条)做 O(全书) 的 dict 双扫描,大书实测
20 ms/次。现在:

- `ui.update_document_layout` 构建布局时顺手产出按行排序的
  `(positions, lines)` 平行数组(稳定排序保持插入序,同一行取插入最早的
  句子位置,与原扫描语义一致),随布局宽度缓存;
- 查询改为 `bisect`:视口内有句子取该行第一个条目;视口为空取视口之前
  最晚行的第一个条目;换书时随 `_load_content` 一并重置;
- **20 ms → 0.4 µs**(5 万倍),索引构建利用 Timsort 对布局顺序输入的线性
  特性,仅 +32 ms(500k 句)。

等价性:`tests/test_reader_index.py` 用原实现的逐字拷贝作 oracle,500 个
随机滚动位置 + 全部边角(书顶、书尾、视口在内容前/后、同行多句、空书)。
测量也纠正了一个预判:每帧的高亮段落构建 + wrap 实测仅 0.29 ms,不需要
优化;`get_visible_content` 保持原样。

## 阶段 4(已完成):TXT 随机访问索引 + 惰性章节

受 legado `TextFile.kt` 的 byte-offset chapter slice 启发，TXT 的主路径
(中文、每行一段、标题分章)不再把全书读进 Python `str` / `List[List[str]]`：

- Rust `build_txt_index(path)`一次扫描原始字节，探测编码(UTF-8/UTF-8 BOM/
  GB18030/Latin-1)，复用 legado TOC 评分规则，产出章节 `(start,end)` **原始
  字节区间**；
- `TxtIndexCache`按 `(canonical path, mtime_ns, size)` 持久化编码与区间，缓存
  是可丢失的 derived data；
- Python `LazyChapters`保持原有序列接口，按需 `seek + read + decode + clean`
  单章，LRU 仅留 64 章；窗口布局自然只解码当前章邻居；
- 不符合该安全轮廓的 TXT(无标题、英文合段、超长单行)和其他格式完整保留
  全量解析回退，行为不回归。

真实 WSL 测试(`高考.txt`,11.7MB,1097章)：首次建立索引约 269ms；索引缓存
命中时打开当前 3 章窗口约 **14ms**；从首章跳到第 500 章窗口 **7ms**，返回
首章 **1ms**。这使主打开路径近似 `O(index lookup + current window)`，而不是
`O(book size + whole-book layout)`。

等价性:`tests/test_lazy_index.py`对 UTF-8、GB18030、UTF-8 BOM 的标题分章
小说逐章对比 `LazyChapters` 与原全量 `_extract_content_txt` 输出；同时覆盖
前言、切片、LRU 触发。所有 136 pytest 用例通过。

## 阶段 5(可选):剩余候选

`input_handler` 的转义序列解析、逐帧 ANSI 渲染、`timing_calculator` 的
词对齐模糊匹配。渲染部分是 I/O 密集,收益有限;输入解析体量小,适合整体搬。

## 阶段 5(远期):完全重写

届时 `lue_rs` 演进为完整的 Rust 渲染/状态核心,Python 薄壳只做 TTS 插件与
CLI 胶水(或最终用 cargo 直接产出可执行文件)。TTS(edge-tts/kokoro)保持
Python 侧,不建议迁移。

## 已知边界与安全说明

- `pick_toc_rule` 的 NFA 快速路径:fast 匹配集是完整 fancy 规则 0「第X章」分支
  的超集(省略了 `(?!课)`/`(?![合和])` 两个前瞻),只在快速评分已判定规则 0
  胜出时提前返回;该结论在完整扫描(超集匹配)下同样成立,方向保守。
  `collect_title_lines` 始终使用完整 fancy 扫描,结果严格等价。
- `split_sentences` 的手写保护扫描逐字符复刻两个正则的子集语义
  (`\b(abbrev)\.` 与 `\b[A-Z]\.(?=\s[A-Z])`),两者互斥,单遍等价;
  边界(串首、标点紧邻)由 parity fuzz 兜底验证。
- 纯字节偏移的 line-start 二分与 Python 字符偏移的 `bisect_right` 一致:
  相同匹配起点落在同一行。