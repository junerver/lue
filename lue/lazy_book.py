"""Lazy chapter access for indexed TXT books (legado TextFile.kt style).

With a byte-range chapter index (see lue_rs.build_txt_index) the reader no
longer keeps every chapter's text in memory: chapters decode on demand from
the original file and are kept in a small LRU. The container implements the
plain sequence protocol so all existing consumers (navigation, TTS, chapter
menu, windowed layout) keep working unchanged.

Chapter decoding mirrors the title-rule path of the full parse
(content_parser._split_txt_by_rule): the chapter's first line is its title
and always kept; remaining lines are cleaned and kept when longer than 3
chars; the preface chapter (before the first title) keeps every non-empty
cleaned line.
"""

import io
from collections import OrderedDict
from typing import List

from . import _rust
from .content_parser import clean_visual_text


class LazyChapters:
    """Read-only lazy sequence of chapters backed by a byte-range index."""

    def __init__(self, path: str, encoding: str, starts: List[int],
                 ends: List[int], intro_first: bool, cache_size: int = 64):
        self._path = path
        self._encoding = encoding
        self._starts = list(starts)
        self._ends = list(ends)
        self._intro_first = bool(intro_first)
        self._cache_size = max(4, cache_size)
        self._cache: "OrderedDict[int, List[str]]" = OrderedDict()
        self._clean = (
            _rust.lue_rs.clean_visual_text
            if _rust.lue_rs is not None
            else clean_visual_text
        )
        # Optional Rust book handle: when available, chapter decode + LRU live
        # in Rust (shared ownership, fewer Python allocations). Falls back to
        # the Python path if your Rust wheel predates IndexedTxtBook.
        self._rust_book = None
        if _rust.lue_rs is not None and hasattr(_rust.lue_rs, "IndexedTxtBook"):
            try:
                self._rust_book = _rust.lue_rs.IndexedTxtBook.from_index(
                    path, encoding, self._starts, self._ends, intro_first
                )
            except Exception:
                self._rust_book = None

    # -- sequence protocol ---------------------------------------------------

    def __len__(self) -> int:
        return len(self._starts)

    def __getitem__(self, i):
        if isinstance(i, slice):
            return [self[k] for k in range(*i.indices(len(self)))]
        if i < 0:
            i += len(self)
        if not 0 <= i < len(self._starts):
            raise IndexError(i)
        if self._rust_book is not None:
            try:
                return self._rust_book.read_chapter(i)
            except Exception:
                # Index may be stale; fall back to Python decode.
                pass
        cached = self._cache.get(i)
        if cached is None:
            cached = self._decode_chapter(i)
            self._cache[i] = cached
            if len(self._cache) > self._cache_size:
                self._cache.popitem(last=False)
        return cached

    # -- decoding ------------------------------------------------------------

    def _is_preface(self, i: int) -> bool:
        return i == 0 and self._intro_first

    def _decode_chapter(self, i: int) -> List[str]:
        start, end = self._starts[i], self._ends[i]
        with open(self._path, "rb") as f:
            f.seek(start)
            data = f.read(end - start)
        content = data.decode(self._encoding)
        content = content.replace("\r\n", "\n").replace("\r", "\n")
        lines = content.split("\n")

        preface = self._is_preface(i)
        out: List[str] = []
        for j, line in enumerate(lines):
            text = self._clean(line.strip())
            if j == 0 and not preface:
                # chapter title line: always kept (mirrors _split_txt_by_rule)
                if text:
                    out.append(text)
            elif text and len(text) > 3:
                out.append(text)
        return out

    # -- repr ----------------------------------------------------------------

    def __repr__(self):
        return f"LazyChapters({len(self._starts)} chapters, encoding={self._encoding!r})"
