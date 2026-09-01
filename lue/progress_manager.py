"""Reading progress management for the Lue eBook reader."""

import os
import json
import re
import glob
import shutil
from . import config
from . import content_parser

# Legacy records were written to this fixed name when a CJK book title was
# sanitized to an empty string (see get_progress_file_path history).
LEGACY_PROGRESS_FILE = ".progress.json"


def get_progress_file_path(book_title):
    """
    Generate the file path for storing reading progress.

    CJK is common in book titles, so the safe name must keep Unicode letters
    and digits (not just ASCII) — otherwise a Chinese title like 魅力 would
    sanitize to an empty string and the record would be written to a
    dotfile (.progress.json) that glob() cannot see.

    Args:
        book_title: Title of the book

    Returns:
        str: Full path to the progress file
    """
    safe_title = re.sub(r'[^0-9A-Za-z\u4e00-\u9fff_]+', '', book_title or '')
    if not safe_title:
        # Absolute fallback: never write an invisible dotfile.
        safe_title = 'book'
    return os.path.join(config.PROGRESS_FILE_DIR, f"{safe_title}.progress.json")

def maybe_migrate_legacy_progress(progress_file, original_file_path):
    """Upgrade a pre-windowing CJK record to the new per-title filename.

    Old versions sanitized CJK titles to an empty string, so the record for
    e.g. 魅力.txt lived at `.progress.json`. When the per-title file is
    missing and the legacy record belongs to this exact book, move it so the
    reading position survives the naming fix.
    """
    if progress_file == os.path.join(config.PROGRESS_FILE_DIR, LEGACY_PROGRESS_FILE):
        return False
    legacy_path = os.path.join(config.PROGRESS_FILE_DIR, LEGACY_PROGRESS_FILE)
    if not os.path.exists(legacy_path) or os.path.exists(progress_file):
        return False
    try:
        with open(legacy_path, 'r', encoding='utf-8') as f:
            data = json.load(f)
        if data.get("original_file_path") != original_file_path:
            return False
        shutil.move(legacy_path, progress_file)
        return True
    except (json.JSONDecodeError, OSError):
        return False


def load_progress(progress_file):
    """
    Load basic reading progress from file.
    
    Args:
        progress_file: Path to the progress file
        
    Returns:
        tuple: (chapter_idx, paragraph_idx, sentence_idx)
    """
    if os.path.exists(progress_file):
        with open(progress_file, 'r', encoding='utf-8') as f:
            try:
                data = json.load(f)
                return data.get("c", 0), data.get("p", 0), data.get("s", 0)
            except json.JSONDecodeError:
                return 0, 0, 0
    return 0, 0, 0

def load_extended_progress(progress_file):
    """
    Load extended reading progress including UI state.
    
    Args:
        progress_file: Path to the progress file
        
    Returns:
        dict: Progress data with reading position and UI state
    """
    default_progress = {
        "c": 0, "p": 0, "s": 0,
        "scroll_offset": 0,
        "tts_enabled": True,
        "auto_scroll_enabled": True,
        "speed_reading_enabled": False,
        "manual_scroll_anchor": None,
        "playback_speed": 1.0
    }
    
    if not os.path.exists(progress_file):
        return default_progress
        
    try:
        with open(progress_file, 'r', encoding='utf-8') as f:
            data = json.load(f)
            return {
                "c": data.get("c", 0),
                "p": data.get("p", 0), 
                "s": data.get("s", 0),
                "scroll_offset": data.get("scroll_offset", 0),
                "tts_enabled": data.get("tts_enabled", True),
                "auto_scroll_enabled": data.get("auto_scroll_enabled", True),
                "speed_reading_enabled": data.get("speed_reading_enabled", False),
                "manual_scroll_anchor": data.get("manual_scroll_anchor", None),
                "playback_speed": data.get("playback_speed", 1.0)
            }
    except (json.JSONDecodeError, IOError):
        return default_progress

def save_progress(progress_file, chapter_idx, paragraph_idx, sentence_idx):
    """
    Save basic reading progress to file.
    
    Args:
        progress_file: Path to the progress file
        chapter_idx: Current chapter index
        paragraph_idx: Current paragraph index
        sentence_idx: Current sentence index
    """
    progress = {"c": chapter_idx, "p": paragraph_idx, "s": sentence_idx}
    with open(progress_file, 'w', encoding='utf-8') as f:
        json.dump(progress, f, indent=2)

def save_extended_progress(progress_file, chapter_idx, paragraph_idx, sentence_idx, 
                          scroll_offset, tts_enabled, auto_scroll_enabled, manual_scroll_anchor=None, original_file_path=None, playback_speed=1.0, percentage=0.0, speed_reading_enabled=False):
    """
    Save extended reading progress including UI state.
    
    Args:
        progress_file: Path to the progress file
        chapter_idx: Current chapter index
        paragraph_idx: Current paragraph index
        sentence_idx: Current sentence index
        scroll_offset: Current scroll position
        tts_enabled: Whether TTS is enabled
        auto_scroll_enabled: Whether auto-scroll is enabled
        manual_scroll_anchor: Manual scroll anchor position (optional)
        original_file_path: Original path to the eBook file (optional)
        playback_speed: Audio playback speed
        percentage: Completion percentage (0.0 to 100.0)
        speed_reading_enabled: Whether speed reading mode is enabled
    """
    progress = {
        "c": chapter_idx,
        "p": paragraph_idx, 
        "s": sentence_idx,
        "scroll_offset": float(scroll_offset),
        "tts_enabled": bool(tts_enabled),
        "auto_scroll_enabled": bool(auto_scroll_enabled),
        "speed_reading_enabled": bool(speed_reading_enabled),
        "playback_speed": float(playback_speed),
        "completion_percentage": float(percentage)
    }
    if manual_scroll_anchor:
        progress["manual_scroll_anchor"] = manual_scroll_anchor
    if original_file_path:
        progress["original_file_path"] = original_file_path

    # 原子写入：先写临时文件再替换，避免异常退出时进度文件写一半损坏
    tmp_file = progress_file + ".tmp"
    with open(tmp_file, 'w', encoding='utf-8') as f:
        json.dump(progress, f, indent=2)
    os.replace(tmp_file, progress_file)

def _iter_progress_files():
    """Yield every progress record file, including dotfiles.

    Older versions sanitized CJK titles to an empty string, so records were
    written to `.progress.json` (a dotfile that `glob("*.progress.json")`
    misses). Scanning the directory instead keeps those records visible.
    """
    try:
        with os.scandir(config.PROGRESS_FILE_DIR) as it:
            for entry in it:
                if entry.is_file() and entry.name.endswith(".progress.json"):
                    yield entry.path
    except FileNotFoundError:
        return


def list_read_books():
    """Return every readable saved-book record, newest first.

    Unlike ``get_recent_books`` this intentionally has no five-book UI
    limit. Each record includes the progress-file timestamp so the CLI can
    present a stable history.
    """
    progress_files = list(_iter_progress_files())
    progress_files.sort(key=os.path.getmtime, reverse=True)

    records = []
    for pf in progress_files:
        try:
            with open(pf, 'r', encoding='utf-8') as f:
                data = json.load(f)
            original_path = data.get("original_file_path")
            if not original_path or not os.path.exists(original_path):
                continue
            records.append({
                "title": os.path.splitext(os.path.basename(original_path))[0],
                "path": original_path,
                "percentage": float(data.get("completion_percentage", 0.0)),
                "modified_time": os.path.getmtime(pf),
            })
        except (json.JSONDecodeError, OSError, TypeError, ValueError):
            continue
    return records


def get_recent_books(limit=5):
    """Get a limited recent-book list for the interactive ``r`` menu."""
    return [
        {key: value for key, value in record.items() if key != "modified_time"}
        for record in list_read_books()[:limit]
    ]


def clear_reading_data():
    """Delete reading records and derived book caches.

    Original book files, user settings, logs and audio cache are never
    touched. The operation is idempotent and reports deletion counts plus
    per-path errors for noninteractive CLI callers.
    """
    result = {"progress": 0, "parsed": 0, "txt_index": 0, "errors": []}

    for pf in _iter_progress_files():
        try:
            os.remove(pf)
            result["progress"] += 1
        except OSError as exc:
            result["errors"].append((pf, str(exc)))

    for dirname, key in (("parsed", "parsed"), ("txt-index", "txt_index")):
        directory = os.path.join(config.PROGRESS_FILE_DIR, dirname)
        try:
            entries = list(os.scandir(directory))
        except FileNotFoundError:
            continue
        except OSError as exc:
            result["errors"].append((directory, str(exc)))
            continue
        for entry in entries:
            try:
                if entry.is_file() or entry.is_symlink():
                    os.remove(entry.path)
                    result[key] += 1
                elif entry.is_dir():
                    result["errors"].append((entry.path, "unexpected directory skipped"))
            except OSError as exc:
                result["errors"].append((entry.path, str(exc)))
    return result

def validate_and_set_progress(chapters, progress_file, c, p, s):
    """
    Validate reading progress against document structure.
    
    Args:
        chapters: Document chapters structure
        progress_file: Path to progress file (for cleanup if invalid)
        c: Chapter index to validate
        p: Paragraph index to validate
        s: Sentence index to validate
        
    Returns:
        tuple: Valid (chapter_idx, paragraph_idx, sentence_idx)
    """
    try:
        paragraph = chapters[c][p]
        sentences = content_parser.split_into_sentences(paragraph)
        _ = sentences[s]  # Test if sentence exists
        return c, p, s
    except IndexError:
        # Invalid progress, reset to beginning
        if os.path.exists(progress_file):
            os.remove(progress_file)
        return 0, 0, 0

def find_most_recent_book():
    """
    Find the most recently updated progress file and return the original file path.
    
    Returns:
        str or None: Path to the most recently read book, or None if no books found
    """
    progress_files = list(_iter_progress_files())
    
    if not progress_files:
        return None
    
    # Find the most recently modified progress file
    most_recent_file = max(progress_files, key=os.path.getmtime)
    
    try:
        with open(most_recent_file, 'r', encoding='utf-8') as f:
            data = json.load(f)
            original_path = data.get("original_file_path")
            
            # Check if the original file still exists
            if original_path and os.path.exists(original_path):
                return original_path
                
    except (json.JSONDecodeError, IOError):
        pass
    
    return None
