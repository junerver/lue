"""Probe where the lue startup time actually goes (WSL).

Simulates the cli() startup sequence minus the interactive loop:
ffmpeg checks, keyboard loading, Lue() construction (which includes
_load_content), and the first display_ui call. Run inside the pipx venv.
"""

import io
import os
import subprocess
import sys
import time

from rich.console import Console

from lue import input_handler, tts_manager

console = Console(file=io.StringIO(), width=80)

# --- ffmpeg checks ---
for tool in ["ffprobe", "ffplay", "ffmpeg"]:
    t0 = time.perf_counter()
    try:
        subprocess.run(
            [tool, "-version"],
            check=True,
            text=True,
            stderr=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
        )
        print(f"{tool:8} check: {(time.perf_counter()-t0)*1000:6.0f} ms")
    except Exception as e:
        print(f"{tool:8} check FAIL: {e}")

# --- keyboard shortcuts load ---
t0 = time.perf_counter()
input_handler.load_keyboard_shortcuts("default")
print(f"keyboard load: {(time.perf_counter()-t0)*1000:6.0f} ms")

# --- Lue() construction (includes _load_content) ---
from lue.reader import Lue

book = sys.argv[1]
class StubTTSPause:
    pass

t0 = time.perf_counter()
reader = Lue(book, tts_model=None, overlap=None)
print(f"Lue() ctor (incl _load_content): {(time.perf_counter()-t0)*1000:6.0f} ms")
print(f"  chapters: {len(reader.chapters)} total_sentences: {reader.total_sentences}")

# --- first display_ui ---
import asyncio

from lue import ui

t0 = time.perf_counter()
ui.display_ui(reader)
print(f"first display_ui: {(time.perf_counter()-t0)*1000:6.0f} ms")