//! lue — pure-Rust terminal eBook reader binary.
//!
//! TXT books open through the byte-level chapter index (`lue_core`), the
//! UI renders the pre-wrapped chapter window with Ratatui/Crossterm, and
//! reading progress persists in the same records the Python build uses.

mod input;
mod library;
mod reader;
mod render;
mod store;

use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind, MouseEventKind,
};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::Terminal;
use render::Overlay;
use std::io::stdout;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use input::{Action, KeyMap};
use reader::ReaderState;

#[derive(Parser)]
#[command(
    name = "lue",
    version,
    about = "lue-rs: fast pure-Rust terminal eBook reader (TXT, chapter-indexed)"
)]
struct Cli {
    /// Book file to open. Omit to reopen the most recent book.
    book: Option<PathBuf>,
    /// Start in a specific UI complexity mode (0=minimal, 1=medium, 2=full, 3=full)
    #[arg(short = 'm', long, default_value_t = 2, value_parser = clap::value_parser!(u8).range(0..=3))]
    mode: u8,
    /// Keyboard layout shipped with lue (default|vim)
    #[arg(short = 'k', long, default_value = "default")]
    keys: String,
    /// TTS engine; the native binary reads silently for now
    #[arg(long, default_value = "none")]
    tts: String,
    /// Print the navigation guide and exit
    #[arg(long)]
    guide: bool,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// List every readable book in local reading history
    List,
    /// Clear reading records and caches (keeps book files and settings)
    Clear,
}

enum Flow {
    Quit,
    Switch(PathBuf),
}

fn main() -> Result<()> {
    install_panic_hook();
    let cli = Cli::parse();
    if cli.guide {
        print_guide();
        return Ok(());
    }
    match cli.command {
        Some(Command::List) => return run_list(),
        Some(Command::Clear) => return run_clear(),
        None => {}
    }
    if cli.tts != "none" {
        bail!(
            "--tts {} is not available in the native binary yet; \
             it reads silently (omit --tts)",
            cli.tts
        );
    }
    let path = match cli.book {
        Some(p) => p,
        None => store::most_recent_path().unwrap_or_else(|| {
            bail_exit("no book given and no reading history found");
        }),
    };
    let mut current = path;
    loop {
        let flow = run_app(&current, cli.mode, &cli.keys)?;
        match flow {
            Flow::Quit => break,
            Flow::Switch(next) => current = next,
        }
    }
    Ok(())
}

fn bail_exit(msg: &str) -> ! {
    eprintln!("{msg}");
    std::process::exit(1);
}

/// A panic inside the TUI would otherwise strand the terminal on a blank
/// alternate screen. Best-effort restore raw mode / the main screen, then
/// let the default hook print the message (plus a copy in a log file for
/// headless diagnosis).
fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = crossterm::execute!(
            std::io::stdout(),
            LeaveAlternateScreen,
            DisableMouseCapture
        );
        let log_path = std::env::temp_dir().join("lue-rs-panic.log");
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
        {
            use std::io::Write;
            let _ = writeln!(file, "{info}");
            if let Some(loc) = info.location() {
                let _ = writeln!(file, "at {loc}");
            }
        }
        default_hook(info);
    }));
}

fn run_list() -> Result<()> {
    let records = store::list_books();
    if records.is_empty() {
        println!("No readable books in history.");
        return Ok(());
    }
    for record in records {
        let secs = record
            .modified
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_secs();
        let (y, mo, d, h, mi) = epoch_to_local_naive(secs);
        println!(
            "{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}\t{:.0}%\t{}\t{}",
            record.percentage,
            record.title,
            record
                .path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default()
        );
    }
    Ok(())
}

/// Local-time `%Y-%m-%d %H:%M` without pulling chrono in: seconds→date via
/// the civil-from-days algorithm (valid for the full u64 seconds range).
fn epoch_to_local_naive(secs: u64) -> (u64, u64, u64, u64, u64) {
    // UTC; the Python build uses local time, so timestamps may differ by the
    // zone offset. Reading history stays sortable and human-readable.
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let (h, mi) = (rem / 3600, (rem % 3600) / 60);
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z % 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d, h, mi)
}

fn run_clear() -> Result<()> {
    let result = store::clear()?;
    println!(
        "Cleared {} reading record(s), {} native cache file(s).",
        result.progress, result.cache
    );
    for (path, error) in result.errors {
        eprintln!("Warning: could not clear {}: {error}", path.display());
    }
    Ok(())
}

fn print_guide() {
    println!(
        "lue navigation\n\
         \n\
         j/k        scroll one line\n\
         h/l        previous / next paragraph\n\
         u/n        small scroll (3 lines)\n\
         i/m        page up / page down\n\
         z/x        previous / next chapter\n\
         c          chapter index (type to filter, Enter jumps)\n\
         r          recent books\n\
         y/b        beginning / end of book\n\
         v          cycle UI complexity (0-3)\n\
         q          quit (progress saved)\n\
         \n\
         arrows, PgUp/PgDn, Home/End and the mouse wheel also work;\n\
         clicking the footer progress bar jumps to that position."
    );
}

fn run_app(path: &Path, ui_mode: u8, keys_layout: &str) -> Result<Flow> {
    let keymap = input::load(keys_layout)?;

    enable_raw_mode()?;
    let mut stdout_guard = stdout();
    crossterm::execute!(stdout_guard, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout_guard);
    let mut terminal = Terminal::new(backend)?;

    let flow = event_loop(&mut terminal, path, ui_mode, &keymap);

    disable_raw_mode()?;
    crossterm::execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    flow
}

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    path: &Path,
    ui_mode: u8,
    keymap: &KeyMap,
) -> Result<Flow> {
    let size = terminal.size()?;
    let mut state = ReaderState::open(path, size.width, size.height)?;
    let progress_file = store::progress_path(&state.title);
    state.restore(&store::load_progress(&progress_file))?;

    let mut ui_mode = ui_mode;
    let mut overlay = Overlay::None;

    loop {
        terminal.draw(|f| {
            render::draw(f, &mut state, ui_mode, &mut overlay)
                .expect("terminal draw failed");
        })?;

        if !event::poll(Duration::from_millis(250))? {
            continue;
        }
        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                match &mut overlay {
                    Overlay::None => {
                        if let Some(action) = input::decode_key(key.code, keymap) {
                            match action {
                                Action::Quit => {
                                    store::save_progress(&progress_file, &state.to_record())?;
                                    return Ok(Flow::Quit);
                                }
                                Action::NextLine => state.scroll_down(1)?,
                                Action::PrevLine => state.scroll_up(1)?,
                                Action::PageDown => state.page_down()?,
                                Action::PageUp => state.page_up()?,
                                Action::ScrollDown => state.scroll_down(3)?,
                                Action::ScrollUp => state.scroll_up(3)?,
                                Action::NextParagraph => {
                                    let line = state.next_paragraph_start(state.scroll);
                                    state.scroll = line.min(state.max_scroll());
                                }
                                Action::PrevParagraph => {
                                    state.scroll = state.prev_paragraph_start(state.scroll);
                                }
                                Action::NextChapter => {
                                    state.next_chapter()?;
                                    store::save_progress(
                                        &progress_file,
                                        &state.to_record(),
                                    )?;
                                }
                                Action::PrevChapter => {
                                    state.prev_chapter()?;
                                    store::save_progress(
                                        &progress_file,
                                        &state.to_record(),
                                    )?;
                                }
                                Action::MoveToBeginning => {
                                    state.goto_top()?;
                                }
                                Action::MoveToEnd => state.goto_bottom(),
                                Action::CycleUiComplexity => ui_mode = (ui_mode + 1) % 4,
                                Action::ToggleChapterIndex => {
                                    let titles = state.chapter_titles()?;
                                    overlay = Overlay::Chapters {
                                        titles,
                                        filter: String::new(),
                                        selected: state.center,
                                    };
                                }
                                Action::ToggleRecentMenu => {
                                    let items = store::list_books()
                                        .into_iter()
                                        .take(5)
                                        .map(|r| {
                                            (
                                                r.title,
                                                r.path
                                                    .map(|p| p.display().to_string())
                                                    .unwrap_or_default(),
                                            )
                                        })
                                        .collect();
                                    overlay = Overlay::Recent { items, selected: 0 };
                                }
                                Action::SelectMenuItem | Action::Ignored => {}
                            }
                        }
                    }
                    Overlay::Chapters { titles, filter, selected } => {
                        match key.code {
                            crossterm::event::KeyCode::Esc => overlay = Overlay::None,
                            crossterm::event::KeyCode::Enter => {
                                let target = *selected;
                                state.jump_chapter(target)?;
                                store::save_progress(&progress_file, &state.to_record())?;
                                overlay = Overlay::None;
                            }
                            crossterm::event::KeyCode::Backspace => {
                                filter.pop();
                                *selected = first_visible(titles, filter).unwrap_or(0);
                            }
                            crossterm::event::KeyCode::Up => {
                                move_selection(titles, filter, selected, false);
                            }
                            crossterm::event::KeyCode::Down => {
                                move_selection(titles, filter, selected, true);
                            }
                            crossterm::event::KeyCode::Char(c) => {
                                filter.push(c);
                                *selected = first_visible(titles, filter).unwrap_or(0);
                            }
                            _ => {}
                        }
                    }
                    Overlay::Recent { items, selected } => {
                        match key.code {
                            crossterm::event::KeyCode::Esc => overlay = Overlay::None,
                            crossterm::event::KeyCode::Enter => {
                                if let Some((_, p)) = items.get(*selected) {
                                    let path = PathBuf::from(p);
                                    if path.is_file() {
                                        return Ok(Flow::Switch(path));
                                    }
                                }
                            }
                            crossterm::event::KeyCode::Up => {
                                *selected = selected.saturating_sub(1);
                            }
                            crossterm::event::KeyCode::Down => {
                                *selected = (*selected + 1).min(items.len().saturating_sub(1));
                            }
                            _ => {}
                        }
                    }
                }
            }
            Event::Mouse(mouse) => match mouse.kind {
                MouseEventKind::ScrollUp => state.scroll_up(render::MOUSE_SCROLL_LINES)?,
                MouseEventKind::ScrollDown => state.scroll_down(render::MOUSE_SCROLL_LINES)?,
                MouseEventKind::Down(_) if ui_mode >= 2 => {
                    let size = terminal.size()?;
                    if let Some(pct) = render::progress_bar_click(
                        mouse.column,
                        mouse.row,
                        Rect::new(0, 0, size.width, size.height),
                    ) {
                        let target = ((pct / 100.0) * state.total_chapters as f64) as usize;
                        state.jump_chapter(target)?;
                        store::save_progress(&progress_file, &state.to_record())?;
                    }
                }
                _ => {}
            },
            Event::Resize(_, _) => {} // next draw() re-syncs the viewport
            _ => {}
        }
    }
}

fn first_visible(titles: &[String], filter: &str) -> Option<usize> {
    let needle = filter.to_lowercase();
    titles
        .iter()
        .position(|t| needle.is_empty() || t.to_lowercase().contains(&needle))
}

fn move_selection(titles: &[String], filter: &str, selected: &mut usize, forward: bool) {
    let needle = filter.to_lowercase();
    let visible: Vec<usize> = titles
        .iter()
        .enumerate()
        .filter(|(_, t)| needle.is_empty() || t.to_lowercase().contains(&needle))
        .map(|(i, _)| i)
        .collect();
    if visible.is_empty() {
        return;
    }
    match visible.iter().position(|&i| i == *selected) {
        Some(pos) => {
            let new_pos = if forward {
                (pos + 1).min(visible.len() - 1)
            } else {
                pos.saturating_sub(1)
            };
            *selected = visible[new_pos];
        }
        None => *selected = visible[0],
    }
}

#[cfg(test)]
mod window_tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn make_book(dir: &std::path::Path, chapters: usize) -> PathBuf {
        let mut text = String::from("书名页\n\n");
        for c in 1..=chapters {
            text.push_str(&format!("第{c}章 测试章节标题{c}\n"));
            for p in 0..70 {
                text.push_str(&format!(
                    "这是第{c}章第{p}段的内容,这一行写长一些保证超过一千字符门槛呀。\n"
                ));
            }
            text.push('\n');
        }
        let path = dir.join(format!("slide-{chapters}.txt"));
        fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn scroll_past_tail_slides_window_forward() {
        let dir = std::env::temp_dir().join("lue_slide_unit");
        fs::create_dir_all(&dir).unwrap();
        let path = make_book(&dir, 25);
        let mut state = ReaderState::open(&path, 60, 18).unwrap();
        let base0 = state.base;
        for _ in 0..100 {
            state.scroll_down(50).unwrap();
            if state.base != base0 {
                break;
            }
        }
        assert!(state.base > base0, "repeated scroll_down must advance the window");
        assert!(state.scroll <= state.max_scroll());
    }

    #[test]
    fn page_down_advances_center() {
        let dir = std::env::temp_dir().join("lue_pagedown_unit");
        fs::create_dir_all(&dir).unwrap();
        let path = make_book(&dir, 25);
        let mut state = ReaderState::open(&path, 60, 18).unwrap();
        let center0 = state.center;
        for _ in 0..80 {
            state.page_down().unwrap();
            if state.center != center0 {
                break;
            }
        }
        assert!(state.center > center0);
    }

    #[test]
    fn page_info_is_sane() {
        let dir = std::env::temp_dir().join("lue_pageinfo_unit");
        fs::create_dir_all(&dir).unwrap();
        let path = make_book(&dir, 25);
        let state = ReaderState::open(&path, 60, 18).unwrap();
        let (page, pages, _ch) = state.page_info();
        assert!(page >= 1 && page <= pages);
    }

    #[test]
    fn restore_uses_manual_anchor() {
        use crate::store::ProgressRecord;
        let dir = std::env::temp_dir().join("lue_restore_unit");
        fs::create_dir_all(&dir).unwrap();
        let path = make_book(&dir, 25);
        let mut state = ReaderState::open(&path, 60, 18).unwrap();
        // Jump somewhere, capture the anchor, then restore it.
        state.jump_chapter(10).unwrap();
        state.scroll = 5.min(state.max_scroll());
        let rec = ProgressRecord {
            c: 10,
            manual_scroll_anchor: state
                .layout
                .line_position(state.scroll)
                .map(|(c, p, s)| [c, p, s]),
            ..ProgressRecord::default()
        };
        let mut restored = ReaderState::open(&path, 60, 18).unwrap();
        restored.restore(&rec).unwrap();
        assert_eq!(restored.center, 10);
    }
}
