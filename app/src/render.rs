//! Ratatui rendering: header, paged body, footer progress bar and the two
//! overlays (chapter index, recent books). The body consumes the pre-wrapped
//! window lines verbatim (no re-wrapping), so a frame costs O(viewport).

use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::reader::ReaderState;
use anyhow::Result;

pub enum Overlay {
    None,
    Chapters {
        titles: Vec<String>,
        filter: String,
        selected: usize,
    },
    Recent {
        items: Vec<(String, String)>,
        selected: usize,
    },
}

/// Render one frame; also syncs the reader's viewport size to the body area
/// (a resize invalidates the window layout only when the width changes).
pub fn draw(
    frame: &mut Frame,
    state: &mut ReaderState,
    ui_mode: u8,
    overlay: &mut Overlay,
) -> Result<()> {
    let full = frame.area();
    let chrome = match ui_mode {
        0 => 0,
        1 => 1,
        _ => 2, // modes 2 and 3 both draw header + footer
    };

    let mut rows = Layout::vertical([
        Constraint::Length(if chrome >= 1 { 1 } else { 0 }),
        Constraint::Min(1),
        Constraint::Length(if chrome >= 2 { 1 } else { 0 }),
    ]).split(full);

    if chrome == 0 {
        // no header: the middle chunk must own everything
        rows = Layout::vertical([Constraint::Min(1)]).split(full);
    }

    let body = rows[if chrome == 0 { 0 } else { 1 }];
    state.set_size(body.width, body.height)?;

    if chrome >= 1 {
        draw_header(frame, rows[0], state);
    }
    draw_body(frame, body, state);
    if chrome >= 2 {
        draw_footer(frame, rows[2], state);
    }
    match overlay {
        Overlay::None => {}
        Overlay::Chapters { titles, filter, selected } => {
            draw_chapters(frame, full, titles, filter, *selected)
        }
        Overlay::Recent { items, selected } => draw_recent(frame, full, items, *selected),
    }
    Ok(())
}

fn draw_header(frame: &mut Frame, area: Rect, state: &ReaderState) {
    let left = Span::styled(
        state.title.clone(),
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
    );
    let middle = Span::styled(
        state.current_chapter_title(),
        Style::default().fg(Color::DarkGray),
    );
    let (page, pages, chapter_no) = state.page_info();
    let right = Span::styled(
        format!("{page}/{pages} 章{chapter_no} {:>5.1}%", state.percentage()),
        Style::default().fg(Color::Yellow),
    );
    let line = Line::from(vec![left, Span::raw("  "), middle, Span::raw(" "), right]);
    frame.render_widget(Paragraph::new(line), area);
}

fn draw_body(frame: &mut Frame, area: Rect, state: &ReaderState) {
    let start = state.scroll;
    let end = (start + area.height as usize).min(state.layout.lines.len());
    let lines: Vec<Line> = state.layout.lines[start..end]
        .iter()
        .map(|l| Line::from(l.as_str()))
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
}

fn draw_footer(frame: &mut Frame, area: Rect, state: &ReaderState) {
    let pct = state.percentage();
    let width = area.width as usize;
    let filled = ((pct / 100.0) * width as f64).round() as usize;
    let filled = filled.min(width);
    let bar = Line::from(vec![
        Span::styled("█".repeat(filled), Style::default().fg(Color::Green)),
        Span::styled("░".repeat(width.saturating_sub(filled)), Style::default().fg(Color::DarkGray)),
    ]);
    frame.render_widget(Paragraph::new(bar), area);
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup = Layout::vertical([
        Constraint::Percentage((100 - percent_y) / 2),
        Constraint::Percentage(percent_y),
        Constraint::Percentage((100 - percent_y) / 2 + (100 - percent_y) % 2),
    ])
    .split(area);
    Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2 + (100 - percent_x) % 2),
    ])
    .split(popup[1])[1]
}

/// Opaque overlay styling: the chapter/recent panels paint an opaque
/// background over the book text so the popup reads as a separate window.
const PANEL_BG: Color = Color::Black;
const PANEL_BORDER: Color = Color::Cyan;
const PANEL_TITLE: Color = Color::Cyan;
const PANEL_ITEM: Color = Color::White;
const PANEL_GUIDE: Color = Color::DarkGray;
const PANEL_HIGHLIGHT_BG: Color = Color::LightBlue;
const PANEL_HIGHLIGHT_FG: Color = Color::White;

/// Fill `popup` with an opaque panel background, hiding the text behind it.
fn draw_panel(frame: &mut Frame, popup: Rect) {
    frame.render_widget(
        Block::default().style(Style::default().bg(PANEL_BG)),
        popup,
    );
}

fn draw_chapters(frame: &mut Frame, area: Rect, titles: &[String], filter: &str, selected: usize) {
    // Full-screen cover: any body text left visible around the panel reads
    // as bleed-through (user feedback), so the panel takes the whole screen.
    let popup = area;
    frame.render_widget(Clear, popup);
    draw_panel(frame, popup);

    let needle = filter.to_lowercase();
    let visible: Vec<usize> = titles
        .iter()
        .enumerate()
        .filter(|(_, t)| needle.is_empty() || t.to_lowercase().contains(&needle))
        .map(|(i, _)| i)
        .collect();

    let items: Vec<ListItem> = visible
        .iter()
        .enumerate()
        .map(|(idx, &i)| {
            ListItem::new(Line::from(vec![
                Span::styled(format!("{idx:>3} "), Style::default().fg(PANEL_GUIDE)),
                Span::styled(titles[i].as_str(), Style::default().fg(PANEL_ITEM)),
            ]))
        })
        .collect();
    let title_style = Style::default()
        .fg(PANEL_TITLE)
        .add_modifier(Modifier::BOLD);
    let list = List::new(items)
        .style(Style::default().bg(PANEL_BG))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(
                    Style::default()
                        .fg(PANEL_BORDER)
                        .add_modifier(Modifier::BOLD),
                )
                .title(Span::styled(
                    format!(" 章节 ({}) — 输入过滤, Enter 跳转: {filter} ", visible.len()),
                    title_style,
                )),
        )
        .highlight_style(
            Style::default()
                .bg(PANEL_HIGHLIGHT_BG)
                .fg(PANEL_HIGHLIGHT_FG)
                .add_modifier(Modifier::BOLD),
        );
    let mut list_state = ListState::default();
    // keep the selection on the currently highlighted visible row
    let selected_visible = visible.iter().position(|&i| i == selected).unwrap_or(0);
    list_state.select(Some(selected_visible.min(visible.len().saturating_sub(1))));
    frame.render_stateful_widget(list, popup, &mut list_state);
}

fn draw_recent(frame: &mut Frame, area: Rect, items: &[(String, String)], selected: usize) {
    let popup = centered_rect(76, 60, area);
    frame.render_widget(Clear, popup);
    draw_panel(frame, popup);
    let list_items: Vec<ListItem> = items
        .iter()
        .map(|(title, path)| {
            ListItem::new(Line::from(vec![
                Span::styled(title.as_str(), Style::default().fg(PANEL_ITEM)),
                Span::raw("  "),
                Span::styled(path.as_str(), Style::default().fg(PANEL_GUIDE)),
            ]))
        })
        .collect();
    let list = List::new(list_items)
        .style(Style::default().bg(PANEL_BG))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(
                    Style::default()
                        .fg(PANEL_BORDER)
                        .add_modifier(Modifier::BOLD),
                )
                .title(Span::styled(
                    " 最近阅读 (Enter 打开) ",
                    Style::default()
                        .fg(PANEL_TITLE)
                        .add_modifier(Modifier::BOLD),
                ))
                .title_alignment(Alignment::Left),
        )
        .highlight_style(
            Style::default()
                .bg(PANEL_HIGHLIGHT_BG)
                .fg(PANEL_HIGHLIGHT_FG)
                .add_modifier(Modifier::BOLD),
        );
    let mut list_state = ListState::default();
    list_state.select(Some(selected.min(items.len().saturating_sub(1))));
    frame.render_stateful_widget(list, popup, &mut list_state);
}

/// Map a mouse click on the footer progress bar (last terminal row) to a
/// target percentage (0.0-100.0); clicks elsewhere return None.
pub const MOUSE_SCROLL_LINES: usize = 3;

pub fn progress_bar_click(x: u16, y: u16, area: Rect) -> Option<f64> {
    let footer_y = area.bottom().saturating_sub(1);
    if y != footer_y || area.width == 0 {
        return None;
    }
    Some((x as f64 / area.width as f64) * 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::ReaderState;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
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
        let path = dir.join("render-test-book.txt");
        fs::write(&path, text).unwrap();
        path
    }

    fn visible_chars(terminal: &Terminal<TestBackend>) -> usize {
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .filter(|cell| !cell.symbol().trim().is_empty())
            .count()
    }

    #[test]
    fn cycling_all_ui_modes_keeps_content_visible() {
        // unique dir per test: two tests rewriting one book file race the
        // fingerprint check in ReaderState::open (flaky in parallel runs)
        let dir = std::env::temp_dir().join("lue_render_tests").join("cycling");
        fs::create_dir_all(&dir).unwrap();
        let path = make_book(&dir, 30);
        let mut state = ReaderState::open(&path, 78, 22).unwrap();
        let mut overlay = Overlay::None;

        // what the user does: launch in mode 2, then tap v through 3, 0, 1, 2
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        for mode in [2u8, 3, 0, 1, 2] {
            for _frame in 0..2 {
                terminal
                    .draw(|f| draw(f, &mut state, mode, &mut overlay).expect("draw"))
                    .unwrap();
            }
            let visible = visible_chars(&terminal);
            assert!(
                visible > 20,
                "mode {mode}: blank screen ({visible} visible chars)"
            );
        }
    }

    #[test]
    fn overlays_render_on_every_mode() {
        let dir = std::env::temp_dir().join("lue_render_tests").join("overlays");
        fs::create_dir_all(&dir).unwrap();
        let path = make_book(&dir, 30);
        let mut state = ReaderState::open(&path, 78, 22).unwrap();
        let titles = state.chapter_titles().unwrap();

        for mode in 0..=3u8 {
            let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
            let mut chapters = Overlay::Chapters {
                titles: titles.clone(),
                filter: String::new(),
                selected: 0,
            };
            terminal
                .draw(|f| draw(f, &mut state, mode, &mut chapters).expect("draw"))
                .unwrap();
            assert!(visible_chars(&terminal) > 20, "chapters overlay, mode {mode}");

            let mut recent = Overlay::Recent {
                items: vec![("book-a".into(), "/tmp/a.txt".into())],
                selected: 0,
            };
            terminal
                .draw(|f| draw(f, &mut state, mode, &mut recent).expect("draw"))
                .unwrap();
            assert!(visible_chars(&terminal) > 20, "recent overlay, mode {mode}");
        }
    }

    #[test]
    fn chapter_overlay_is_opaque_panel() {
        // The popup must paint an opaque background so the book text behind
        // it is hidden — the reader should visually separate panel from body.
        let dir = std::env::temp_dir().join("lue_render_tests").join("overlay_panel");
        fs::create_dir_all(&dir).unwrap();
        let path = make_book(&dir, 30);
        let mut state = ReaderState::open(&path, 78, 22).unwrap();
        let titles = state.chapter_titles().unwrap();

        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let mut overlay = Overlay::Chapters {
            titles: titles.clone(),
            filter: String::new(),
            selected: 0,
        };
        terminal
            .draw(|f| draw(f, &mut state, 2, &mut overlay).expect("draw"))
            .unwrap();

        // The panel covers the whole screen now; sample an interior band and
        // assert it is opaque panel background (body text fully hidden).
        let buffer = terminal.backend().buffer();
        let sample_y = 8;
        let mut opaque = 0;
        let mut body_leak = 0;
        for x in 13..68 {
            if let Some(cell) = buffer.cell((x, sample_y)) {
                if cell.style().bg == Some(PANEL_BG) {
                    opaque += 1;
                } else if !cell.symbol().trim().is_empty() && cell.style().bg != Some(PANEL_BG) {
                    body_leak += 1;
                }
            }
        }
        // most interior cells must be panel background (non-title, non-item)
        assert!(
            opaque >= 40,
            "panel interior expected to be opaque: opaque={opaque} leak={body_leak}"
        );
        assert_eq!(body_leak, 0, "book text leaked through the chapter panel");
    }

    #[test]
    fn tiny_terminal_does_not_panic() {
        let dir = std::env::temp_dir().join("lue_render_tests").join("tiny");
        fs::create_dir_all(&dir).unwrap();
        let path = make_book(&dir, 30);
        let mut state = ReaderState::open(&path, 78, 22).unwrap();
        let mut overlay = Overlay::None;
        for mode in 0..=3u8 {
            for (w, h) in [(10u16, 3u16), (4, 1), (80, 1), (80, 2), (1, 5)] {
                let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
                let result = terminal.draw(|f| {
                    draw(f, &mut state, mode, &mut overlay).expect("draw failed")
                });
                if let Err(e) = result {
                    panic!("mode {mode} on {w}x{h} failed: {e}");
                }
            }
        }
    }
}
