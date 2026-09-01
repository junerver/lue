"""Tests for the parsed-book and layout disk caches (lue/cache.py)."""

import os
import time

import pytest
from rich.console import Console
from rich.text import Text

from lue import cache as cache_mod
from lue import content_parser as cp


def make_console():
    return Console(file=None, width=80)


@pytest.fixture
def book_path(tmp_path):
    digits = "零一二三四五六七八九十"
    parts = ["测试书名\n\n"]
    for i in range(1, 6):
        parts.append(f"第{digits[i % 10]}章 试验{i}\n\n")
        for _ in range(8):
            parts.append("他说道一句话,场面顿时安静下来。有人低声议论!\n")
        parts.append("\n")
    path = tmp_path / "book.txt"
    path.write_text("".join(parts), encoding="utf-8")
    return str(path)


def build_layout(chapters, width):
    """Minimal layout payload matching ui.update_document_layout's shape."""
    position_to_line = {}
    line = 0
    for c, chapter in enumerate(chapters):
        if c > 0:
            line += 1
        for p, paragraph in enumerate(chapter):
            position_to_line[(c, p, 0)] = line
            line += 2
    sorted_items = sorted(position_to_line.items(), key=lambda kv: kv[1])
    return {
        "lines": [Text("x", justify="left", no_wrap=False)],
        "line_to_position": {0: (0, 0, 0)},
        "position_to_line": position_to_line,
        "paragraph_ranges": {(0, 0): (0, 0)},
        "sorted_index": (
            [pos for pos, _ in sorted_items],
            [ln for _, ln in sorted_items],
        ),
    }


def test_parsed_cache_roundtrip(book_path):
    chapters = cp._extract_content_txt(book_path, make_console())
    stat = os.stat(book_path)
    key = cache_mod.ParsedBookCache.key_for(stat)
    c = cache_mod.ParsedBookCache(book_path, key)
    c.store(chapters)
    loaded = cache_mod.ParsedBookCache(book_path, key).load()
    assert loaded is not None
    assert loaded == chapters


def test_parsed_cache_invalidated_on_change(book_path):
    chapters = cp._extract_content_txt(book_path, make_console())
    stat = os.stat(book_path)
    key = cache_mod.ParsedBookCache.key_for(stat)
    cache_mod.ParsedBookCache(book_path, key).store(chapters)
    # Rewrite the file (different size) -> key changes -> cache miss.
    with open(book_path, "a", encoding="utf-8") as f:
        f.write("追加的内容让文件变长。\n")
    new_key = cache_mod.ParsedBookCache.key_for(os.stat(book_path))
    assert new_key != key
    assert cache_mod.ParsedBookCache(book_path, new_key).load() is None


def test_parsed_cache_corrupt_returns_none(book_path, tmp_path):
    stat = os.stat(book_path)
    key = cache_mod.ParsedBookCache.key_for(stat)
    c = cache_mod.ParsedBookCache(book_path, key)
    os.makedirs(os.path.dirname(c._file), exist_ok=True)
    with open(c._file, "wb") as f:
        f.write(b"not a pickle")
    assert c.load() is None


def test_layout_cache_roundtrip(book_path):
    chapters = cp._extract_content_txt(book_path, make_console())
    layout = build_layout(chapters, 100)
    stat = os.stat(book_path)
    key = cache_mod.LayoutCache.key_for(stat, 100)
    cache_mod.LayoutCache(book_path, key).store(layout)
    loaded = cache_mod.LayoutCache(book_path, key).load()
    assert loaded is not None
    # lines are stored as plain strings now (Text reconstructs on load)
    assert loaded["lines"] == ["x"]
    assert loaded["position_to_line"] == layout["position_to_line"]
    # Different width -> different key -> miss.
    assert cache_mod.LayoutCache(book_path, cache_mod.LayoutCache.key_for(stat, 80)).load() is None


def test_layout_cache_corrupt_returns_none(book_path, tmp_path):
    stat = os.stat(book_path)
    key = cache_mod.LayoutCache.key_for(stat, 100)
    c = cache_mod.LayoutCache(book_path, key)
    os.makedirs(os.path.dirname(c._file), exist_ok=True)
    with open(c._file, "wb") as f:
        f.write(b"junk")
    assert c.load() is None
