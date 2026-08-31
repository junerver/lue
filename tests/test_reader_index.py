"""Behavior-parity tests for the topmost-visible-sentence index.

The bisect implementation in reader._topmost_visible_sentence must return
exactly what the original O(book) dict double-scan returned: the earliest
line in the viewport (first-inserted position on that line), or when the
viewport holds no sentence, the first-inserted position on the latest line
before the viewport.
"""

import random

from lue.reader import _topmost_visible_sentence


def original_scan(position_to_line, top_visible_line, bottom_visible_line):
    """Verbatim copy of the pre-index implementation (the oracle)."""
    topmost_sentence_pos = None
    earliest_line = float("inf")
    for pos, line_num in position_to_line.items():
        if top_visible_line <= line_num < bottom_visible_line:
            if line_num < earliest_line:
                earliest_line = line_num
                topmost_sentence_pos = pos
    if topmost_sentence_pos:
        return topmost_sentence_pos
    last_pos_before_view = None
    latest_line = -1
    for pos, line_num in position_to_line.items():
        if line_num < top_visible_line and line_num > latest_line:
            latest_line = line_num
            last_pos_before_view = pos
    return last_pos_before_view


def make_index(position_to_line):
    # Same construction as ui.update_document_layout: stable sort by line,
    # stored as parallel (positions, lines) arrays.
    items = sorted(position_to_line.items(), key=lambda kv: kv[1])
    return (
        [pos for pos, _ in items],
        [ln for _, ln in items],
    )


def build_book_map():
    """Insertion-ordered map mimicking update_document_layout."""
    position_to_line = {}
    line = 0
    for c in range(5):
        if c > 0:
            line += 1
        for p in range(20):
            start = line
            for s in range(4):
                position_to_line[(c, p, s)] = start + s // 2
            line += 3
    return position_to_line


def test_matches_original_scan_random_scroll():
    position_to_line = build_book_map()
    index = make_index(position_to_line)
    total_lines = max(position_to_line.values()) + 10
    rng = random.Random(7)
    for _ in range(500):
        top = rng.randint(0, total_lines)
        height = rng.choice([1, 10, 40])
        expected = original_scan(position_to_line, top, top + height)
        got = _topmost_visible_sentence(position_to_line, index, top, top + height)
        assert got == expected, (top, height, got, expected)


def test_topmost_zero_position():
    # Scroll to the very top: topmost is (0, 0, 0). (Non-empty tuples are
    # always truthy in Python, so the original `if topmost_sentence_pos:`
    # returns it directly — no quirk, just the normal path.)
    position_to_line = build_book_map()
    index = make_index(position_to_line)
    assert position_to_line[(0, 0, 0)] == 0
    expected = original_scan(position_to_line, 0, 40)
    assert expected == (0, 0, 0)
    got = _topmost_visible_sentence(position_to_line, index, 0, 40)
    assert got == expected


def test_empty_book():
    assert _topmost_visible_sentence({}, None, 0, 40) is None
    assert _topmost_visible_sentence({}, [], 5, 45) is None


def test_viewport_before_and_after_content():
    position_to_line = build_book_map()
    index = make_index(position_to_line)
    last_line = max(position_to_line.values())
    # Viewport entirely before the first line -> None (nothing before it).
    assert original_scan(position_to_line, -50, -10) is None
    assert _topmost_visible_sentence(position_to_line, index, -50, -10) is None
    # Viewport entirely after the last line -> last sentence before it.
    expected = original_scan(position_to_line, last_line + 5, last_line + 45)
    got = _topmost_visible_sentence(position_to_line, index, last_line + 5, last_line + 45)
    assert got == expected
    assert got == max(position_to_line.items(), key=lambda kv: kv[1])[0]


def test_duplicate_lines_first_inserted_wins():
    # Several positions share the topmost line; the earliest-inserted one
    # (lowest (c, p, s) as built by the layout loop) must be returned.
    position_to_line = {}
    for c in range(3):
        for p in range(3):
            for s in range(3):
                position_to_line[(c, p, s)] = 5  # all on one line
    index = make_index(position_to_line)
    # (0, 0, 0) is the topmost position and is returned as-is.
    assert original_scan(position_to_line, 5, 10) == (0, 0, 0)
    assert _topmost_visible_sentence(position_to_line, index, 5, 10) == (0, 0, 0)
    # Without the (0, 0, 0) entry the first-inserted position must win.
    position_to_line2 = {(0, 0, 1): 5, (9, 9, 9): 5}
    index2 = make_index(position_to_line2)
    expected = original_scan(position_to_line2, 5, 10)
    assert expected == (0, 0, 1)
    assert _topmost_visible_sentence(position_to_line2, index2, 5, 10) == (0, 0, 1)
