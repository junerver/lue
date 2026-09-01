"""Disk caches for parsed books and document layouts.

Opening a large book currently pays parse (~1.1s), sentence counting
(~0.2s) and full layout rebuild (~1.6s) synchronously before the UI shows
anything. Re-opening the same book — the common case — re-pays all of it.

Two caches, both content-addressed so a changed file never serves stale
data:

- ``ParsedBookCache``: the parsed chapters + total_sentences for a file,
  keyed by (mtime, size). Every reader session needs exactly this.
- ``LayoutCache``: the full layout for a file at a given terminal width,
  keyed by (mtime, size, width). Rebuilds on width change (v key / resize)
  are ~1.6s of blocking work; the in-memory layout cache only covers width
  changes within a session, not re-opens.

Pickle is safe here: the payloads are plain lists of str (chapters) and
plain dicts of Text instances (layout). Values are validated on load and a
corrupt/mismatched cache is treated as a miss (atomic write via tmp +
os.replace).
"""

import hashlib
import os
import pickle
import tempfile

from . import config


def _sha1_hex(text: str) -> str:
    return hashlib.sha1(text.encode("utf-8", errors="replace")).hexdigest()


class _BaseCache:
    _dir = ""

    def __init__(self, book_path: str, book_key: str) -> None:
        self._file = os.path.join(
            config.PROGRESS_FILE_DIR, self._dir, f"{_sha1_hex(book_path)}-{book_key}.cache"
        )

    def _load(self):
        try:
            with open(self._file, "rb") as f:
                return pickle.load(f)
        except Exception:
            return None

    def _store(self, payload) -> None:
        try:
            dirname = os.path.dirname(self._file)
            os.makedirs(dirname, exist_ok=True)
            fd, tmp = tempfile.mkstemp(dir=dirname, suffix=".tmp")
            with os.fdopen(fd, "wb") as f:
                pickle.dump(payload, f, protocol=pickle.HIGHEST_PROTOCOL)
            os.replace(tmp, self._file)
        except Exception:
            try:
                if os.path.exists(tmp):
                    os.unlink(tmp)
            except Exception:
                pass


class ParsedBookCache(_BaseCache):
    """Caches (chapters, total_sentences) for one book file.

    Keyed by (mtime, size): touching the file (or replacing it) invalidates
    the entry; same mtime+size on a different machine is an accepted edge
    case. Values are the exact objects the reader needs.
    """

    _dir = "parsed"

    @classmethod
    def key_for(cls, stat) -> str:
        return f"m{int(stat.st_mtime_ns)}-s{stat.st_size}"

    def load(self):
        data = self._load()
        if not isinstance(data, dict):
            return None
        chapters = data.get("chapters")
        if not isinstance(chapters, list):
            return None
        # Validate shape: list of list of str, non-empty.
        for chapter in chapters:
            if not isinstance(chapter, list) or not all(isinstance(p, str) for p in chapter):
                return None
        if not chapters:
            return None
        return chapters

    def store(self, chapters) -> None:
        self._store({"chapters": chapters})


class LayoutCache(_BaseCache):
    """Caches a built layout for one book at one width.

    Stores plain strings for the document lines (Text objects reconstruct
    from plain text at load — this keeps the cache file ~30x smaller and
    deserialization ~70x faster than pickling Text instances). Keyed by
    (mtime, size, width) so a terminal resize or book edit rebuilds.
    """

    _dir = "layout"

    @classmethod
    def key_for(cls, stat, width: int) -> str:
        return f"m{int(stat.st_mtime_ns)}-s{stat.st_size}-w{width}"

    def load(self):
        data = self._load()
        if not isinstance(data, dict):
            return None
        try:
            lines = data["lines"]  # list of str
            line_to_position = data["line_to_position"]
            position_to_line = data["position_to_line"]
            paragraph_ranges = data["paragraph_ranges"]
            sorted_index = data["sorted_index"]
        except KeyError:
            return None
        if not isinstance(lines, list) or not all(isinstance(l, str) for l in lines):
            return None
        if not isinstance(line_to_position, dict) or not isinstance(position_to_line, dict):
            return None
        if not isinstance(paragraph_ranges, dict) or not isinstance(sorted_index, (list, tuple)):
            return None
        return {
            "lines": lines,
            "line_to_position": line_to_position,
            "position_to_line": position_to_line,
            "paragraph_ranges": paragraph_ranges,
            "sorted_index": sorted_index,
            "total_sentences": data.get("total_sentences", 0) or 0,
        }

    def store(self, layout: dict) -> None:
        payload = dict(layout)
        # lines are plain strings in the rebuilt layout (Text objects only on
        # the in-memory cache path); normalize to str either way.
        payload["lines"] = [
            line.plain if hasattr(line, "plain") else line for line in layout["lines"]
        ]
        self._store(payload)

