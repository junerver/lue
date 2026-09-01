"""Equivalence tests: lazy indexed chapter decoding must produce exactly the
same chapters as the whole-book parse for the eligible (Chinese, one-line-
per-paragraph, titled chapters) profile — in UTF-8 and GB18030.
"""

import io

import pytest
from rich.console import Console

from lue import _rust, content_parser as cp
from lue.lazy_book import LazyChapters


def make_console():
    return Console(file=io.StringIO(), width=80)


def build_book_text(chapters=12):
    # Chapters must exceed the 1000-char validity threshold of the legado
    # rule scorer (as in real novels) or no TOC rule is selected at all —
    # by design, on BOTH the full-parse and the lazy path.
    digits = "零一二三四五六七八九十"
    parts = ["测试书名\n", "作者:某人\n\n"]
    for i in range(1, chapters + 1):
        parts.append(f"第{digits[i % 10]}章 试验标题{i}\n")
        parts.append("开头的一句话,用来填充正文内容!\n")
        parts.append("第二段,同样比较短。\n")
        parts.append("\n")
        parts.append("空行之后的段落,应该仍然归入本章。\n")
        for j in range(70):
            parts.append(f"第{j}行正文,内容足够长以满足章节有效性判定。\n")
        parts.append("\n")
    return "".join(parts)


def full_parse(path):
    return cp._extract_content_txt(path, make_console())


def lazy_parse(path):
    encoding, intro_first, starts, ends = _rust.lue_rs.build_txt_index(path)
    assert encoding, "book should be lazily indexable"
    lazy = LazyChapters(path, encoding, starts, ends, intro_first)
    return list(lazy), (encoding, intro_first, starts, ends)


@pytest.mark.skipif(_rust.lue_rs is None, reason="lue_rs extension not built")
def test_index_equivalence_utf8(tmp_path):
    path = tmp_path / "book_utf8.txt"
    path.write_bytes(build_book_text().encode("utf-8"))
    full = full_parse(str(path))
    lazy, (encoding, intro_first, starts, ends) = lazy_parse(str(path))
    assert encoding == "utf-8"
    assert intro_first is True
    assert len(lazy) == len(full), (len(lazy), len(full))
    for i, (a, b) in enumerate(zip(lazy, full)):
        assert a == b, f"chapter {i} differs"


@pytest.mark.skipif(_rust.lue_rs is None, reason="lue_rs extension not built")
def test_index_equivalence_gb18030(tmp_path):
    path = tmp_path / "book_gbk.txt"
    path.write_bytes(build_book_text().encode("gb18030"))
    full = full_parse(str(path))
    lazy, (encoding, _intro, _s, _e) = lazy_parse(str(path))
    assert encoding == "gb18030"
    assert len(lazy) == len(full)
    for i, (a, b) in enumerate(zip(lazy, full)):
        assert a == b, f"chapter {i} differs"


@pytest.mark.skipif(_rust.lue_rs is None, reason="lue_rs extension not built")
def test_index_equivalence_utf8_bom(tmp_path):
    path = tmp_path / "book_bom.txt"
    path.write_bytes(bytes([0xEF, 0xBB, 0xBF]) + build_book_text().encode("utf-8"))
    full = full_parse(str(path))
    lazy, (encoding, _intro, _s, _e) = lazy_parse(str(path))
    assert encoding == "utf-8-sig"
    assert lazy == full
    assert lazy[0][0] == full[0][0]


@pytest.mark.skipif(_rust.lue_rs is None, reason="lue_rs extension not built")
def test_index_preface_content(tmp_path):
    path = tmp_path / "book_preface.txt"
    path.write_bytes(build_book_text().encode("utf-8"))
    lazy, (encoding, intro_first, starts, ends) = lazy_parse(str(path))
    # preface keeps every non-empty cleaned line (no >3 length filter)
    assert "作者:某人" in lazy[0][0] or "测试书名" in lazy[0][0]
    assert intro_first is True


@pytest.mark.skipif(_rust.lue_rs is None, reason="lue_rs extension not built")
def test_lazy_slice_and_len(tmp_path):
    path = tmp_path / "book_slice.txt"
    path.write_bytes(build_book_text(chapters=20).encode("utf-8"))
    lazy, _meta = lazy_parse(str(path))
    assert len(lazy) == 21  # preface + 20 chapters
    window = lazy[9:12]  # slice triggers on-demand decode
    assert len(window) == 3
    assert window[1] == lazy[10]
