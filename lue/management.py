"""Noninteractive reading-history management commands.

Kept separate from ``__main__`` so list/clear stay importable and testable on
non-POSIX hosts where the interactive terminal stack (termios/tty) is absent.
"""

import sys
from datetime import datetime

from . import progress_manager


def run_management_command(argv):
    """Run ``lue list`` / ``lue clear`` and return whether one was handled.

    Exact one-token matching avoids stealing ordinary files named ``list`` or
    ``clear`` when opened as ``./list`` or an absolute path.
    """
    if len(argv) != 1 or argv[0] not in {"list", "clear"}:
        return False

    command = argv[0]
    if command == "list":
        records = progress_manager.list_read_books()
        if not records:
            print("No readable books in history.")
            return True
        for record in records:
            timestamp = datetime.fromtimestamp(record["modified_time"]).strftime(
                "%Y-%m-%d %H:%M"
            )
            print(
                f"{timestamp}\t{record['percentage']:.0f}%\t"
                f"{record['title']}\t{record['path']}"
            )
        return True

    result = progress_manager.clear_reading_data()
    print(
        "Cleared "
        f"{result['progress']} reading record(s), "
        f"{result['parsed']} parsed cache file(s), "
        f"{result['txt_index']} TXT index file(s)."
    )
    for path, error in result["errors"]:
        print(f"Warning: could not clear {path}: {error}", file=sys.stderr)
    return True
