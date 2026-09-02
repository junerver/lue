# Lue-rs Developer Guide

Lue-rs is a two-crate Cargo workspace at the repository root — everything is
Rust, there is no Python toolchain anymore.

## Layout

```
Cargo.toml          # virtual workspace manifest
core/               # lue_core — pure library logic, no I/O or UI
├── src/book.rs       # TXT byte-level chapter index + random-access TxtBook
├── src/toc.rs        # chapter-title rules (legado-derived, scored)
├── src/extract.rs    # EPUB/PDF/DOCX/HTML/MD/RTF text extraction
├── src/tts.rs        # Edge TTS client (tungstenite + rustls)
├── src/text.rs       # sentence split, visual cleaning, CJK analysis
├── src/wrap.rs       # cell-width wrapping and cell tables
└── src/layout.rs     # paragraph/line layout for chapter windows
app/                # lue — the Ratatui/Crossterm reader binary
├── src/main.rs       # CLI + event loop
├── src/reader.rs     # ReaderState: chapter window, scroll, progress
├── src/library.rs    # open() dispatch: TXT index / extraction cache
├── src/tts.rs        # TTS manager thread (synthesis queue + ffplay)
├── src/render.rs     # drawing + overlays
├── src/input.rs      # key layouts → actions
└── src/store.rs      # progress records + caches on disk
```

## Build and test

```bash
cargo build --release --workspace
cargo test --workspace        # 54 tests; two are #[ignore]d live-network tests
cargo test -p lue_core live_synthesis -- --ignored   # real edge-tts round trip
```

## Architecture notes

- **TXT books** are never fully decoded. `build_txt_index` scans raw bytes
  once, picks a chapter-title rule (scored over the first 500 KiB) and
  records byte spans per chapter; `TxtBook` seeks and decodes a chapter only
  when it is read.
- **Every other format** is extracted into a normalized text file under
  `cache/extracted/` (keyed by source fingerprint) and indexed the same way.
  Extraction already knows the chapter boundaries, so no title rules run.
- **Untitled TXT** falls back to `build_whole_book_index`: ~10 KiB chunks.
- **TTS** runs on a manager thread: sentences are synthesized one by one
  into rotating temp buffers and played by an ffplay subprocess. Pause aborts
  the current sentence; resume replays it.
- **Progress records** live next to the caches in the platform data dir
  (`~/.local/share/lue/` on Linux) as JSON keyed by sanitized book title.

## Deploy to WSL

`benchmarks/wsl_redeploy.sh` rsyncs the tree to `~/lue-src`, builds the
release workspace there and installs the binary as `~/.local/bin/lue-rs`
(`lue` is a symlink). Build on the WSL filesystem, never from `/mnt/e`.
