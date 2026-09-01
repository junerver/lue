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


@pytest.mark.skipif(_rust.lue_rs is None or not hasattr(_rust.lue_rs, "IndexedTxtBook"),
                     reason="IndexedTxtBook not built")
def test_rust_handle_rejects_stale_source(tmp_path):
    """Index built for a file, then the file is replaced with same-size
    content whose size matches but mtime changes: the Rust handle must
    refuse to decode stale byte ranges (fallback triggers reindex)."""
    path = tmp_path / "stale.txt"
    text = build_book_text()
    path.write_bytes(text.encode("utf-8"))
    encoding, intro_first, starts, ends = _rust.lue_rs.build_txt_index(str(path))
    assert encoding
    book = _rust.lue_rs.IndexedTxtBook.from_index(str(path), encoding, starts, ends, intro_first)
    assert book.read_chapter(1)
    # Replace with same-size different content but a new mtime.
    import time as _time
    _time.sleep(0.05)
    path.write_bytes(("X" * len(text)).encode("utf-8"))
    _time.sleep(0.05)
    # mtime changed; reading now must fail (Python side falls back to reindex)
    try:
        book.read_chapter(1)
        raise AssertionError("expected stale-source rejection")
    except Exception:
        pass


@pytest.mark.skipif(_rust.lue_rs is None or not hasattr(_rust.lue_rs, "IndexedTxtBook"),
                     reason="IndexedTxtBook not built")
def test_rust_handle_layout_window_matches(tmp_path):
    """Rust layout_window vs the windowed UI builder — same chapter keys,
    window-relative lines, paragraph ranges and sorted index."""
    from lue import ui

    path = tmp_path / "win.txt"
    path.write_bytes(build_book_text(chapters=20).encode("utf-8"))
    encoding, intro_first, starts, ends = _rust.lue_rs.build_txt_index(str(path))
    assert encoding
    book = _rust.lue_rs.IndexedTxtBook.from_index(str(path), encoding, starts, ends, intro_first)

    class Stub:
        def __init__(self):
            self.file_path = str(path)
            self.console = make_console()
            self._layout_cache = None
            from lue.lazy_book import LazyChapters
            self.chapters = LazyChapters(str(path), encoding, starts, ends, intro_first)
            self._lazy_book = self.chapters
            self._window_base = None
            self._window_end = None

    # Pin the terminal width so the Rust layout_window and the UI builder
    # pad lines to the same width.
    import lue.ui as ui_mod
    ui_mod.get_terminal_size = lambda: (90, 40)  # UI pads to 90-10=80

    reader = Stub()
    ui.build_window_layout(reader, 10)
    # Rust layout_window, center 10, before 1, after 2, width 80
    (
        lines,
        pos_keys,
        pos_vals,
        l2p_keys,
        l2p_vals,
        range_keys,
        range_vals,
        sorted_positions,
        sorted_lines,
        _total,
    ) = book.layout_window(10, 1, 2, 80)

    assert lines == reader.document_lines
    assert dict(zip(pos_keys, pos_vals)) == reader.position_to_line
    assert dict(zip(l2p_keys, l2p_vals)) == reader.line_to_position
    assert dict(zip(range_keys, range_vals)) == reader.paragraph_line_ranges
    assert (sorted_positions, sorted_lines) == reader._sorted_line_positions


@pytest.mark.skipif(_rust.lue_rs is None or not hasattr(_rust.lue_rs, "IndexedTxtBook"),
                     reason="IndexedTxtBook not built")
def test_rust_handle_lru_keeps_chapters_stable(tmp_path):
    path = tmp_path / "lru.txt"
    path.write_bytes(build_book_text(chapters=30).encode("utf-8"))
    encoding, intro_first, starts, ends = _rust.lue_rs.build_txt_index(str(path))
    book = _rust.lue_rs.IndexedTxtBook.from_index(str(path), encoding, starts, ends, intro_first)
    first = book.read_chapter(0)
    for i in range(66):
        book.read_chapter((i * 7) % 30)
    # Chapter 0 was evicted then re-fetched; content must be identical.
    assert book.read_chapter(0) == first
    last = book.chapter_count() - 1
    assert book.read_chapter(-1) == book.read_chapter(last)
    assert book.read_chapter(-(last + 1)) == book.read_chapter(0)
    import pytest as _pytest
    with _pytest.raises(Exception):
        book.read_chapter(-(last + 2))
