"""Tests for read-history listing, scoped clear, and early CLI dispatch."""

import json
import os
import time

from lue import cache, config, progress_manager
from lue.management import run_management_command


def write_record(directory, name, path, percentage=0.0, modified=None):
    progress = directory / f"{name}.progress.json"
    progress.write_text(json.dumps({
        "original_file_path": str(path),
        "completion_percentage": percentage,
    }), encoding="utf-8")
    if modified is not None:
        os.utime(progress, (modified, modified))
    return progress


def test_list_read_books_all_sorted_and_skips_invalid(tmp_path, monkeypatch):
    monkeypatch.setattr(config, "PROGRESS_FILE_DIR", str(tmp_path))
    first = tmp_path / "first.txt"
    latest = tmp_path / "latest.txt"
    first.write_text("x", encoding="utf-8")
    latest.write_text("x", encoding="utf-8")
    now = time.time()
    write_record(tmp_path, "first", first, 25.5, now - 20)
    write_record(tmp_path, "latest", latest, 80.0, now - 10)
    write_record(tmp_path, "gone", tmp_path / "missing.txt", 50.0, now)
    (tmp_path / "corrupt.progress.json").write_text("not json", encoding="utf-8")

    records = progress_manager.list_read_books()
    assert [r["title"] for r in records] == ["latest", "first"]
    assert records[0]["percentage"] == 80.0
    assert records[0]["path"] == str(latest)
    assert [r["title"] for r in progress_manager.get_recent_books(limit=1)] == ["latest"]


def test_clear_reading_data_scopes_to_records_and_derived_caches(tmp_path, monkeypatch):
    monkeypatch.setattr(config, "PROGRESS_FILE_DIR", str(tmp_path))
    book = tmp_path / "book.txt"
    book.write_text("source must survive", encoding="utf-8")
    write_record(tmp_path, "book", book)

    parsed = tmp_path / "parsed"
    index = tmp_path / "txt-index"
    parsed.mkdir()
    index.mkdir()
    (parsed / "x.cache").write_bytes(b"derived")
    (index / "y.cache").write_bytes(b"derived")
    unrelated = tmp_path / "settings.json"
    unrelated.write_text("keep", encoding="utf-8")

    result = progress_manager.clear_reading_data()
    assert result["progress"] == 1
    assert result["parsed"] == 1
    assert result["txt_index"] == 1
    assert result["errors"] == []
    assert book.exists()
    assert unrelated.exists()
    assert not list(parsed.iterdir())
    assert not list(index.iterdir())
    # Repeating clear is harmless.
    assert progress_manager.clear_reading_data()["progress"] == 0


def test_list_cli_is_early_noninteractive(tmp_path, monkeypatch, capsys):
    monkeypatch.setattr(config, "PROGRESS_FILE_DIR", str(tmp_path))
    book = tmp_path / "book.txt"
    book.write_text("x", encoding="utf-8")
    write_record(tmp_path, "book", book, 42.0)

    assert run_management_command(["list"]) is True
    out = capsys.readouterr().out
    assert "42%" in out
    assert str(book) in out
    # Exact token policy: paths/options are not management commands.
    assert run_management_command(["./list"]) is False
    assert run_management_command(["clear", "extra"]) is False


def test_clear_cli_reports_scope(tmp_path, monkeypatch, capsys):
    monkeypatch.setattr(config, "PROGRESS_FILE_DIR", str(tmp_path))
    book = tmp_path / "book.txt"
    book.write_text("x", encoding="utf-8")
    write_record(tmp_path, "book", book)

    assert run_management_command(["clear"]) is True
    out = capsys.readouterr().out
    assert "Cleared 1 reading record(s)" in out
    assert book.exists()
