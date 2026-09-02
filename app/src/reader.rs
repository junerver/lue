//! Reader state: chapter window, viewport scrolling, navigation and the
//! progress record bridge. The window strategy mirrors the Python reader
//! (`reader.py`): only the current chapter ±(1 before, 2 after) is decoded
//! and laid out; scrolling happens inside the laid-out window; crossing a
//! chapter boundary re-centres the window on the new chapter.

use anyhow::Result;
use lue_core::{DocumentLayout, TxtBook};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};

use crate::{library, store::ProgressRecord};

const WINDOW_BEFORE: usize = 1;
const WINDOW_AFTER: usize = 2;

pub struct ReaderState {
    pub book: TxtBook,
    pub path: PathBuf,
    pub title: String,
    pub total_chapters: usize,
    /// Window centre chapter.
    pub center: usize,
    /// First chapter of the window.
    pub base: usize,
    /// One past the last chapter of the window.
    pub end: usize,
    pub layout: DocumentLayout,
    /// Top visible line inside the window layout.
    pub scroll: usize,
    pub body_width: u16,
    pub body_height: u16,
    titles: Option<Vec<String>>,
    /// Background warm-up result channel (`spawn_title_warmup`).
    title_rx: Option<Receiver<Vec<String>>>,
}

impl ReaderState {
    pub fn open(path: &Path, width: u16, height: u16) -> Result<Self> {
        let opened = library::open(path)?;
        let total_chapters = opened.book.chapter_count();
        let mut state = Self {
            book: opened.book,
            path: opened.path,
            title: opened.title,
            total_chapters,
            center: 0,
            base: 0,
            end: 0,
            layout: DocumentLayout::default(),
            scroll: 0,
            body_width: width,
            body_height: height,
            titles: opened.titles,
            title_rx: None,
        };
        state.reload_window(0)?;
        Ok(state)
    }

    fn reload_window(&mut self, center: usize) -> Result<()> {
        let center = center.min(self.total_chapters.saturating_sub(1));
        self.layout = self
            .book
            .layout_window(center, WINDOW_BEFORE, WINDOW_AFTER, self.body_width as usize)?;
        self.base = center.saturating_sub(WINDOW_BEFORE);
        self.end = (center + WINDOW_AFTER + 1).min(self.total_chapters);
        self.center = center;
        self.clamp_scroll();
        Ok(())
    }

    pub fn clamp_scroll(&mut self) {
        let max = self.max_scroll();
        self.scroll = self.scroll.min(max);
    }

    pub fn max_scroll(&self) -> usize {
        self.layout
            .lines
            .len()
            .saturating_sub(self.body_height as usize)
    }

    /// Restore a progress record: centre the window on the stored chapter,
    /// then place the anchor sentence (fall back to the stored scroll).
    pub fn restore(&mut self, record: &ProgressRecord) -> Result<()> {
        let c = record.c.min(self.total_chapters.saturating_sub(1));
        self.reload_window(c)?;
        if let Some(anchor) = record.manual_scroll_anchor {
            let key = (anchor[0], anchor[1], anchor[2]);
            if let Some(line) = self
                .layout
                .position_to_line
                .iter()
                .find(|(pos, _)| *pos == key)
                .map(|(_, line)| *line)
            {
                self.scroll = line;
                self.clamp_scroll();
                return Ok(());
            }
        }
        self.scroll = (record.scroll_offset as usize).min(self.max_scroll());
        Ok(())
    }

    pub fn to_record(&self) -> ProgressRecord {
        let anchor = self
            .layout
            .line_position(self.scroll)
            .map(|(c, p, s)| [c, p, s]);
        ProgressRecord {
            c: self.center,
            p: 0,
            s: 0,
            scroll_offset: self.scroll as f64,
            completion_percentage: self.percentage(),
            manual_scroll_anchor: anchor,
            original_file_path: Some(self.path.to_string_lossy().into_owned()),
            ..ProgressRecord::default()
        }
    }

    pub fn percentage(&self) -> f64 {
        if self.total_chapters == 0 {
            return 0.0;
        }
        let total_lines = self.layout.lines.len();
        let frac = if total_lines > 0 {
            self.scroll as f64 / total_lines as f64
        } else {
            0.0
        };
        let chapter_f = self.base as f64 + frac * (self.end - self.base) as f64;
        ((chapter_f / self.total_chapters as f64) * 100.0).clamp(0.0, 100.0)
    }

    /// Page number of the current viewport (1-based) and total pages.
    /// A page is a screenful of body lines; the window's line count is the
    /// total when it's the only "page".
    pub fn page_info(&self) -> (usize, usize, usize) {
        let per = (self.body_height as usize).max(1);
        let total_lines = self.layout.lines.len();
        let total_pages = if total_lines == 0 {
            1
        } else {
            total_lines.div_ceil(per)
        };
        let current = (self.scroll / per).min(total_pages.saturating_sub(1)) + 1;
        (current, total_pages, self.viewport_chapter() + 1)
    }

    /// The chapter the reader is actually looking at: the chapter owning the
    /// line at the top of the viewport (which may differ from the window
    /// center after paging into a neighbouring chapter).
    pub fn viewport_chapter(&self) -> usize {
        let chapter = self
            .layout
            .line_position(self.scroll)
            .map(|(c, _, _)| c.max(0) as usize)
            .unwrap_or(self.center);
        chapter.min(self.total_chapters.saturating_sub(1))
    }

    pub fn current_chapter_title(&self) -> String {
        let line = self.first_line_of(self.viewport_chapter());
        self.layout
            .lines
            .get(line)
            .cloned()
            .unwrap_or_default()
    }

    fn first_line_of(&self, chapter: usize) -> usize {
        self.layout
            .line_to_paragraph
            .iter()
            .position(|owner| matches!(owner, Some((c, _)) if *c == chapter as i64))
            .unwrap_or(0)
    }

    pub fn jump_chapter(&mut self, chapter: usize) -> Result<()> {
        let chapter = chapter.min(self.total_chapters.saturating_sub(1));
        self.reload_window(chapter)?;
        self.scroll = self.first_line_of(chapter);
        self.clamp_scroll();
        Ok(())
    }

    pub fn next_chapter(&mut self) -> Result<()> {
        if self.center + 1 < self.total_chapters {
            self.jump_chapter(self.center + 1)?;
        } else {
            self.scroll = self.max_scroll();
        }
        Ok(())
    }

    pub fn prev_chapter(&mut self) -> Result<()> {
        // At a chapter top, `z` walks to the previous chapter; further up in
        // a scrolled chapter just scrolls to its top first.
        if self.scroll > self.first_line_of(self.center) {
            self.scroll = self.first_line_of(self.center);
        } else if self.center > 0 {
            self.jump_chapter(self.center - 1)?;
        }
        Ok(())
    }

    pub fn goto_top(&mut self) -> Result<()> {
        self.jump_chapter(0)
    }

    pub fn goto_bottom(&mut self) {
        self.scroll = self.max_scroll();
    }

    pub fn scroll_down(&mut self, n: usize) -> Result<()> {
        let max = self.max_scroll();
        if self.scroll < max {
            self.scroll = (self.scroll + n).min(max);
            return Ok(());
        }
        // Already at the window tail with a next chapter: slide forward one
        // chapter and land on the content that was under the old top line
        // (mirrors Python's _slide_window(1) without the buggy offset math).
        if self.center + 1 < self.total_chapters {
            let anchor = self.anchor_at(self.scroll);
            let next = self.center + 1;
            self.reload_window(next)?;
            self.scroll = self
                .layout
                .line_of(anchor.1)
                .unwrap_or(self.first_line_of(next))
                .min(self.max_scroll());
            self.clamp_scroll();
        }
        Ok(())
    }

    pub fn scroll_up(&mut self, n: usize) -> Result<()> {
        let target = self.scroll.saturating_sub(n);
        if self.scroll > 0 {
            self.scroll = target;
            return Ok(());
        }
        // At the window top with an earlier chapter: slide back and anchor
        // to the content that was under the old top line.
        if self.center > 0 {
            let anchor = self.anchor_at(self.scroll);
            let prev = self.center - 1;
            self.reload_window(prev)?;
            self.scroll = self
                .layout
                .line_of(anchor.1)
                .or_else(|| {
                    self.layout
                        .line_to_paragraph
                        .iter()
                        .rposition(|o| matches!(o, Some((c, _)) if *c == prev as i64))
                })
                .unwrap_or(0)
                .min(self.max_scroll());
            self.clamp_scroll();
        }
        Ok(())
    }

    pub fn page_down(&mut self) -> Result<()> {
        // One screenful; when the tail of the window is near, the next
        // page continues at the next chapter head instead of running off.
        self.scroll_down((self.body_height as usize).saturating_sub(1))
    }

    pub fn page_up(&mut self) -> Result<()> {
        self.scroll_up((self.body_height as usize).saturating_sub(1))
    }

    /// Position under a window line + that line itself: `(line, (c,p,s))`.
    fn anchor_at(&self, line: usize) -> (usize, (i64, i64, i64)) {
        let line = line.min(self.layout.lines.len().saturating_sub(1));
        let pos = self
            .layout
            .line_position(line)
            .unwrap_or((self.center as i64, 0, 0));
        (line, pos)
    }

    /// Start line of the paragraph after the one under `line` (used by the
    /// paragraph-wise h/l navigation from the key layouts).
    pub fn next_paragraph_start(&self, line: usize) -> usize {
        let Some(Some((cur_c, cur_p))) = self.layout.line_to_paragraph.get(line).copied()
        else {
            return self.max_scroll();
        };
        self.layout
            .line_to_paragraph
            .iter()
            .position(|owner| matches!(owner, Some((c, p)) if *c == cur_c && *p == cur_p + 1))
            .or_else(|| {
                // next chapter head, if inside the window
                self.layout
                    .line_to_paragraph
                    .iter()
                    .position(|owner| matches!(owner, Some((c, p)) if *c == cur_c + 1 && *p == 0))
            })
            .unwrap_or(self.max_scroll())
    }

    pub fn prev_paragraph_start(&self, line: usize) -> usize {
        if self.layout.line_to_paragraph.is_empty() {
            return 0;
        }
        let para_start = |mut i: usize| -> usize {
            while i > 0
                && self.layout.line_to_paragraph[i].is_some()
                && self.layout.line_to_paragraph[i - 1] == self.layout.line_to_paragraph[i]
            {
                i -= 1;
            }
            i
        };
        let line = line.min(self.layout.line_to_paragraph.len() - 1);
        let start = para_start(line);
        if start < line {
            return start;
        }
        // already at a paragraph start: step to the previous paragraph
        if start == 0 {
            return 0;
        }
        para_start(start - 1)
    }

    /// Handle a viewport resize: re-wrap on width change, keeping the content
    /// under the top visible line anchored. A re-wrap renumbers every line,
    /// so the raw scroll value must never be reused as the new offset.
    pub fn set_size(&mut self, width: u16, height: u16) -> Result<()> {
        if width == self.body_width && height == self.body_height {
            return Ok(());
        }
        if width != self.body_width {
            let anchor = self.anchor_at(self.scroll).1;
            self.body_width = width;
            self.body_height = height;
            self.reload_window(self.center)?;
            self.scroll = self
                .layout
                .line_of(anchor)
                .unwrap_or(self.first_line_of(self.viewport_chapter()))
                .min(self.max_scroll());
        } else {
            self.body_height = height;
            self.clamp_scroll();
        }
        Ok(())
    }

    /// Chapter titles for the index menu. Filled from the disk cache, the
    /// background warm-up, or — if `c` is pressed before either lands — a
    /// synchronous head-only pass (small preads, no full-book decode).
    pub fn chapter_titles(&mut self) -> Result<Vec<String>> {
        if let Some(titles) = &self.titles {
            return Ok(titles.clone());
        }
        let titles = self.book.collect_titles()?;
        self.titles = Some(titles.clone());
        Ok(titles)
    }

    /// Start warming the chapter titles in the background right after the
    /// book opens, so pressing `c` never waits on a whole-book decode. The
    /// warmed titles are also merged into the on-disk index cache, making
    /// every later open of this book skip the warm-up.
    pub fn spawn_title_warmup(&mut self) {
        if self.titles.is_some() || self.title_rx.is_some() || self.total_chapters == 0 {
            return;
        }
        let mut book = self.book.clone();
        let cache_path = self.path.clone();
        let (tx, rx) = channel();
        self.title_rx = Some(rx);
        std::thread::spawn(move || {
            if let Ok(titles) = book.collect_titles() {
                library::cache_titles(&cache_path, &titles);
                let _ = tx.send(titles);
            }
        });
    }

    /// Number of warmed titles once available (tests / diagnostics).
    #[cfg(test)]
    pub fn title_count(&self) -> Option<usize> {
        self.titles.as_ref().map(|t| t.len())
    }

    /// Non-blocking pickup of the background warm-up result; call each loop
    /// iteration.
    pub fn poll_titles(&mut self) {
        if self.titles.is_some() {
            self.title_rx = None;
            return;
        }
        let received = self.title_rx.as_ref().and_then(|rx| rx.try_recv().ok());
        if let Some(titles) = received {
            self.titles = Some(titles);
            self.title_rx = None;
        }
    }
}

#[cfg(test)]
mod slide_anchor_tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    /// Chinese-profile book whose paragraphs are single lines at width 100,
    /// so every paragraph is followed by a blank line and window tails often
    /// have a blank top line — the exact shape that used to break the slide.
    fn make_book(dir: &Path, name: &str, chapters: usize, paras: usize) -> PathBuf {
        let mut text = String::from("书名页\n\n");
        for c in 1..=chapters {
            text.push_str(&format!("第{c}章 测试章节标题{c}\n"));
            for p in 0..paras {
                text.push_str(&format!(
                    "这是第{c}章第{p}段的内容,这一行写长一些保证不会折行呀。\n"
                ));
            }
            text.push('\n');
        }
        let path = dir.join(name);
        fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn page_down_at_blank_top_keeps_reading_position() {
        let dir = std::env::temp_dir().join("lue_slide_anchor");
        fs::create_dir_all(&dir).unwrap();
        let path = make_book(&dir, "slide-anchor.txt", 14, 40);
        let mut hit_blank_top = false;
        for body_height in 4..=34u16 {
            let mut state = ReaderState::open(&path, 100, body_height).unwrap();
            // walk to the first window tail that still has a next chapter
            for _ in 0..300 {
                if state.scroll == state.max_scroll() && state.center + 1 < state.total_chapters {
                    break;
                }
                state.page_down().unwrap();
            }
            if state.scroll != state.max_scroll() || state.center + 1 >= state.total_chapters {
                continue;
            }
            let top = state.scroll.min(state.layout.lines.len().saturating_sub(1));
            if !state.layout.lines[top].trim().is_empty() {
                continue;
            }
            hit_blank_top = true;
            let before = state.layout.line_position(top);
            state.page_down().unwrap();
            let after = state.layout.line_position(state.scroll);
            assert_eq!(
                after, before,
                "body_height {body_height}: paging past the window tail must anchor to the \
                 content under the old top line, not jump to the window centre chapter"
            );
        }
        assert!(hit_blank_top, "no blank-top tail scenario exercised");
    }

    #[test]
    fn width_resize_keeps_top_content_anchored() {
        let dir = std::env::temp_dir().join("lue_slide_anchor");
        fs::create_dir_all(&dir).unwrap();
        // Long paragraphs wrap differently at the two widths, so a re-wrap
        // renumbers every line — the raw scroll value must not be reused.
        let mut text = String::from("书名页\n\n");
        for c in 1..=10 {
            text.push_str(&format!("第{c}章 测试章节标题{c}\n"));
            for p in 0..30 {
                let body = format!(
                    "这是第{c}章第{p}段的内容,这一行写长一些保证不会折行呀。"
                );
                for _ in 0..3 {
                    text.push_str(&body);
                }
                text.push('\n');
            }
            text.push('\n');
        }
        let path = dir.join("resize-anchor.txt");
        fs::write(&path, text).unwrap();

        let mut state = ReaderState::open(&path, 100, 20).unwrap();
        for _ in 0..60 {
            state.scroll_down(7).unwrap();
        }
        let before = state
            .layout
            .line_position(state.scroll.min(state.layout.lines.len().saturating_sub(1)));
        state.set_size(60, 20).unwrap();
        let after = state.layout.line_position(state.scroll);
        assert_eq!(
            after, before,
            "re-wrapping renumbers lines; the viewport must follow the anchored content"
        );
    }
}
