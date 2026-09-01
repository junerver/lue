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
    file: Option<File>,
    chapter_cache: VecDeque<(usize, Vec<String>)>,
    cache_capacity: usize,
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
        let file = match self.file.as_mut() {
            Some(f) => f,
            None => {
                let f = File::open(&self.path)?;
                self.file.insert(f)
            }
        };
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
