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
mod tts;

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
    about = "lue-rs: fast pure-Rust terminal eBook reader (TXT/EPUB/PDF/DOCX/HTML/MD/RTF)"
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
    /// TTS engine; `edge` reads aloud via Microsoft Edge's online TTS
    /// (played through ffplay)
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
    if cli.tts != "none" && cli.tts != "edge" {
        bail!(
            "--tts {} is not available; the native binary supports --tts edge (or none)",
            cli.tts
        );
    }
    let tts_enabled = cli.tts == "edge";
    let path = match cli.book {
        Some(p) => p,
        None => store::most_recent_path().unwrap_or_else(|| {
            bail_exit("no book given and no reading history found");
        }),
    };
    let mut current = path;
    loop {
        let flow = run_app(&current, cli.mode, &cli.keys, tts_enabled)?;
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
         a          read aloud from here --tts edge (a again stops)\n\
         p          pause / resume read-aloud\n\
         q          quit (progress saved)\n\
         \n\
         arrows, PgUp/PgDn, Home/End and the mouse wheel also work;\n\
         clicking the footer progress bar jumps to that position."
    );
}

/// Sentence queue for read-aloud: every sentence of the centre chapter at
/// or after the current scroll line — or, when the viewport already sits at
/// the chapter end, the last sentence above it.
fn tts_queue_from_position(
    state: &mut ReaderState,
) -> Result<Vec<(usize, usize, usize, String)>> {
    let chapter = state.center;
    let paragraphs = state.book.read_chapter(chapter)?;
    let scroll_line = state.scroll;
    let mut queue = Vec::new();
    for ((c, p, s), line) in &state.layout.lines_sorted_positions {
        if *c != chapter as i64 || *line < scroll_line {
            continue;
        }
        push_sentence(&mut queue, &paragraphs, *c as usize, *p as usize, *s as usize);
    }
    if queue.is_empty() {
        for ((c, p, s), line) in state.layout.lines_sorted_positions.iter().rev() {
            if *c != chapter as i64 || *line > scroll_line {
                continue;
            }
            push_sentence(&mut queue, &paragraphs, *c as usize, *p as usize, *s as usize);
            break;
        }
    }
    Ok(queue)
}

fn push_sentence(
    queue: &mut Vec<(usize, usize, usize, String)>,
    paragraphs: &[String],
    chapter: usize,
    paragraph: usize,
    sentence: usize,
) {
    if let Some(text) = paragraphs
        .get(paragraph)
        .and_then(|par| lue_core::split_sentences_impl(par).into_iter().nth(sentence))
    {
        if !text.trim().is_empty() {
            queue.push((chapter, paragraph, sentence, text));
        }
    }
}

fn run_app(path: &Path, ui_mode: u8, keys_layout: &str, tts_enabled: bool) -> Result<Flow> {
    let keymap = input::load(keys_layout)?;

    enable_raw_mode()?;
    let mut stdout_guard = stdout();
    crossterm::execute!(stdout_guard, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout_guard);
    let mut terminal = Terminal::new(backend)?;

    let flow = event_loop(&mut terminal, path, ui_mode, &keymap, tts_enabled);

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
    tts_enabled: bool,
) -> Result<Flow> {
    let size = terminal.size()?;
    let mut state = ReaderState::open(path, size.width, size.height)?;
    let progress_file = store::progress_path(&state.title);
    state.restore(&store::load_progress(&progress_file))?;
    // Warm the chapter index menu in the background right away, so the
    // first `c` press never pays for a full-book decode.
    state.spawn_title_warmup();

    let (tts, tts_events) = if tts_enabled {
        let voice = std::env::var("LUE_TTS_VOICE")
            .unwrap_or_else(|_| lue_core::DEFAULT_VOICE.to_string());
        let (c, r) = tts::TtsController::spawn(voice);
        (Some(c), Some(r))
    } else {
        (None, None)
    };

    let mut ui_mode = ui_mode;
    let mut overlay = Overlay::None;

    loop {
        state.poll_titles();
        // TTS feedback: follow the sentence being read by scrolling its
        // paragraph into view.
        if let Some(rx) = &tts_events {
            while let Ok(event) = rx.try_recv() {
                if let tts::TtsEvent::Sentence { chapter, paragraph } = event {
                    let line = state
                        .layout
                        .paragraph_line_ranges
                        .iter()
                        .find(|((c, p), _)| *c == chapter as i64 && *p == paragraph as i64)
                        .map(|(_, range)| range.0);
                    if let Some(line) = line {
                        state.scroll = line.min(state.max_scroll());
                    }
                }
            }
        }
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
                                    if let Some(c) = &tts {
                                        c.send(tts::TtsCmd::Stop);
                                    }
                                    store::save_progress(&progress_file, &state.to_record())?;
                                    return Ok(Flow::Quit);
                                }
                                Action::TtsToggleRead => {
                                    if let Some(controller) = &tts {
                                        if controller.is_active() {
                                            controller.send(tts::TtsCmd::Stop);
                                        } else if let Ok(jobs) =
                                            tts_queue_from_position(&mut state)
                                        {
                                            if !jobs.is_empty() {
                                                controller.send(tts::TtsCmd::Start(jobs));
                                            }
                                        }
                                    }
                                }
                                Action::TtsPlayPause => {
                                    if let Some(controller) = &tts {
                                        controller.toggle();
                                    }
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
                                    if let Some(c) = &tts {
                                        c.send(tts::TtsCmd::Stop);
                                    }
                                    state.next_chapter()?;
                                    store::save_progress(
                                        &progress_file,
                                        &state.to_record(),
                                    )?;
                                }
                                Action::PrevChapter => {
                                    if let Some(c) = &tts {
                                        c.send(tts::TtsCmd::Stop);
                                    }
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
                                if let Some(c) = &tts {
                                    c.send(tts::TtsCmd::Stop);
                                }
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

#[cfg(test)]
mod warmup_tests {
    use super::*;
    use std::fs;
    use std::time::{Duration, Instant};

    fn make_book(dir: &std::path::Path, chapters: usize) -> PathBuf {
        let mut text = String::from("书名页\n\n");
        for c in 1..=chapters {
            text.push_str(&format!("第{c}章 后台预热测试章节{c}\n"));
            for p in 0..70 {
                text.push_str(&format!(
                    "这是第{c}章第{p}段的内容,这一行写长一些保证超过一千字符门槛呀。\n"
                ));
            }
            text.push('\n');
        }
        let path = dir.join(format!("warmup-{chapters}.txt"));
        fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn background_warmup_delivers_titles_without_pressing_c() {
        let dir = std::env::temp_dir().join("lue_warmup_unit");
        fs::create_dir_all(&dir).unwrap();
        let path = make_book(&dir, 20);
        let mut state = ReaderState::open(&path, 60, 18).unwrap();
        assert!(state.title_count().is_none());
        state.spawn_title_warmup();
        let deadline = Instant::now() + Duration::from_secs(5);
        while state.title_count().is_none() && Instant::now() < deadline {
            state.poll_titles();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(state.title_count(), Some(21));
        // Warm-up already delivered: pressing c must not recompute.
        let titles = state.chapter_titles().unwrap();
        assert_eq!(titles.len(), 21);
        assert!(titles[1].contains("后台预热测试章节1"));
    }

    #[test]
    fn chapter_titles_falls_back_to_fast_sync_pass() {
        let dir = std::env::temp_dir().join("lue_titles_sync_unit");
        fs::create_dir_all(&dir).unwrap();
        let path = make_book(&dir, 15);
        let mut state = ReaderState::open(&path, 60, 18).unwrap();
        let started = Instant::now();
        let titles = state.chapter_titles().unwrap();
        assert_eq!(titles.len(), 16); // preface + 15 chapters
        assert!(titles[0] == "书名页");
        assert!(started.elapsed() < Duration::from_secs(5));
        // Second call is the memoized list.
        assert_eq!(state.chapter_titles().unwrap(), titles);
    }
}

#[cfg(test)]
mod header_title_tests {
    use super::*;
    use std::fs;

    #[test]
    fn header_title_follows_viewport_chapter() {
        let dir = std::env::temp_dir().join("lue_header_unit");
        fs::create_dir_all(&dir).unwrap();
        let mut text = String::from("书名页\n\n");
        for c in 1..=25 {
            text.push_str(&format!("第{c}章 测试章节标题{c}\n"));
            for p in 0..70 {
                text.push_str(&format!(
                    "这是第{c}章第{p}段的内容,这一行写长一些保证超过一千字符门槛呀。\n"
                ));
            }
            text.push('\n');
        }
        let path = dir.join("header-book.txt");
        fs::write(&path, text).unwrap();

        let mut state = ReaderState::open(&path, 60, 18).unwrap();
        state.jump_chapter(10).unwrap();
        // the preface occupies index 0, so chapter index c ↔ 第c章
        assert!(
            state.current_chapter_title().starts_with("第10章"),
            "got '{}'",
            state.current_chapter_title()
        );

        // page down until the viewport reaches a different chapter in the
        // window; the header title must follow the viewport chapter
        let mut guard = 0;
        while state.viewport_chapter() == 10 && guard < 500 {
            state.page_down().unwrap();
            guard += 1;
        }
        assert_ne!(state.viewport_chapter(), 10, "paging must leave chapter 10");
        let expect = format!("第{}章", state.viewport_chapter());
        assert!(
            state.current_chapter_title().starts_with(&expect),
            "header '{}' must match viewport chapter {}",
            state.current_chapter_title(),
            expect
        );
    }
}
