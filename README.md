<div align="center">

<img src="images/lue-icon.png" alt="Lue-rs Icon" width="35%" />

### Lue-rs - Fast Terminal eBook Reader with Text-to-Speech

<img src="images/lue-screenshot.gif" alt="Lue Screenshot" width="100%" />

Lue-rs is a performance fork of [Lue](https://github.com/paulilaaso/lue), a versatile terminal eBook and document reader that blends reading and listening — fully rewritten in Rust. It keeps the core Lue experience — multi-format support, multilingual text-to-speech, chapter navigation and persistent progress tracking — with instant startup and a zero-dependency native binary.

</div>

---

## Why Lue-rs

| **Area**               | **Upstream Lue**                                | **Lue-rs**                                                                        |
| ---------------------- | ----------------------------------------------- | ---------------------------------------------------------------------------------- |
| **Language**           | Pure Python                                     | 100% Rust — no Python runtime, no wheels, one static binary                        |
| **Parsing & layout**   | Whole-file parse on open                        | Byte-level TXT chapter index, windowed rendering: only the current chapter (± a small window) is decoded and laid out |
| **Startup**            | Grows with book size (an 11 MB novel took ~4 s) | Near-instant: ~0.1 s cold open (indexing an 11 MB / 1097-chapter novel takes 64 ms), 11–45 ms warm opens, ~7 ms chapter jumps |
| **Formats**            | EPUB, PDF, TXT, DOCX, HTML, RTF, MD             | Same — TXT/EPUB/PDF/DOCX/HTML/MD/RTF, all parsed natively                          |
| **Text-to-Speech**     | Edge TTS + Kokoro (Python)                      | Edge TTS re-implemented natively (WebSocket + SSML), played through ffplay         |
| **Progress records**   | —                                               | Same JSON records and data directories as upstream, so existing history carries over |

---

## Features

| **Feature**                        | **Description**                                                                              |
| ---------------------------------- | ---------------------------------------------------------------------------------------------- |
| **Multi-Format Support**           | TXT, EPUB, PDF, DOCX, HTML, RTF and Markdown, parsed natively in Rust                        |
| **Windowed Rendering**             | Only the visible chapter window is decoded and laid out, so open time is independent of book size |
| **Text-to-Speech**                 | `--tts edge` reads your book aloud via Microsoft Edge's online TTS (mp3 played by ffplay); `LUE_TTS_VOICE` selects the voice |
| **Smart Persistence**              | Automatic progress saving, state restoration and cross-session continuity                     |
| **Fast Navigation**                | Chapter index with type-to-filter, recent-books menu, mouse wheel and progress-bar click jumps |
| **Keyboard Layouts**               | Default and Vim-style layouts, shipped as JSON and trivially customizable                      |
| **UI Modes**                       | Four complexity levels from minimal text-only to full chrome, cycled with `v`                 |

---

## Installation

### Prerequisites

- **Rust toolchain** — <https://rustup.rs>
- **ffmpeg/ffplay** — only needed for `--tts edge`

```bash
# Ubuntu/Debian
sudo apt update && sudo apt install ffmpeg
# macOS
brew install ffmpeg
```

### Build from source

```bash
git clone https://github.com/junerver/lue-rs.git
cd lue-rs
cargo build --release

# the binary lands here
target/release/lue            # lue.exe on Windows

# install for daily use (optional)
cp target/release/lue ~/.local/bin/lue-rs
ln -s ~/.local/bin/lue-rs ~/.local/bin/lue
```

---

## Usage

```bash
# Open a book
lue path/to/your/book.txt

# Launch without arguments to open the last book you were reading
lue

# List every readable book in local reading history
lue list

# Clear reading records plus index/extract caches (keeps book files)
lue clear

# Print the navigation guide
lue --guide

# Read aloud with Edge TTS (ffplay required)
lue --tts edge path/to/your/book.txt

# Pick a voice (default: zh-CN-XiaoxiaoNeural)
LUE_TTS_VOICE=en-US-AriaNeural lue --tts edge path/to/your/book.txt

# Use the Vim keyboard layout
lue --keys vim path/to/your/book.txt

# Start in a specific UI mode: 0=minimal, 1=medium, 2=full, 3=speed reading
lue -m 0 path/to/your/book.txt
```

### Keyboard Controls (Default)

| **Key Binding**   | **Action Description**                                                |
| ----------------- | ---------------------------------------------------------------------- |
| `q`               | Quit and save current reading progress automatically                  |
| `a`               | Start / stop read-aloud from the current position (`--tts edge`)      |
| `p`               | Pause or resume read-aloud                                            |
| `h` / `l`         | Previous / next paragraph                                              |
| `j` / `k`         | Previous / next sentence (line)                                        |
| `z` / `x`         | Previous / next chapter                                                |
| `c`               | Chapter index menu — type to filter, Enter jumps                      |
| `i` / `m`         | Page up / page down                                                    |
| `u` / `n`         | Small scroll up / down (3 lines)                                       |
| `y` / `b`         | Jump to the beginning / end of the book                                |
| `r`               | Recent books menu (last 5)                                             |
| `v`               | Cycle UI complexity modes (Minimal, Medium, Full, Full)                |

Arrows, PgUp/PgDn, Home/End, the mouse wheel and progress-bar clicks work too.

### Voice Selection

Read-aloud uses Microsoft Edge's neural voices. Set `LUE_TTS_VOICE` to any
Edge voice name (for example `zh-CN-YunxiNeural`, `en-US-AriaNeural`); the
default is `zh-CN-XiaoxiaoNeural`. See [VOICES.md](VOICES.md) for the voice
and language guide.

---

## Development

The project is a two-crate Cargo workspace:

- **`core` (`lue_core`)** — pure library logic: TXT byte-level chapter indexing, TOC rules, sentence splitting and text cleaning, EPUB/PDF/DOCX/HTML/MD/RTF extraction, layout and wrapping, the Edge TTS client.
- **`app` (`lue`)** — the Ratatui/Crossterm terminal reader binary.

```bash
# run the test suite
cargo test --workspace
```

`benchmarks/wsl_redeploy.sh` rebuilds and installs the binary into a WSL
environment. Architecture notes live in [docs/](docs/).

### Data Storage

- **Linux:** `~/.local/share/lue/`
- **macOS:** `~/Library/Application Support/lue/`

---

## Credits

Lue-rs is a fork of [Lue](https://github.com/paulilaaso/lue) by [@paulilaaso](https://github.com/paulilaaso) and its contributors. All credit for the original design and feature set belongs to them; this fork focuses on the pure-Rust rewrite.

---

## License

This project is licensed under the **GPL-3.0-or-later License** - see the [LICENSE](LICENSE) file for details.

---

<div align="center">

<a title="This tool is Tool of The Week on Terminal Trove, The $HOME of all things in the terminal" href="https://terminaltrove.com/"><img src="https://cdn.terminaltrove.com/media/badges/tool_of_the_week/png/terminal_trove_tool_of_the_week_gold_transparent.png" width="40%" alt="Terminal Trove Tool of The Week" /></a>

</div>
