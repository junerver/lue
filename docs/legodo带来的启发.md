可以，而且我认为 **legado-E 给我们的最大启发，会改变前面“先 Rust Layout Engine，再 Rust UI”的优先级**。

我重新看了 `legado-E` 的仓库、近期 PR/Release 信息，并结合 Lue 当前的设计后，我会把方案调整成：

> **先建立“超大文档的懒加载 + 持久化索引 + 增量排版”核心，再迁移 UI。**
>
> Rust 的价值不是单纯把 Python 算得更快，而是让 Lue 从“把整本书变成内存对象”转变为“把整本书视为一个可随机访问的数据集”。

`legado-E` 本身是 Legado 的继承项目，核心阅读能力包括本地 TXT/EPUB、可调排版、多种翻页模式等；仓库近期也仍然在处理“大文件 EPUB 无法导入”等问题，说明它确实面对过大型文档场景。([GitHub][1])

---

# 先说结论：我们应该借鉴的是“数据访问模型”，不是 Android UI

我会把两个项目抽象成：

```text
                 Lue
                  │
        ┌─────────┴─────────┐
        │                   │
     Python               Terminal
     today                  UI
        │                   │
        └──────┬────────────┘
               │
        ┌──────▼──────┐
        │ Reader Core │
        │    Rust     │
        └──────┬──────┘
               │
       ┌───────┼────────┐
       │       │        │
    Storage  Index    Layout
       │       │        │
       └───────┼────────┘
               │
          Book on disk
```

而不是：

```text
EPUB
 ↓
整个解析
 ↓
整个转换
 ↓
整个 layout
 ↓
Vec<Line>
 ↓
UI
```

后者正是“大书秒开”最大的敌人。

---

# 1. “秒开任意规模书籍”的关键其实不是 CPU

这是最重要的一点。

假设一本 TXT：

```text
1 GB
```

即使 Rust 比 Python 快 10 倍：

```text
Python：10 秒
Rust：1 秒
```

这仍然不是“秒开”。

真正的解决办法是：

```text
不要读取 1 GB
```

打开时只需要：

```text
metadata
+
TOC
+
当前位置附近的数据
```

也就是：

```text
1 GB book

       ┌─────────────────────────────────────────┐
       │                                         │
       │                                         │
       │              entire book                │
       │                                         │
       │                        ▲                │
       │                        │                │
       └────────────────────────┼────────────────┘
                                │
                         current viewport
```

启动成本从：

```text
O(book_size)
```

变成：

```text
O(metadata + index lookup + viewport)
```

这才是真正的“秒开”。

---

# 2. 这里应该借鉴 legado 的“分层数据模型”

我会把 Lue 的 Rust Core 设计成四层。

```text
┌─────────────────────────────┐
│        Book Metadata        │
│ title / author / format     │
└──────────────┬──────────────┘
               │
┌──────────────▼──────────────┐
│       Structural Index      │
│ chapter / paragraph / etc.  │
└──────────────┬──────────────┘
               │
┌──────────────▼──────────────┐
│       Layout Index           │
│ visual line / page / cell   │
└──────────────┬──────────────┘
               │
┌──────────────▼──────────────┐
│        Source Content       │
│ EPUB/TXT/PDF/etc.           │
└─────────────────────────────┘
```

而不是把：

```text
source + parsed model + layout
```

全部放进 RAM。

---

# 3. 最值得借鉴的是“持久化 cache”

这里其实有非常强的旁证。

一些高性能阅读器/固件采用的思路也是：

```text
book
 ↓
parse
 ↓
layout
 ↓
persistent cache
```

例如 CrossPoint 的架构明确把 EPUB metadata、CSS rules、section layout 持久化到磁盘，后续打开直接复用，而不是重新解析和排版。([GitHub][2])

这对 Lue 非常重要。

---

# 4. 我会让 Rust Core 拥有一个 Book Cache

例如：

```text
~/.cache/lue/books/
    └── <book-id>/
          ├── meta.bin
          ├── source.idx
          ├── chapters.idx
          ├── layout/
          │    ├── 80.bin
          │    ├── 100.bin
          │    └── 120.bin
          └── version
```

这里的：

```text
80.bin
```

代表：

> terminal width = 80 时的 layout。

```text
120.bin
```

代表：

> terminal width = 120 时的 layout。

这非常关键，因为：

```text
terminal width
```

改变以后，**visual layout 会改变**。

---

# 5. 但不要缓存整个“终端画面”

不要这样：

```text
cache/
  page0001.ansi
  page0002.ansi
  page0003.ansi
```

因为主题改变：

```text
dark → light
```

ANSI 全部失效。

字体/宽度变化：

```text
80 → 120
```

也全部失效。

应该缓存：

```text
semantic layout
```

例如：

```rust
struct VisualLine {
    source_start: u64,
    source_end: u64,

    word_start: u32,
    word_end: u32,

    width: u16,
}
```

然后 UI 每次：

```text
VisualLine
   ↓
theme
   ↓
Ratatui Cell
```

这样主题变化不会导致整个 layout 重建。

---

# 6. 第二个巨大改进：不要把整本 TXT 读进 String

这是我现在认为 Lue Rust 化最应该做的事情。

现在典型 Python reader 很容易走向：

```python
text = file.read()
```

然后：

```text
text
 ↓
chapters
 ↓
paragraphs
 ↓
sentences
 ↓
words
 ↓
lines
```

内存会非常恐怖。

Rust 应该改成：

```text
BookFile
    │
    ▼
memory mapped / random access
    │
    ├── offset 0
    ├── offset 4096
    ├── offset 8192
    ├── ...
    └── offset N
```

对于 TXT：

```rust
struct TextIndex {
    line_offsets: Vec<u64>,
}
```

甚至进一步：

```rust
struct BlockIndex {
    file_offset: u64,
    decoded_len: u32,
}
```

这样：

```text
打开 10GB TXT
```

不需要：

```text
10GB RAM
```

只需要：

```text
几个 MB index
+
当前 viewport
```

---

# 7. 这里还有一个非常重要的优化：分层索引

不要直接建立：

```text
每一个字符的 offset
```

太浪费。

我更推荐：

```text
Level 0
Book
 │
 ├── Chapter 0
 ├── Chapter 1
 └── Chapter 2

Level 1
Chapter
 │
 ├── Block 0
 ├── Block 1
 ├── Block 2
 └── ...

Level 2
Block
 │
 ├── paragraph
 ├── paragraph
 └── ...
```

例如每个 block：

```text
256 KB
```

那么：

```text
1 GB / 256 KB ≈ 4096 blocks
```

只需要一个非常小的 index。

---

# 8. 于是“跳到 90%”也不需要扫描全文

这是大型阅读器和普通文本阅读器的区别。

普通实现：

```text
goto 90%
 ↓
seek(90% file)
 ↓
向前扫描
 ↓
找 paragraph
 ↓
重新 layout
```

更好的：

```text
90%
 ↓
BlockIndex
 ↓
block #3686
 ↓
seek()
 ↓
decode only local block
 ↓
layout viewport
```

于是：

```text
O(N)
```

变成近似：

```text
O(log N) + O(viewport)
```

---

# 9. EPUB 要采用不同策略

TXT 和 EPUB 不应该强行使用同一种模型。

TXT：

```text
byte offset
 ↓
text block
 ↓
line
```

EPUB：

```text
container.xml
 ↓
OPF
 ↓
manifest
 ↓
spine
 ↓
chapter.xhtml
```

所以 EPUB 应该缓存：

```text
Book
 ├── metadata
 ├── spine
 ├── toc
 └── sections
       ├── section0
       ├── section1
       ├── section2
       └── ...
```

打开 EPUB 时：

```text
读取 metadata
+
读取 spine
+
读取 TOC
+
读取当前 section
```

而不是：

```text
解压整个 EPUB
+
解析所有 XHTML
+
建立完整 DOM
```

---

# 10. EPUB 的 cache 可以做到更漂亮

例如：

```text
book-cache/
    abc123/
       book.bin
       toc.bin
       spine.bin

       section/
          0000.bin
          0001.bin
          0002.bin

       layout/
          0000-120.bin
          0001-120.bin
```

这里：

```text
0001-120.bin
```

表示：

```text
section 1
terminal width 120
```

对应的 layout。

这样：

```text
第一次打开
```

只计算当前 section。

用户往后读：

```text
section 2
```

后台再计算：

```text
section 2 layout
```

---

# 11. 这就引出了一个非常重要的概念：后台渐进式预取

最终 Lue 应该类似：

```text
                 Reader
                   │
          current viewport
                   │
                   ▼
             ┌──────────┐
             │  visible │
             └────┬─────┘
                  │
        ┌─────────┴─────────┐
        │                   │
      behind              ahead
        │                   │
      cache               preload
```

例如用户当前：

```text
chapter 10
page 42
```

系统后台：

```text
priority 0: page 42
priority 1: page 43
priority 1: page 41
priority 2: page 44
priority 2: page 40
priority 3: chapter 11
```

这会让用户感觉：

> “整本书已经打开了。”

实际上：

> **根本没有打开整本书。**

---

# 12. 这比单纯 Rust 化 UI 重要一个数量级

所以我会修改之前的路线。

之前：

```text
① profiling
② Rust layout
③ Rust reader
④ Rust UI
⑤ Rust parser
```

现在我建议：

```text
① profiling
② Rust Book Storage / Index
③ Rust Lazy Reader
④ Rust Layout Cache
⑤ Rust Reader State
⑥ Rust TUI
⑦ Parser migration
```

也就是说：

> **UI 反而往后放。**

---

# 13. 新的核心架构

我会设计成：

```text
                         CLI
                          │
                          ▼
                  ┌──────────────┐
                  │ Reader App   │
                  └──────┬───────┘
                         │
              ┌──────────▼──────────┐
              │     Reader Core     │
              │        Rust         │
              └──────────┬──────────┘
                         │
        ┌────────────────┼────────────────┐
        │                │                │
        ▼                ▼                ▼
   Book Store         Indexer          Layout
        │                │                │
        ▼                ▼                ▼
   File/Archive     Chapter Index    Visual Lines
   Random Access    Paragraph Index  Word Mapping
        │                │                │
        └────────────────┼────────────────┘
                         │
                         ▼
                   Reader State
                         │
                         ▼
                     Ratatui
                         │
                         ▼
                    Crossterm
```

---

# 14. Reader Core 应该变成整个项目的“核心资产”

最终 Python 应该只是一个 adapter。

例如：

```text
lue/
├── python/
│
└── rust/
    ├── lue-core/
    │
    ├── lue-document/
    │
    ├── lue-index/
    │
    ├── lue-layout/
    │
    ├── lue-cache/
    │
    └── lue-tui/
```

其中：

```text
lue-core
```

不应该知道：

```text
Ratatui
Terminal
ANSI
```

它只知道：

```text
Document
Position
Viewport
Layout
Highlight
```

---

# 15. 这会带来一个非常好的测试方式

你甚至可以完全脱离终端测试。

例如：

```rust
let book = Book::open(path)?;

let position = book.seek_percent(0.9)?;

assert_eq!(position.chapter, 83);
```

然后：

```rust
let viewport = reader.viewport();

let lines = layout.render(viewport);
```

测试：

```text
输入
 ↓
Book
 ↓
Index
 ↓
Position
 ↓
Layout
```

完全不涉及 TUI。

---

# 16. 性能目标应该重新定义

我建议不要用：

> “Rust 比 Python 快多少？”

作为目标。

应该定义：

### 打开

```text
1 MB     < 100 ms
100 MB   < 100 ms
1 GB     < 200 ms
10 GB    < 500 ms
```

这里当然只是**工程目标**，实际数字需要 benchmark 后调整。

关键是：

```text
open time ≈ independent of book size
```

而不是：

```text
open time ∝ book size
```

---

# 17. 翻页

目标：

```text
next page
< 1 frame
```

理想状态：

```text
cache hit

viewport
   ↓
layout cache
   ↓
Ratatui
```

不进行：

```text
parse
layout
allocation
```

---

# 18. 首次打开和第二次打开应该分别优化

这是 `legado` 类项目特别值得借鉴的思想。

### 第一次打开

```text
Book
 ↓
metadata
 ↓
minimal index
 ↓
current chapter
 ↓
current viewport
 ↓
显示
```

然后：

```text
background worker
 ↓
build more index
 ↓
build layout cache
```

### 第二次打开

```text
Book
 ↓
cache metadata
 ↓
cache index
 ↓
cache layout
 ↓
instant viewport
```

这才是真正的：

```text
cold open
warm open
```

两套路径。

---

# 19. 不要一开始做“全书完整 layout”

这一点尤其重要。

假设：

```text
1,000,000 pages
```

你如果设计：

```text
open
 ↓
generate 1,000,000 pages
```

即使放后台也不理想。

应该：

```text
current chapter
    ↓
current page
    ↓
neighbor pages
    ↓
neighbor chapters
```

逐渐扩大 cache。

也就是说：

> **Lazy + Incremental + Persistent**

这三个词应该成为新架构的核心。

---

# 20. 还有一个来自近期项目的启发：大文件要避免“从头反复读取”

这不仅仅是 legado 类项目的问题。

现代大文件阅读器也在专门优化这个问题。例如 Readwide 的近期实现明确强调：超大 TXT 顺序读取时避免每次向前移动都从文件开头重新读取，并把 sequential large-TXT 读取做到“每行只 decode 一次”。([GitHub][3])

这说明一个非常重要的原则：

```text
Forward:
A → B → C → D → E

不要：

A → scan from 0
B → scan from 0
C → scan from 0
D → scan from 0
```

而应该：

```text
A
 ↓
B
 ↓
C
 ↓
D
 ↓
E
```

维护：

```rust
struct SequentialCursor {
    file_offset: u64,
    decoded_offset: u64,
}
```

---

# 21. 因此我会加入“Cursor Engine”

这是之前方案里没有强调的。

```rust
struct BookCursor {
    source_offset: u64,
    block_id: u32,
    local_offset: u32,
}
```

它负责：

```text
next()
previous()
seek()
read_block()
```

对于：

```text
TXT
```

特别高效。

例如：

```text
cursor.next_line()
```

只读取当前 block。

而不是：

```text
document.lines()
```

一次性建立整个数组。

---

# 22. Layout 也应该变成“按需生成”

例如：

```rust
trait LayoutProvider {
    fn get_line(
        &mut self,
        line_id: u64,
        width: u16
    ) -> Result<VisualLine>;
}
```

内部：

```text
get_line(832)
      │
      ▼
cache?
 ┌────┴────┐
yes       no
 │          │
 ▼          ▼
return    load block
             │
             ▼
          layout
             │
             ▼
           cache
```

于是 UI 根本不关心：

```text
这一行是不是刚刚生成的。
```

---

# 23. 这也会改变 Ratatui 的使用方式

Ratatui 仍然推荐。

但它只负责：

```text
VisualLine[]
   ↓
Cell[]
   ↓
terminal diff
```

而不是：

```text
Ratatui
 ↓
负责文档解析
 ↓
负责 layout
 ↓
负责分页
```

所以：

```text
Ratatui = renderer

Rust Reader Core = document engine
```

这两个边界必须保持非常干净。

---

# 24. 我最终推荐的技术栈

### Core

```text
Rust
```

### File access

```text
std::fs::File
BufReader
mmap / memmap2（适合的格式）
```

不建议一开始到处 mmap。

TXT：

```text
BufReader + seek
```

已经很好。

---

### Index

```text
custom binary index
```

而不是 SQLite。

为什么？

因为：

```text
Book index
```

本质是：

```text
array of offsets
```

例如：

```text
[u64; N]
```

用 SQLite 有点重。

可以：

```text
meta.bin
index.bin
```

简单高效。

SQLite 更适合：

```text
bookshelf
history
bookmarks
settings
```

而不是核心文本索引。

---

# 25. Cache 格式

我反而建议你自己设计一个：

```text
LUEC
```

例如：

```text
┌──────────────┐
│ magic        │ 4B
├──────────────┤
│ version      │ 4B
├──────────────┤
│ book hash    │ 16B
├──────────────┤
│ format       │ 1B
├──────────────┤
│ layout width │ 2B
├──────────────┤
│ metadata     │ ...
├──────────────┤
│ index        │ ...
└──────────────┘
```

这样未来：

```text
Python
Rust
其他 frontend
```

都可以复用。

---

# 26. Book identity 也必须设计好

不能简单：

```text
filename
```

应该：

```text
BookId =
    hash(
        canonical_path
        + file_size
        + modified_time
    )
```

对于远程下载书，还可以：

```text
content_hash
```

否则：

```text
book.epub
```

换了一版内容之后：

```text
旧 layout cache
```

可能被错误复用。

---

# 27. 一个特别值得借鉴的设计：Cache 可丢失

Cache 必须被视为：

```text
derived data
```

而不是：

```text
source of truth
```

所以：

```text
delete ~/.cache/lue
```

之后：

```text
书仍然能打开
```

只是：

```text
第一次变慢
```

这是非常重要的工程原则。

---

# 28. 我会把新的迁移路线改成这样

## Phase 0 — Benchmark

先建立：

```text
bench/
 ├── open_small
 ├── open_100mb
 ├── open_1gb
 ├── seek_50
 ├── seek_90
 ├── next_page
 └── resize
```

---

## Phase 1 — Rust Book Store ⭐

先不碰 UI。

实现：

```text
Book
BookSource
BookId
RandomAccessReader
Block
```

目标：

```text
10GB TXT
```

也能：

```text
open()
seek()
read_viewport()
```

而不读取整个文件。

---

## Phase 2 — Rust Index ⭐⭐⭐

实现：

```text
ChapterIndex
ParagraphIndex
SentenceIndex
WordIndex
```

但采用：

```text
lazy index
```

而不是一次性构建全部 index。

---

## Phase 3 — Persistent Cache ⭐⭐⭐

加入：

```text
meta.bin
index.bin
```

让第二次启动直接：

```text
load index
```

---

## Phase 4 — Layout Engine ⭐⭐⭐⭐⭐

这是我现在认为真正的核心。

实现：

```text
source
 ↓
grapheme
 ↓
word
 ↓
line wrap
 ↓
VisualLine
```

并 cache：

```text
width → layout
```

---

## Phase 5 — Python Adapter

Python 调：

```python
book = RustBook(path)
book.viewport(...)
book.next(...)
book.seek(...)
```

这样：

```text
Python UI
     │
     ▼
Rust Core
```

可以真实测试：

> Rust Core 是否已经解决性能问题。

---

## Phase 6 — Ratatui

最后：

```text
Python UI
     ↓
删除
     ↓
Ratatui
```

这时 UI migration 其实会非常机械。

---

# 29. Phase 7 — Parser migration

然后：

```text
TXT
 ↓
Markdown
 ↓
HTML
 ↓
EPUB
 ↓
DOCX
 ↓
PDF
```

逐个迁移。

尤其：

```text
PDF
```

我会最后做。

因为 PDF 的“阅读顺序”本身就不像 TXT/EPUB 那么简单。

---

# 30. 最终你会得到一个非常不一样的 Lue

不是：

```text
Python reader
        ↓
Rust faster reader
```

而是：

```text
                     Lue 2

                ┌─────────────┐
                │   Reader    │
                └──────┬──────┘
                       │
                ┌──────▼──────┐
                │ Reader Core │
                │    Rust     │
                └──────┬──────┘
                       │
       ┌───────────────┼────────────────┐
       │               │                │
       ▼               ▼                ▼
  Random Access     Index            Layout
       │               │                │
       │               │                │
       └───────────────┼────────────────┘
                       │
                 Persistent Cache
                       │
                ┌──────▼──────┐
                │  Viewport   │
                └──────┬──────┘
                       │
                  Ratatui TUI
                       │
                  Crossterm
                       │
                    Terminal
```

这时候你就可以做到：

```text
1 MB book       ≈ instant
100 MB book     ≈ instant
1 GB book       ≈ instant
10 GB book      ≈ instant*
```

`*` 前提是格式本身支持合理的随机访问/索引建立，而且“instant”指打开当前 viewport，而不是瞬间完成整本书的全文索引。

---

## 最值得从 legado-E / 同类高性能阅读器吸取的 5 个原则

| 原则                       | 对 Lue 的意义       |     优先级 |
| -------------------------- | ------------------- | ---------: |
| **Lazy loading**           | 不加载整本书        | ⭐⭐⭐⭐⭐ |
| **Persistent index/cache** | 第二次打开接近瞬开  | ⭐⭐⭐⭐⭐ |
| **Random access**          | 任意位置快速跳转    | ⭐⭐⭐⭐⭐ |
| **Incremental layout**     | 只排版可见/附近内容 | ⭐⭐⭐⭐⭐ |
| **Background prefetch**    | 用户感知不到加载    |   ⭐⭐⭐⭐ |

而：

| 技术          |   优先级 |
| ------------- | -------: |
| Rust          | ⭐⭐⭐⭐ |
| Ratatui       |   ⭐⭐⭐ |
| ANSI 优化     |     ⭐⭐ |
| Tokio         |       ⭐ |
| GPU rendering |       ❌ |

**所以我会把之前的方案明确调整为：`Storage → Index → Lazy Reader → Layout Cache → Reader State → TUI`。**

另外，`legado-E` 目前的公开仓库更像是一个完整 Android 阅读平台，而不是单纯的“阅读器内核”；它同时包含书源、WebView、订阅、媒体等大量功能。([GitHub][1]) 因此如果我们的目标是**准确复刻它“超大书籍秒开”的技术原因**，下一步最有价值的工作不是继续看 README，而是直接做一次 **legado-E 源码级 reverse engineering：定位 `阅读正文` → `章节加载` → `TXT/EPUB` → `缓存/索引` → `分页/排版` 的实际调用链**，然后把其中真正值得迁移到 Rust 的算法逐项映射到 Lue。

如果按这个方向继续，我建议下一步直接做一张 **“Lue Python → legado-E Android → Rust Core” 三方架构对照表**，并进一步给出 Rust 的 `BookStore / Index / LayoutCache / Viewport / ReaderState` 具体 trait 和 struct 设计。

[1]: https://github.com/Luoyacheng/legado-E?utm_source=chatgpt.com "GitHub - Luoyacheng/legado-E: 阅读Sigma是legado的继承，保持开源免费，延续开源精神。 · GitHub"
[2]: https://github.com/lukecarbis/CrossPointXT?utm_source=chatgpt.com "GitHub - lukecarbis/CrossPointXT: Firmware for the Xteink X3 and X4 e-paper display readers that can also edit txt or md files. · GitHub"
[3]: https://github.com/k1717/Readwide?utm_source=chatgpt.com "GitHub - k1717/Readwide: File reader / manager · GitHub"
