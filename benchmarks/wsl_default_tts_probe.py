"""Verify the tts-default-none startup: no TTS instance, no ffmpeg check,
no panic, and the Lue() load time on the given book."""

import io
import subprocess
import sys
import time

from rich.console import Console

console = Console(file=io.StringIO(), width=80)

# --- simulate cli()'s tts_enabled decision (mirrors __main__.py) ---
from lue.tts_manager import TTSManager

tts_manager = TTSManager()
available_tts = tts_manager.get_available_tts_names()
args_tts = None  # user ran plain `lue book.txt`, no -t flag
tts_enabled = bool(available_tts) and args_tts and args_tts != "none"
print(f"available_tts: {available_tts}")
print(f"tts_enabled (default, no -t): {tts_enabled}")

ffmpeg_checked = False
if tts_enabled:
    for tool in ["ffprobe", "ffplay", "ffmpeg"]:
        subprocess.run([tool, "-version"], check=True, text=True,
                       stderr=subprocess.DEVNULL, stdout=subprocess.DEVNULL)
        ffmpeg_checked = True
print(f"ffmpeg checked: {ffmpeg_checked} (should be False)")

tts_instance = None
if tts_enabled:
    tts_instance = tts_manager.create_model(args_tts, console)
print(f"tts instance created: {tts_instance is not None} (should be False)")

# --- the real load on the book ---
from lue.reader import Lue

book = sys.argv[1]
t0 = time.perf_counter()
reader = Lue(book, tts_model=None, overlap=None)
load_ms = (time.perf_counter() - t0) * 1000
print(f"Lue() ctor: {load_ms:.0f} ms | chapters: {len(reader.chapters)} | "
      f"total_sentences: {reader.total_sentences}")

# --- first frame render ---
from lue import ui
import asyncio

t0 = time.perf_counter()
asyncio.run(ui.display_ui(reader))
print(f"first display_ui: {(time.perf_counter()-t0)*1000:.0f} ms")