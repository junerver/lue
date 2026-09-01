"""Disk caches for parsed books and document layouts.

Opening a large book currently pays parse (~1.1s), sentence counting
(~0.2s) and full layout rebuild (~1.6s) synchronously before the UI shows
anything. Re-opening the same book — the common case — re-pays all of it.

ParsedBookCache: the parsed chapters for a file, keyed by (mtime, size).
Every reader session needs exactly this; layout is windowed (legado-style,
current chapter +/- neighbors) and rebuilds in ~10ms, so it is not cached
on disk.

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
