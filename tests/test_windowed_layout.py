"""Behavior tests for the legado-style windowed layout.

The reader's document_lines / position maps only cover the current chapter
window (current +/- neighbors). These tests pin the window semantics:
global chapter numbers in keys, window-relative line numbers, window
sliding via _ensure_window, and chapter-based progress.
"""

import io

import pytest
from rich.console import Console

from lue import ui
from lue.reader import Lue


def make_console():
    return Console(file=io.StringIO(), width=80)


class StubReader:
    """Minimal reader state for window layout functions."""

    def __init__(self, chapters):
        self.chapters = chapters
        self.file_path = "stub.txt"
        self.console = make_console()
        self._layout_cache = None
        self._initial_load_complete = False
        self.ui_chapter_idx = 0
        self.ui_paragraph_idx = 0
        self.ui_sentence_idx = 0
        self.chapter_idx = 0
        self.paragraph_idx = 0
        self.sentence_idx = 0
        self.document_lines = []
        self.line_to_position = {}
        self.position_to_line = {}
        self.paragraph_line_ranges = {}
        self._sorted_line_positions = None
        self._window_base = None
        self._window_end = None
        self.scroll_offset = 0

    # reader helpers used by the window code live on Lue; borrow them
    _ensure_window = Lue._ensure_window
    _chapter_progress_fraction = Lue._chapter_progress_fraction
    _slide_window = Lue._slide_window
    _position_at_line = Lue._position_at_line


def build_chapters(n):
    return [
        [f"第{i}章标题", f"第{i}章第一段内容。第二句!\n", f"第{i}章第二段。"]
        for i in range(n)
    ]


@pytest.fixture
def book():
    return build_chapters(20)


def test_window_covers_center_and_neighbors(book):
    r = StubReader(book)
    ui.build_window_layout(r, 10)
    assert r._window_base == 9
    assert r._window_end == 13  # 9..12 inclusive
    # keys keep global chapter numbers
    assert (10, 0, 0) in r.position_to_line
    assert (9, 0, 0) in r.position_to_line
    assert (12, 0, 0) in r.position_to_line
    # chapters outside the window are absent
    assert (8, 0, 0) not in r.position_to_line
    assert (13, 0, 0) not in r.position_to_line
    assert (0, 0) in r.paragraph_line_ranges or (9, 0) in r.paragraph_line_ranges


def test_window_at_book_start(book):
    r = StubReader(book)
    ui.build_window_layout(r, 0)
    assert r._window_base == 0
    assert r._window_end == 3
    assert (0, 0, 0) in r.position_to_line


def test_window_at_book_end(book):
    r = StubReader(book)
    ui.build_window_layout(r, 19)
    assert r._window_base == 18
    assert r._window_end == 20
    assert (19, 0, 0) in r.position_to_line


def test_ensure_window_rebuilds_when_outside(book):
    r = StubReader(book)
    ui.build_window_layout(r, 0)
    lines_before = r.document_lines
    # inside window: no rebuild
    r._ensure_window(1)
    assert r.document_lines is lines_before
    # outside: rebuild centered on the new chapter
    r._ensure_window(10)
    assert r._window_base == 9
    assert (10, 1) in r.paragraph_line_ranges


def test_line_numbers_are_window_relative(book):
    r = StubReader(book)
    ui.build_window_layout(r, 10)
    # first line of the window belongs to chapter 9 (base) and is line 0
    first_pos = min(r.position_to_line.items(), key=lambda kv: kv[1])
    assert first_pos[0][0] == 9
    assert first_pos[1] == 0
    assert r.line_to_position[0][0] == 9


def test_chapter_progress(book):
    r = StubReader(book)
    r.chapter_idx = 10
    expected = 10 / 20 * 100
    assert Lue._calculate_progress_percentage(r) == pytest.approx(expected)
    r.chapter_idx = 0
    assert Lue._calculate_progress_percentage(r) == 0.0
    r.chapter_idx = 19
    assert Lue._calculate_progress_percentage(r) == pytest.approx(95.0)


def test_update_document_layout_reuses_cached_window(book):
    r = StubReader(book)
    ui.build_window_layout(r, 0)
    lines_first = r.document_lines
    ui.update_document_layout(r)  # same center (ui_chapter_idx=0), same width
    assert r.document_lines is lines_first


def test_initialize_progress_recomputes_scroll_in_window(tmp_path, monkeypatch):
    """Regression: saved scroll_offset is a whole-book line number; after the
    windowed refactor it must NOT be applied directly or the viewport is empty."""
    import json

    from lue import progress_manager

    chapters = build_chapters(20)

    class ProgressStub(StubReader):
        def __init__(self, chapters, progress_file):
            super().__init__(chapters)
            self.file_path = "book.txt"
            self.book_title = "t"
            self.progress_file = str(progress_file)
            self.tts_model = None

    progress_file = tmp_path / "t.progress.json"
    # Simulate a pre-windowing progress file: huge scroll offset, position in
    # chapter 10, manual anchor matching.
    progress_file.write_text(json.dumps({
        "c": 10, "p": 0, "s": 0,
        "scroll_offset": 999999,
        "tts_enabled": False,
        "auto_scroll_enabled": False,
        "speed_reading_enabled": False,
        "manual_scroll_anchor": [10, 0, 0],
        "playback_speed": 1.0,
    }), encoding="utf-8")

    r = ProgressStub(chapters, progress_file)
    ui.build_window_layout(r, 0)  # initial window at book start
    Lue._initialize_progress(r)

    # Window must now cover chapter 10 and the scroll offset must be inside it.
    assert r._window_base <= 10 < r._window_end
    assert 0 <= r.scroll_offset < len(r.document_lines)
    # The manual anchor maps to a real line and is visible-ish (not empty view).
    assert (10, 0, 0) in r.position_to_line


@pytest.fixture
def sliding_reader(tmp_path, monkeypatch):
    """A reader-like stub with a real book and initial window; lets us call
    the slide logic without raw TTY/audio plumbing."""
    import json

    from lue import progress_manager

    chapters = build_chapters(40)
    book_path = tmp_path / "slide_book.txt"
    book_path.write_text(
        "\n".join(f"第{i}章标题\n正文{i}内容足够长以参与排版。" for i in range(40)),
        encoding="utf-8",
    )

    class SlideStub(StubReader):
        def __init__(self):
            super().__init__(chapters)
            self.file_path = str(book_path)
            self.book_title = "slide"
            self.progress_file = str(tmp_path / "slide.progress.json")
            self.tts_model = None
            self._initial_load_complete = True
            self.scroll_offset = 0.0
            self.target_scroll_offset = 0.0
            self.smooth_scroll_task = None
            self.auto_scroll_enabled = False

    r = SlideStub()
    ui.build_window_layout(r, 0)
    return r


def test_slide_window_forward_keeps_viewport_anchor(sliding_reader):
    r = sliding_reader
    base_before = r._window_base
    # Put the viewport near the bottom of the first window, then slide.
    max_scroll = max(0, len(r.document_lines) - 20)
    r.scroll_offset = r.target_scroll_offset = float(max_scroll)
    new_offset = r._slide_window(1)
    assert new_offset >= 0
    # Window must have advanced.
    assert r._window_base > base_before
    # The anchor sentence (top of old viewport) must exist in the new window.
    anchor = r._position_at_line(int(r.scroll_offset))
    assert anchor is not None
    assert anchor in r.position_to_line


def test_slide_window_backward_keeps_viewport_anchor(sliding_reader):
    r = sliding_reader
    ui.build_window_layout(r, 10)
    base_before = r._window_base
    r.scroll_offset = r.target_scroll_offset = 5.0
    new_offset = r._slide_window(-1)
    assert new_offset >= 0
    assert r._window_base < base_before


def test_manual_anchor_restore_centers_anchor_chapter(tmp_path, monkeypatch):
    """A manual scroll anchor in a later chapter must center the restore
    window on that chapter, not on the saved playback chapter."""
    import json

    from lue import progress_manager

    chapters = build_chapters(30)
    progress_file = tmp_path / "anchor.progress.json"
    progress_file.write_text(json.dumps({
        "c": 2, "p": 0, "s": 0,
        "scroll_offset": 999999,
        "tts_enabled": False,
        "auto_scroll_enabled": False,
        "speed_reading_enabled": False,
        "manual_scroll_anchor": [20, 0, 0],
        "playback_speed": 1.0,
    }), encoding="utf-8")

    class AnchorStub(StubReader):
        def __init__(self):
            super().__init__(chapters)
            self.progress_file = str(progress_file)
            self.tts_model = None

    r = AnchorStub()
    ui.build_window_layout(r, 0)
    Lue._initialize_progress(r)
    # Window must cover chapter 20 (the anchor), not chapter 2 (playback).
    assert r._window_base <= 20 < r._window_end
    # Manual anchor must map into the window and be visible-ish.
    assert (20, 0, 0) in r.position_to_line
