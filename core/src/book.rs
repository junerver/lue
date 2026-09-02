//! TXT random-access index + lazily decoded chapter book handle
//! (legado TextFile.kt style): scan the file once, detect the encoding,
//! pick the TOC rule and record the byte range of every chapter so opening
//! a book never needs to decode/parse the whole text.

use crate::error::LueError;
use crate::layout::{layout_document, DocumentLayout};
use crate::text::{analyze_cjk_impl, clean_visual_text_impl, py_trim};
use crate::toc::{collect_title_lines_impl, pick_toc_rule_impl};
use std::collections::VecDeque;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::time::UNIX_EPOCH;

/// Result of a successful TXT index build: chapter byte ranges into the raw
/// file plus the encoding needed to decode them.
#[derive(Debug, Clone)]
pub struct TxtIndex {
    pub encoding: String,
    pub intro_first: bool,
    pub starts: Vec<u64>,
    pub ends: Vec<u64>,
}

impl TxtIndex {
    pub fn is_eligible(&self) -> bool {
        !self.encoding.is_empty() && !self.starts.is_empty()
    }
}

/// Build a TXT index. Returns `Ok(None)` when the book does not match the
/// lazy-eligible profile (non-Chinese layout, no title rule, single-line
/// wall of text…) and `Err` on IO failure. Callers fall back to a full
/// in-memory parse on `Ok(None)`.
pub fn build_txt_index(path: &str) -> Result<Option<TxtIndex>, LueError> {
    let bytes = fs::read(path)?;

    // Encoding probe, same cascade as _read_text_with_encoding_detect:
    // BOM -> utf-8; else strict utf-8; else strict gb18030; else latin-1.
    let (encoding, content) = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        match std::str::from_utf8(&bytes[3..]) {
            Ok(s) => ("utf-8-sig".to_string(), s.to_string()),
            Err(_) => decode_gb18030_or_latin1(&bytes),
        }
    } else if std::str::from_utf8(&bytes).is_ok() {
        ("utf-8".to_string(), String::from_utf8_lossy(&bytes).into_owned())
    } else {
        decode_gb18030_or_latin1(&bytes)
    };

    let content = content.replace("\r\n", "\n").replace('\r', "\n");
    let raw_lines: Vec<&str> = content.split('\n').collect();
    if !raw_lines.iter().any(|l| !l.trim().is_empty()) {
        return Ok(None);
    }

    // Line start offsets in RAW file bytes. 0x0A is a safe line delimiter for
    // UTF-8 (continuation bytes are >= 0x80) and GB18030 (trail bytes are
    // 0x40-0xFE), so raw byte scanning matches the decoded line order 1:1.
    let mut line_starts: Vec<u64> = Vec::with_capacity(raw_lines.len());
    line_starts.push(if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) { 3u64 } else { 0u64 });
    for (i, b) in bytes.iter().enumerate() {
        if *b == 0x0A {
            line_starts.push((i + 1) as u64);
        }
    }

    // Paragraph-strategy gate: only the "one line = one paragraph Chinese
    // novel with titled chapters" profile is lazily indexed.
    let (cjk_count, cjk_total) = analyze_cjk_impl(&content);
    if cjk_count == 0 {
        return Ok(None);
    }
    let is_chinese = cjk_count as f64 / raw_lines.len() as f64 > 0.5;
    let cjk_avg_len = cjk_total as f64 / cjk_count as f64;
    let nonempty_lines = raw_lines.iter().filter(|l| !l.trim().is_empty()).count();
    if !is_chinese || cjk_avg_len > 120.0 || nonempty_lines <= 1 {
        return Ok(None);
    }

    let sample_chars: usize = content
        .char_indices()
        .map(|(i, _)| i)
        .find(|&i| i > 500 * 1024)
        .unwrap_or(content.len());
    let rule_idx = match pick_toc_rule_impl(&content[..sample_chars]) {
        Some(r) => r,
        None => return Ok(None),
    };
    let title_lines = collect_title_lines_impl(&content, rule_idx);
    if title_lines.is_empty() {
        return Ok(None);
    }

    // Chapter byte ranges: each title line starts a chapter that runs until
    // the next title line. Content before the first title becomes a preface
    // chapter when non-empty.
    let file_len = bytes.len() as u64;
    let line_start = |t: u64| -> u64 {
        line_starts
            .get(t as usize)
            .cloned()
            .unwrap_or(file_len)
    };
    let mut starts: Vec<u64> = Vec::new();
    let mut ends: Vec<u64> = Vec::new();
    let first_title = title_lines[0];
    let mut intro_first = false;
    if first_title > 0 {
        // preface only counts when it holds at least one non-blank line
        let has_text = raw_lines[..first_title as usize]
            .iter()
            .any(|l| !l.trim().is_empty());
        if has_text {
            starts.push(0);
            ends.push(line_start(first_title));
            intro_first = true;
        }
    }
    for (k, &t) in title_lines.iter().enumerate() {
        let s = line_start(t);
        let e = title_lines
            .get(k + 1)
            .map(|&next| line_start(next))
            .unwrap_or(file_len);
        if e > s {
            starts.push(s);
            ends.push(e);
        }
    }
    if starts.is_empty() {
        return Ok(None);
    }
    Ok(Some(TxtIndex {
        encoding,
        intro_first,
        starts,
        ends,
    }))
}

fn decode_gb18030_or_latin1(bytes: &[u8]) -> (String, String) {
    let (cow, _enc, had_errors) = encoding_rs::GB18030.decode(bytes);
    if !had_errors {
        ("gb18030".to_string(), cow.into_owned())
    } else {
        (
            "latin-1".to_string(),
            bytes.iter().map(|&b| b as char).collect(),
        )
    }
}

/// Chapter chunk size for the no-title fallback (content_parser
/// `_TOC_FALLBACK_CHAPTER_CHARS`): ~10 KiB of decoded text per chapter.
pub const TOC_FALLBACK_CHAPTER_CHARS: usize = 10 * 1024;

/// Fallback index for TXT files without detectable chapter titles: the whole
/// book is split into ~10 KiB chapters on paragraph lines (Python
/// `_split_txt_by_size`) so untitled books still open instead of being
/// rejected. Returns `None` only when the file holds no text at all.
pub fn build_whole_book_index(path: &str) -> Result<Option<TxtIndex>, LueError> {
    let bytes = fs::read(path)?;
    let (encoding, content) = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        (
            "utf-8-sig".to_string(),
            String::from_utf8_lossy(&bytes[3..]).into_owned(),
        )
    } else if std::str::from_utf8(&bytes).is_ok() {
        ("utf-8".to_string(), String::from_utf8_lossy(&bytes).into_owned())
    } else {
        decode_gb18030_or_latin1(&bytes)
    };
    let content = content.replace("\r\n", "\n").replace('\r', "\n");
    if !content.split('\n').any(|l| !l.trim().is_empty()) {
        return Ok(None);
    }

    // Byte offsets of every line start (0x0A is a safe line delimiter for
    // UTF-8 and GB18030, same argument as build_txt_index).
    let bom = bytes.starts_with(&[0xEF, 0xBB, 0xBF]);
    let mut line_starts: Vec<u64> = Vec::with_capacity(content.len() / 24 + 1);
    line_starts.push(if bom { 3 } else { 0 });
    for (i, b) in bytes.iter().enumerate() {
        if *b == 0x0A {
            line_starts.push((i + 1) as u64);
        }
    }
    let file_len = bytes.len() as u64;
    let line_start = |t: u64| -> u64 {
        line_starts.get(t as usize).cloned().unwrap_or(file_len)
    };

    let mut starts: Vec<u64> = Vec::new();
    let mut ends: Vec<u64> = Vec::new();
    let mut chunk_chars: i64 = 0;
    for (line_no, line) in content.split('\n').enumerate() {
        if starts.len() == ends.len() {
            starts.push(line_start(line_no as u64));
        }
        chunk_chars += line.chars().count() as i64;
        if chunk_chars >= TOC_FALLBACK_CHAPTER_CHARS as i64 {
            ends.push(line_start(line_no as u64 + 1));
            chunk_chars = 0;
        }
    }
    if starts.len() > ends.len() {
        ends.push(file_len);
    }
    Ok(Some(TxtIndex {
        encoding,
        intro_first: false,
        starts,
        ends,
    }))
}

/// First line of `complete_lines` that the chapter decode would keep as the
/// display title: the first non-empty cleaned line, provided it is line 0,
/// the preface's opener, or longer than 3 chars (short lines deeper in the
/// chapter are content, not titles).
fn kept_title(complete_lines: &[&str], preface: bool) -> Option<String> {
    for (line_index, line) in complete_lines.iter().enumerate() {
        let cleaned = clean_visual_text_impl(py_trim(line));
        if cleaned.is_empty() {
            continue;
        }
        if preface || line_index == 0 || cleaned.chars().count() > 3 {
            return Some(cleaned);
        }
    }
    None
}

/// Build the index, treating every failure ("not eligible", IO error) as an
/// empty index. This mirrors the Python binding's `(“”, False, [], [])`
/// contract; native callers prefer [`build_txt_index`].
pub fn build_txt_index_lenient(path: &str) -> TxtIndex {
    build_txt_index(path).unwrap_or(Some(TxtIndex {
        encoding: String::new(),
        intro_first: false,
        starts: Vec::new(),
        ends: Vec::new(),
    }))
    .unwrap_or(TxtIndex {
        encoding: String::new(),
        intro_first: false,
        starts: Vec::new(),
        ends: Vec::new(),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceFingerprint {
    pub size: u64,
    pub mtime_ns: u128,
}

pub fn source_fingerprint(path: &str) -> Result<SourceFingerprint, LueError> {
    let metadata = fs::metadata(path)?;
    let mtime_ns = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    Ok(SourceFingerprint {
        size: metadata.len(),
        mtime_ns,
    })
}

/// Random-access TXT book handle: decodes and cleans a chapter only when it
/// is asked for, keeping a small LRU of decoded chapters. Every read
/// re-validates the source fingerprint so a changed file is rejected before
/// stale byte ranges can produce wrong content.
pub struct TxtBook {
    path: String,
    encoding: String,
    starts: Vec<u64>,
    ends: Vec<u64>,
    intro_first: bool,
    fingerprint: SourceFingerprint,
    /// Lazily opened handle; clones start handle-free so a background warm-up
    /// copy never shares (or competes for) the reader's seek position.
    file: Option<File>,
    chapter_cache: VecDeque<(usize, Vec<String>)>,
    cache_capacity: usize,
}

impl Clone for TxtBook {
    fn clone(&self) -> Self {
        Self {
            path: self.path.clone(),
            encoding: self.encoding.clone(),
            starts: self.starts.clone(),
            ends: self.ends.clone(),
            intro_first: self.intro_first,
            fingerprint: self.fingerprint,
            file: None,
            chapter_cache: VecDeque::new(),
            cache_capacity: self.cache_capacity,
        }
    }
}

impl TxtBook {
    /// Validate a (possibly deserialized) index against the file on disk.
    pub fn from_index(
        path: String,
        encoding: String,
        starts: Vec<u64>,
        ends: Vec<u64>,
        intro_first: bool,
    ) -> Result<Self, LueError> {
        if starts.is_empty() || starts.len() != ends.len() {
            return Err(LueError::Invalid("invalid TXT chapter index".into()));
        }
        if starts.windows(2).any(|w| w[0] >= w[1]) || starts.iter().zip(&ends).any(|(s, e)| s >= e) {
            return Err(LueError::Invalid(
                "TXT chapter spans must be ordered and non-empty".into(),
            ));
        }
        let fingerprint = source_fingerprint(&path)?;
        if ends.iter().any(|e| *e > fingerprint.size) {
            return Err(LueError::Invalid("TXT chapter index exceeds source file".into()));
        }
        Ok(Self {
            path,
            encoding,
            starts,
            ends,
            intro_first,
            fingerprint,
            file: None,
            chapter_cache: VecDeque::new(),
            cache_capacity: 64,
        })
    }

    pub fn chapter_count(&self) -> usize {
        self.starts.len()
    }

    pub fn fingerprint(&self) -> SourceFingerprint {
        self.fingerprint
    }

    pub fn intro_first(&self) -> bool {
        self.intro_first
    }

    fn ensure_current(&self) -> Result<(), LueError> {
        let current = source_fingerprint(&self.path)?;
        if current != self.fingerprint {
            return Err(LueError::Invalid(
                "indexed TXT source changed; rebuild the index".into(),
            ));
        }
        Ok(())
    }

    /// Seek + decode one chapter's byte range (span-only read: chapter
    /// random access never re-reads the whole file).
    fn decode_chapter_uncached(&mut self, index: usize) -> Result<Vec<String>, LueError> {
        self.ensure_current()?;
        let start = self.starts[index] as usize;
        let end = self.ends[index] as usize;
        if start > end {
            return Err(LueError::Invalid(
                "indexed TXT chapter span is outside the source file".into(),
            ));
        }
        let span = end - start;
        let file = self.ensure_file()?;
        file.seek(SeekFrom::Start(start as u64))?;
        let mut raw = vec![0u8; span];
        file.read_exact(&mut raw)?;
        let text = match self.encoding.as_str() {
            "utf-8" | "utf-8-sig" => std::str::from_utf8(&raw)
                .map_err(|e| LueError::Decode(e.to_string()))?
                .to_string(),
            "gb18030" => encoding_rs::GB18030.decode(&raw).0.into_owned(),
            "latin-1" => raw.iter().map(|&b| b as char).collect(),
            _ => return Err(LueError::Invalid("unsupported TXT encoding".into())),
        };
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        let mut result = Vec::new();
        let preface = self.intro_first && index == 0;
        for (line_index, line) in normalized.split('\n').enumerate() {
            let cleaned = clean_visual_text_impl(py_trim(line));
            if cleaned.is_empty() {
                continue;
            }
            if preface || line_index == 0 || cleaned.chars().count() > 3 {
                result.push(cleaned);
            }
        }
        Ok(result)
    }

    pub fn read_chapter(&mut self, index: usize) -> Result<Vec<String>, LueError> {
        if index >= self.starts.len() {
            return Err(LueError::Invalid(format!("chapter index out of range: {index}")));
        }
        if let Some(pos) = self.chapter_cache.iter().position(|(i, _)| *i == index) {
            let entry = self.chapter_cache.remove(pos).unwrap();
            let result = entry.1.clone();
            self.chapter_cache.push_back(entry);
            return Ok(result);
        }
        let result = self.decode_chapter_uncached(index)?;
        self.chapter_cache.push_back((index, result.clone()));
        while self.chapter_cache.len() > self.cache_capacity {
            self.chapter_cache.pop_front();
        }
        Ok(result)
    }

    /// First non-empty line of a chapter is its display title (mirrors how
    /// the Python reader names chapters from parsed content).
    pub fn chapter_title(&mut self, index: usize) -> Result<String, LueError> {
        let chapter = self.read_chapter(index)?;
        Ok(chapter
            .first()
            .cloned()
            .unwrap_or_else(|| format!("Chapter {}", index + 1)))
    }

    /// Every chapter title in one pass: per chapter only the first bytes of
    /// the span are read and decoded, so warming the whole index menu costs
    /// a handful of small preads instead of a full-book decode. The source
    /// fingerprint is validated once up front.
    pub fn collect_titles(&mut self) -> Result<Vec<String>, LueError> {
        self.ensure_current()?;
        let mut titles = Vec::with_capacity(self.starts.len());
        for index in 0..self.starts.len() {
            titles.push(self.read_title_line(index)?);
        }
        Ok(titles)
    }

    fn ensure_file(&mut self) -> Result<&mut File, LueError> {
        if self.file.is_none() {
            self.file = Some(File::open(&self.path)?);
        }
        Ok(self.file.as_mut().expect("file handle just inserted"))
    }

    /// Decode a head window, trimming an incomplete multi-byte character that
    /// the window cut in half (it can only affect the unterminated tail, and
    /// callers only trust newline-terminated lines).
    fn decode_head(&self, raw: &[u8]) -> Result<String, LueError> {
        match self.encoding.as_str() {
            "utf-8" | "utf-8-sig" => {
                let usable = match std::str::from_utf8(raw) {
                    Ok(_) => raw.len(),
                    Err(e) => e.valid_up_to(),
                };
                Ok(String::from_utf8_lossy(&raw[..usable]).into_owned())
            }
            "gb18030" => Ok(encoding_rs::GB18030.decode(raw).0.into_owned()),
            "latin-1" => Ok(raw.iter().map(|&b| b as char).collect()),
            _ => Err(LueError::Invalid("unsupported TXT encoding".into())),
        }
    }

    /// Head-window read limit before giving up on the cheap path and
    /// decoding the whole chapter instead (giant first lines, blank walls).
    const TITLE_HEAD_MAX: usize = 256 * 1024;

    /// The title of one chapter from its head bytes only: the first line
    /// `decode_chapter_uncached` would keep, same filter (keep line 0, the
    /// preface's first non-blank line, or any cleaned line longer than 3
    /// chars). The head window doubles until that line is unambiguous.
    fn read_title_line(&mut self, index: usize) -> Result<String, LueError> {
        let start = self.starts[index] as usize;
        let end = self.ends[index] as usize;
        if start > end {
            return Err(LueError::Invalid(
                "indexed TXT chapter span is outside the source file".into(),
            ));
        }
        let span = end - start;
        let preface = self.intro_first && index == 0;
        let mut head_len = span.min(4096);
        loop {
            let file = self.ensure_file()?;
            file.seek(SeekFrom::Start(start as u64))?;
            let mut raw = vec![0u8; head_len];
            file.read_exact(&mut raw)?;
            let text = self.decode_head(&raw)?;
            let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
            let parts: Vec<&str> = normalized.split('\n').collect();
            // parts[len-1] has no trailing newline in the window; only
            // newline-terminated lines may decide the title.
            if let Some(title) = kept_title(&parts[..parts.len() - 1], preface) {
                return Ok(title);
            }
            if head_len >= span || head_len >= Self::TITLE_HEAD_MAX {
                // Chapter exhausted (or pathologically long lines): fall back
                // to the authoritative full decode.
                let chapter = self.decode_chapter_uncached(index)?;
                return Ok(chapter
                    .first()
                    .cloned()
                    .unwrap_or_else(|| format!("Chapter {}", index + 1)));
            }
            head_len = (head_len * 2).min(span).min(Self::TITLE_HEAD_MAX);
        }
    }

    /// Lay out the chapter window centred on `center` (`before`/`after`
    /// chapters each side). Chapter numbers in the returned layout are
    /// absolute (already shifted by the window base).
    pub fn layout_window(
        &mut self,
        center: usize,
        before: usize,
        after: usize,
        width: usize,
    ) -> Result<DocumentLayout, LueError> {
        if center >= self.starts.len() {
            return Err(LueError::Invalid(format!(
                "chapter index out of range: {center}"
            )));
        }
        let base = center.saturating_sub(before);
        let end = (center + after + 1).min(self.starts.len());
        let mut chapters = Vec::with_capacity(end - base);
        for index in base..end {
            chapters.push(self.read_chapter(index)?);
        }
        let mut layout = layout_document(&chapters, width);
        let shift = base as i64;
        if shift != 0 {
            for (pos, _) in &mut layout.position_to_line {
                pos.0 += shift;
            }
            for owner in layout.line_to_paragraph.iter_mut().flatten() {
                owner.0 += shift;
            }
            for (key, _) in &mut layout.paragraph_line_ranges {
                key.0 += shift;
            }
            for (pos, _) in &mut layout.lines_sorted_positions {
                pos.0 += shift;
            }
        }
        Ok(layout)
    }
}

#[cfg(test)]
mod title_tests {
    use super::*;

    /// Chinese-profile book with `chapters` titled chapters plus a preface.
    fn make_book(dir: &std::path::Path, chapters: usize, name: &str) -> std::path::PathBuf {
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
        let path = dir.join(name);
        fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn collect_titles_matches_per_chapter_decode() {
        let dir = std::env::temp_dir().join("lue_collect_titles");
        fs::create_dir_all(&dir).unwrap();
        let path = make_book(&dir, 12, "collect.txt");
        let index = build_txt_index(path.to_str().unwrap())
            .unwrap()
            .expect("eligible profile");
        let mut bulk = TxtBook::from_index(
            path.to_string_lossy().into_owned(),
            index.encoding.clone(),
            index.starts.clone(),
            index.ends.clone(),
            index.intro_first,
        )
        .unwrap();
        let mut per_chapter = TxtBook::from_index(
            path.to_string_lossy().into_owned(),
            index.encoding,
            index.starts,
            index.ends,
            index.intro_first,
        )
        .unwrap();

        let titles = bulk.collect_titles().unwrap();
        assert_eq!(titles.len(), per_chapter.chapter_count());
        for (i, title) in titles.iter().enumerate() {
            assert_eq!(title, &per_chapter.chapter_title(i).unwrap(), "chapter {i}");
        }
        assert!(titles[1].starts_with("第1章"));
    }

    #[test]
    fn collect_titles_survives_gb18030_and_long_chapters() {
        // Each chapter body far exceeds the 4 KiB head window, and the file
        // is GB18030 so the head cut can split a double-byte character.
        let dir = std::env::temp_dir().join("lue_collect_titles_gb");
        fs::create_dir_all(&dir).unwrap();
        let mut text = String::from("序章介绍\n\n");
        for c in 1..=8 {
            text.push_str(&format!("第{c}章 很长很长的章节标题{c}\n"));
            for _ in 0..400 {
                text.push_str("这一段故意写得特别长，用来确保章节内容远超四KB的头部窗口呀。\n");
            }
        }
        let (encoded, _enc, had_errors) = encoding_rs::GB18030.encode(&text);
        assert!(!had_errors);
        let path = dir.join("gb-book.txt");
        fs::write(&path, &encoded).unwrap();

        let index = build_txt_index(path.to_str().unwrap())
            .unwrap()
            .expect("eligible profile");
        assert_eq!(index.encoding, "gb18030");
        let mut book = TxtBook::from_index(
            path.to_string_lossy().into_owned(),
            index.encoding,
            index.starts,
            index.ends,
            index.intro_first,
        )
        .unwrap();
        let titles = book.collect_titles().unwrap();
        assert_eq!(titles.len(), 9);
        assert_eq!(titles[0], "序章介绍");
        assert!(titles[8].starts_with("第8章"), "got '{}'", titles[8]);
    }

    #[test]
    fn cloned_book_reads_independently() {
        let dir = std::env::temp_dir().join("lue_clone_book");
        fs::create_dir_all(&dir).unwrap();
        let path = make_book(&dir, 6, "clone.txt");
        let index = build_txt_index(path.to_str().unwrap())
            .unwrap()
            .expect("eligible profile");
        let mut original = TxtBook::from_index(
            path.to_string_lossy().into_owned(),
            index.encoding.clone(),
            index.starts.clone(),
            index.ends.clone(),
            index.intro_first,
        )
        .unwrap();
        original.read_chapter(0).unwrap();
        let mut copy = original.clone();
        // The copy shares no file handle or decode cache; it must still read.
        let titles = copy.collect_titles().unwrap();
        assert_eq!(titles.len(), 7);
        assert_eq!(titles[6], original.chapter_title(6).unwrap());
    }

    /// Books without any title-rule structure still open: the whole-book
    /// fallback splits them into ~10 KiB chapters like `_split_txt_by_size`.
    #[test]
    fn whole_book_fallback_chunks_untitled_text() {
        let dir = std::env::temp_dir().join("lue_whole_book");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("untitled.txt");
        let line = "这一行正文用来把无标题的书凑出足够多的字数分块。";
        let per_line = line.chars().count();
        let lines = (TOC_FALLBACK_CHAPTER_CHARS * 5) / per_line + 3;
        let mut text = String::new();
        for i in 0..lines {
            text.push_str(line);
            text.push_str(&format!("序号{i}\n"));
        }
        fs::write(&path, &text).unwrap();

        let index = build_whole_book_index(path.to_str().unwrap())
            .unwrap()
            .expect("untitled book still indexes");
        assert!(index.starts.len() >= 4, "chunks: {}", index.starts.len());
        assert_eq!(index.starts.len(), index.ends.len());

        let mut book = TxtBook::from_index(
            path.to_string_lossy().into_owned(),
            index.encoding.clone(),
            index.starts.clone(),
            index.ends.clone(),
            index.intro_first,
        )
        .unwrap();
        assert_eq!(book.chapter_count(), index.starts.len());
        assert!(book.chapter_title(0).unwrap().starts_with(line));
        book.read_chapter(index.starts.len() - 1).unwrap();
    }

    #[test]
    fn whole_book_fallback_rejects_blank_file() {
        let dir = std::env::temp_dir().join("lue_whole_book_blank");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("blank.txt");
        fs::write(&path, " \n\n \n").unwrap();
        assert!(build_whole_book_index(path.to_str().unwrap())
            .unwrap()
            .is_none());
    }
}
