"""Optional Rust accelerator for lue's text pipeline.

The compiled extension (crate ``rust/``, module name ``lue_rs``) provides
drop-in fast implementations of the hot paths in ``content_parser``:
sentence splitting, visual text cleaning and TXT chapter-rule scanning.

Every caller must treat the extension as an optional speedup: when the
wheel is not installed, ``lue_rs`` is ``None`` and callers fall back to
the pure-Python implementations retained in ``content_parser``.

Set ``LUE_NO_RUST=1`` to disable the extension at runtime (A/B testing
against the pure-Python fallback without uninstalling the wheel).
"""

import os

lue_rs = None
if not os.environ.get("LUE_NO_RUST"):
    try:  # pragma: no cover - trivial import probe
        import lue_rs
    except ImportError:  # pragma: no cover
        lue_rs = None

# pyo3 raises PanicException (a BaseException) from Rust panics; the
# per-call `except Exception` fallbacks in content_parser cannot catch it.
# Guarding with BaseException at the 9 Rust call sites is the robust way
# (a Rust panic should disable the extension, not crash the reader).
_PANIC_TYPES = BaseException

__all__ = ["lue_rs"]