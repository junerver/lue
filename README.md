<div align="center">

<img src="images/lue-icon.png" alt="Lue-rs Icon" width="35%" />

### Lue-rs - Fast Terminal eBook Reader with Text-to-Speech

<img src="images/lue-screenshot.gif" alt="Lue Screenshot" width="100%" />

Lue-rs is a performance fork of [Lue](https://github.com/paulilaaso/lue), a versatile terminal eBook and document reader that blends reading and listening. It keeps the full Lue feature set — multi-format support, modular multilingual text-to-speech with synchronized word highlighting, auto-scrolling, chapter navigation and persistent progress tracking — while reworking the engine for instant startup.

</div>

---

## What's Different from Upstream Lue

| **Area**                  | **Upstream Lue**                                | **Lue-rs**                                                                        |
| ------------------------- | ----------------------------------------------- | --------------------------------------------------------------------------------- |
| **Parsing & layout**      | Pure Python                                     | Hot paths rewritten in Rust (PyO3): sentence splitting, text cleaning, TXT chapter-rule scanning, line layout |
| **Document model**        | Reads and parses the whole file up front        | Windowed rendering: the reader never loads the full book — a byte-level TXT index enables random access, and only the current chapter (± a small window) is decoded and laid out |
| **Startup**               | Grows with book size (an 11 MB novel took ~4 s) | Near-instant: ~0.1 s cold open (indexing an 11 MB / 1097-chapter novel takes 64 ms), 11–45 ms warm opens, ~7 ms chapter jumps |
| **Native binary**         | Python package only                             | A pure-Rust Ratatui binary (`lue`, in `rust/app`) with zero Python dependency — see [Pure Rust binary](#pure-rust-binary) |
| **Startup pipeline**      | TTS + audio stack initialized eagerly          | Simplified launch: TTS disabled by default, heavy imports deferred, parsed/index/layout results cached on disk |
| **Management commands**   | —                                               | `lue list` (reading history) and `lue clear` (records + caches)                   |

The Rust accelerator is **optional** in the Python package: without the `lue_rs` wheel everything falls back to the retained pure-Python implementations, and `LUE_NO_RUST=1` disables it at runtime for A/B testing. See [docs/RUST_REWRITE.md](docs/RUST_REWRITE.md) for the architecture notes.

---

## Pure Rust binary

The native build is a standalone Ratatui/Crossterm terminal reader with **no Python dependency**. It currently opens titled Chinese TXT books (the chapter-index fast path) and reads/writes the same progress records as the Python build, so your history and positions carry over.

```bash
# from the repository root: build everything (PyO3 wheel + native binary)
(cd rust && cargo build --release --workspace)

# the binary lands here
rust/target/release/lue            # lue.exe on Windows

# open a book
rust/target/release/lue path/to/book.txt

# same management commands as the Python build
rust/target/release/lue list
rust/target/release/lue clear

# install for daily use (optional)
cp rust/target/release/lue ~/.local/bin/lue-rs
```

Keys: `j`/`k` line, `h`/`l` paragraph, `u`/`n` small scroll, `i`/`m` page, `z`/`x` chapter, `c` chapter index (type-to-filter), `r` recent books, `y`/`b` start/end, `v` UI mode, `q` quit; arrows, PgUp/PgDn, Home/End, mouse wheel and progress-bar clicks work too. `--keys vim` selects the Vim layout, `lue --guide` prints the cheat sheet.

TTS is not wired into the native binary yet (it reads silently); the Python build remains the full-featured TTS client. Other formats (EPUB/PDF/DOCX/HTML/RTF/MD) are planned for the native parser in later phases — see the roadmap.

---

## Features

| **Feature**                          | **Description**                                                                                |
| ------------------------------------ | ---------------------------------------------------------------------------------------------- |
| **Multi-Format Support**             | Support for EPUB, PDF, TXT, DOCX, HTML, RTF, and Markdown with seamless format detection  |
| **Rust-Accelerated Engine**          | Parsing, chapter indexing, and layout run in Rust (PyO3) with graceful pure-Python fallbacks |
| **Windowed Rendering**               | Only the visible chapter window is decoded and laid out, so open time is independent of book size |
| **Modular TTS System**               | Edge TTS and Kokoro TTS (local/offline) with extensible architecture for new models; disabled by default for the fastest startup |
| **Cross-Platform & Multilingual**    | Full support for macOS, Linux, Windows (via WSL) with 100+ languages and consistent global experience    |
| **Speed Adjustment**                 | Adjust text-to-speech playback speed from 1x to 3x for personalized listening experience       |
| **Auto-Scroll & Precise Word Highlighting** | Automatic scrolling and word-level highlighting synchronized with actual speech, improving focus and concentration     |
| **Smart Persistence**                | Automatic progress saving, state restoration, and cross-session continuity for seamless reading|
| **Fast Navigation**                  | Intuitive shortcuts, mouse support, smooth scrolling and chapters list for fast navigation.     |
| **Reading History Management**       | `lue list` shows every book in your history; `lue clear` wipes records and caches in one step |
| **Extensive Customization**          | Fully customizable keyboard layouts (including Vim-style bindings), adjustable UI elements, colors, and display modes|

---

## Quick Start (macOS and Linux)

> **Want to try Lue-rs right away?** Follow these simple steps:

```bash
# 1. Install Rust (used only for the optional accelerator)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# 2. Clone this fork
git clone https://github.com/junerver/lue-rs.git
cd lue-rs

# 3. Install the Python package (standalone, pure-Python fallback included)
pip install .

# 4. Optional but recommended: build and install the Rust accelerator
pip install maturin
(cd rust && maturin build --release)
pip install rust/target/wheels/lue_rs-*.whl

# 5. Practice using Lue-rs with the navigation guide
lue --guide

# 6. Start reading!
lue path/to/your/book.txt
```

> **Note:** TTS is disabled by default, so no audio stack is needed to start reading. FFmpeg is only required when you launch with `--tts edge` / `--tts kokoro`.

---

## Installation (macOS, Linux and Windows)

### Prerequisites

#### Core Requirements
- **Python 3.10+** (required)

#### Optional Dependencies
- **Rust toolchain + maturin** - to build the `lue_rs` accelerator wheel (highly recommended)
- **FFmpeg** - audio processing, only needed for TTS
- **espeak** - Kokoro TTS support

#### macOS (Homebrew)
```bash
brew install ffmpeg
# Optional
brew install espeak
```

#### Ubuntu/Debian
```bash
sudo apt update && sudo apt install ffmpeg
# Optional
sudo apt install espeak
```

#### Windows
```bash
# 1. Install WSL
# Open PowerShell as Administrator:
wsl --install

# 2. Restart your PC if prompted, then launch Ubuntu from Start Menu

# 3. Inside Ubuntu terminal:
sudo apt update && sudo apt upgrade -y
sudo apt install ffmpeg python3 python3-pip -y
# Optional
sudo apt install espeak
```

### Install Lue-rs

#### Standard Installation

```bash
# 1. Clone this fork
git clone https://github.com/junerver/lue-rs.git
cd lue-rs

# 2. Install the Python package
pip install .

# 3. Install the Rust accelerator (optional, recommended)
rustup component add cargo  # if you skipped rustup earlier: https://rustup.rs
pip install maturin
(cd rust && maturin build --release)
pip install rust/target/wheels/lue_rs-*.whl
```

#### Using pipx

```bash
# 1. Install the Python package into its own venv
pipx install .

# 2. Inject the Rust accelerator wheel built in the previous step
pipx inject lue-reader rust/target/wheels/lue_rs-*.whl
```

#### Enable Kokoro TTS (Optional)

For local/offline TTS capabilities:

```bash
pip install ".[kokoro]"

# Install PyTorch
# CPU version:
pip install torch torchvision torchaudio
# GPU version (CUDA):
pip install torch torchvision torchaudio --index-url https://download.pytorch.org/whl/cu121
```

---

## Usage

### Basic Commands

```bash
# Open a book (TTS is disabled by default for the fastest startup)
lue path/to/your/book.epub

# Launch without arguments to open the last book you were reading
lue

# List every readable book in local reading history
lue list

# Clear reading records plus parsed/TXT-index caches (keeps book files,
# settings, logs, and audio cache)
lue clear

# Practice Lue-rs default keys with the navigation guide
lue --guide

# View available command line options
lue --help

# Use a TTS model (edge/kokoro/none; default: none)
lue --tts edge path/to/your/book.epub

# Use a specific voice (full list at VOICES.md)
lue --voice "en-US-AriaNeural" path/to/your/book.epub

# Set the speech speed (e.g., 1.5x)
lue --speed 1.5 path/to/your/book.epub

# Specify a language code if needed
lue --lang a path/to/your/book.epub

# Seconds of overlap between sentences
lue --over 0.2 path/to/your/book.epub

# Enable PDF cleaning filter (removes page numbers, headers and footnotes, default: 10% (0.1) from both bottom and top of the page)
lue --filter path/to/your/book.pdf

# Set custom PDF filter margins (0.0-1.0, where 0.1 = 10% of page)
lue --filter 0.15 path/to/your/book.pdf          # Both margins to 15%
lue --filter 0.12 0.20 path/to/your/book.pdf     # Header 12%, footnote 20%

# Use the Vim keyboard layout
lue --keys vim path/to/your/book.epub

# Start in a specific visual layout mode: 0=minimal, 1=medium, 2=full, 3=speed reading
lue --mode 0 path/to/your/book.epub
lue -m 1 path/to/your/book.epub
lue -m 2 path/to/your/book.epub
lue -m 3 path/to/your/book.epub

```

### Keyboard Controls (Default)

<div align="center">

| **Key Binding**                         | **Action Description**                                                                         |
| --------------------------------------- | ---------------------------------------------------------------------------------------------- |
| `q`                                     | Quit the application and save current reading progress automatically                           |
| `p`                                     | Pause or resume the text-to-speech audio playback                                              |
| `a`                                     | Toggle auto-scroll mode to automatically advance during TTS playback                           |
| `t`                                     | Select and highlight the top sentence of the current visible page                              |
| `h` / `l`                               | Move the reading line to the previous or next paragraph in the document                        |
| `j` / `k`                               | Move the reading line to the previous or next sentence in the document                         |
| `z` / `x`                               | Move the reading line to the previous or next chapter in the document                          |
| `c`                                     | Open the chapter index menu to browse and jump to any chapter                                  |
| `i` / `m`                               | Jump up or down by full pages for rapid navigation through longer documents                    |
| `u` / `n`                               | Scroll up or down by smaller increments for fine-grained position control                      |
| `y` / `b`                               | Jump directly to the beginning or end of the document for quick navigation                     |
| `r`                                     | Open the recent books menu to quickly switch between 5 last read books                       |
| `,` / `.`                               | Decrease or increase text-to-speech playback speed (1x to 3x)                                  |
| `s` / `w`                               | Toggle sentence highlighting or word highlighting on/off                                       |
| `v`                                     | Cycle through UI complexity modes (Minimal, Medium, Full)                                      |

</div>

### Mouse Controls

- **Click** - Jump to sentence
- **Scroll** - Navigate content
- **Progress bar click** - Jump to position

## Customize

### UI Modes

Lue-rs offers four UI complexity modes that you can cycle through using the `v` key, set as your default in the [config.py](lue/config.py) file, or choose at launch with `-m` / `--mode`:

- **Mode 0 (Minimal)** - Clean text-only display with no borders or UI elements
- **Mode 1 (Medium)** - Displays a top title bar with progress information and borders
- **Mode 2 (Full)** - Full UI with both top title bar and bottom control information
- **Mode 3 (Speed Reading)** - Single-word speed-reading display

Additionally, Lue-rs provides customizable word-level and sentence-level highlighting that can be adjusted to suit your reading preferences. You can cycle through different highlighting modes using the `w` and `s` keys. These highlighting settings can also be configured as defaults in the [config.py](lue/config.py) file.

### Keyboard Layouts

Lue-rs comes with two built-in keyboard layouts that can be set using -k/--key command line option or set as your default in the [config.py](lue/config.py) file. You can create your own keyboard layout by copying and modifying one of the existing layout files:

- **Default Layout** - [keys_default.json](lue/keys_default.json) - Standard keyboard layout
- **Vim Layout** - [keys_vim.json](lue/keys_vim.json) - Vim-style keyboard layout
- **Custom Layout** - Customize your own navigation keys by creating your own keyboard layout json file. Each command can accept a single key (e.g., `"play_pause": "p"`) or multiple keys as an array (e.g., `"play_pause": ["p", " "]`).

### Color Themes

Lue-rs allows you to customize the color theme, visual icons/symbols and all ui elements of the interface by modifying the classes in [ui.py](lue/ui.py). Create your own theme or choose one of the three themes that come with the default installation.

- **Default Theme** - The default colorful theme with various colors for different UI elements
- **Black Theme** - A dark monochrome theme that's suitable for bright backgrounds
- **White Theme** - A light monochrome theme that's suitable for dark backgrounds

---

## Development

> **Interested in extending Lue-rs?**

The engine lives in [rust/](rust/) as a three-crate workspace: `core/` is the pure-Rust document library, `src/` holds the PyO3 bindings consumed by the Python package, and `app/` builds the native `lue` binary. Architecture and roadmap are documented in [docs/RUST_REWRITE.md](docs/RUST_REWRITE.md); check out the [Developer Guide](DEVELOPER.md) for adding new TTS models to the Python build.

### Data Storage

**Reading Progress:**
- **macOS:** `~/Library/Application Support/lue/`
- **Linux:** `~/.local/share/lue/`
- **Windows (WSL):** `~/.local/share/lue/` (within WSL filesystem)

**Error Logs:**
- **macOS:** `~/Library/Logs/lue/error.log`
- **Linux:** `~/.cache/lue/log/error.log`
- **Windows (WSL):** `~/.cache/lue/log/error.log` (within WSL filesystem)

---

## Credits

Lue-rs is a fork of [Lue](https://github.com/paulilaaso/lue) by [@paulilaaso](https://github.com/paulilaaso) and its contributors. All credit for the original design and feature set belongs to them; this fork focuses on the Rust-accelerated performance work.

---

## License

This project is licensed under the **GPL-3.0-or-later License** - see the [LICENSE](LICENSE) file for details.

---

<div align="center">

<a title="This tool is Tool of The Week on Terminal Trove, The $HOME of all things in the terminal" href="https://terminaltrove.com/"><img src="https://cdn.terminaltrove.com/media/badges/tool_of_the_week/png/terminal_trove_tool_of_the_week_gold_transparent.png" width="40%" alt="Terminal Trove Tool of The Week" /></a>

</div>
