//! lue_rs — Rust accelerator for the lue terminal eBook reader.
//!
//! Phase 1 of the incremental Rust rewrite (see docs/RUST_REWRITE.md): the
//! pure-text hot paths that dominate book-open time and per-frame cost in
//! `lue/content_parser.py`. Every public function mirrors the observable
//! behaviour of its Python counterpart exactly; parity is enforced from
//! Python by tests/test_rust_parity.py against the retained pure-Python
//! fallbacks.

use fancy_regex::Regex;
use pyo3::prelude::*;
use std::collections::{BTreeSet, HashMap};
use std::sync::OnceLock;

// ---------------------------------------------------------------------------
// Character classes (kept aligned with Python's `\s` / `str.strip()` sets)
// ---------------------------------------------------------------------------

/// Python's `str.isspace()` set: Unicode White_Space plus \x1c-\x1f,
/// which `char::is_whitespace()` alone does not cover.
fn is_py_ws(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

fn py_trim(s: &str) -> &str {
    s.trim_matches(is_py_ws)
}

fn is_spaced_punc(c: char) -> bool {
    matches!(c, '.' | '!' | '?' | '।' | '॥')
}

fn is_cn_punc(c: char) -> bool {
    matches!(c, '。' | '！' | '？')
}

fn is_cn_quote(c: char) -> bool {
    matches!(c, '”' | '」' | '』' | '’' | '"' | '\'')
}

// ---------------------------------------------------------------------------
// split_into_sentences (content_parser.py:20-78)
// ---------------------------------------------------------------------------

const PLACEHOLDER: &str = "<LUE_PERIOD>";

const ABBREVS: &[&str] = &[
    "mr", "mrs", "ms", "dr", "prof", "rev", "hon", "jr", "sr", "cpl", "sgt", "gen", "col",
    "capt", "lt", "pvt", "vs", "viz", "etc", "eg", "ie", "co", "inc", "ltd", "corp", "st",
    "ave", "blvd",
];

/// Python's `\w` for `str` patterns: Unicode alphanumerics plus underscore.
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Whether the '.' at index `i` needs protection: either an abbreviation
/// period (`\b(Mr|Mrs|...|Blvd)\.`, case-insensitive) or an initial
/// (`\b([A-Z])\.(?=\s[A-Z])`). Mirrors the two `re.sub` passes in Python;
/// the two conditions are mutually exclusive, so a single scan is equivalent.
fn needs_protection(chars: &[char], i: usize) -> bool {
    let n = chars.len();
    debug_assert!(chars[i] == '.');
    // initial: `\b([A-Z])\.(?=\s[A-Z])`
    if i >= 1
        && chars[i - 1].is_ascii_uppercase()
        && (i < 2 || !is_word_char(chars[i - 2]))
        && i + 2 < n
        && is_py_ws(chars[i + 1])
        && chars[i + 2].is_ascii_uppercase()
    {
        return true;
    }
    // abbreviation: collect the ASCII letters immediately before the period
    // (max 4: "Prof"/"Blvd"), require a word boundary before the word.
    let mut j = i;
    while j > 0 && i - j < 4 && chars[j - 1].is_ascii_alphabetic() {
        j -= 1;
    }
    let len = i - j;
    if len >= 1 && (j == 0 || !is_word_char(chars[j - 1])) {
        let mut lower = [0u8; 4];
        for (k, slot) in lower.iter_mut().enumerate() {
            *slot = chars[j + k].to_ascii_lowercase() as u8;
        }
        let word = std::str::from_utf8(&lower[..len]).unwrap();
        if ABBREVS.contains(&word) {
            return true;
        }
    }
    false
}

fn protect_periods(chars: &[char]) -> Vec<char> {
    let mut out = Vec::with_capacity(chars.len() + 16);
    for (i, &c) in chars.iter().enumerate() {
        if c == '.' && needs_protection(chars, i) {
            out.extend(PLACEHOLDER.chars());
        } else {
            out.push(c);
        }
    }
    out
}

/// Hand-rolled equivalent of `re.split` with the 4-branch sentence pattern:
///
///   1. `(?<=[.!?।॥])\s+`                — consume whitespace after spaced punct
///   2. `(?<=[。！？])(?!punc|quote)\s*`   — cut right after CJK punctuation
///   3. `(?<=[。！？][quote])(?!punc)\s*`  — same, after one closing quote
///   4. `(?<=[。！？][quote]{2})(?!punc)\s*` — same, after two closing quotes
///
/// The lookahead is evaluated at the match start (before `\s*` consumes),
/// exactly as the Python regex engine does. Rust's `regex` crate cannot
/// split on empty matches, so branches 2-4 are scanned by hand: the cut
/// point must strictly increase between consecutive zero-width cuts, which
/// matches CPython's `re.split` semantics for empty matches (3.7+).
fn sentence_split_segments(chars: &[char]) -> Vec<String> {
    let n = chars.len();
    let mut segments = Vec::new();
    let mut seg_start = 0usize;
    let mut i = 0usize;
    while i <= n {
        if i < n && i >= 1 && is_spaced_punc(chars[i - 1]) && is_py_ws(chars[i]) {
            let mut q = i;
            while q < n && is_py_ws(chars[q]) {
                q += 1;
            }
            segments.push(chars[seg_start..i].iter().collect());
            seg_start = q;
            i = q;
            continue;
        }
        let at_end = i >= n;
        let b2 = i >= 1
            && is_cn_punc(chars[i - 1])
            && (at_end || !(is_cn_punc(chars[i]) || is_cn_quote(chars[i])));
        let b3 = i >= 2
            && is_cn_punc(chars[i - 2])
            && is_cn_quote(chars[i - 1])
            && (at_end || !is_cn_punc(chars[i]));
        let b4 = i >= 3
            && is_cn_punc(chars[i - 3])
            && is_cn_quote(chars[i - 2])
            && is_cn_quote(chars[i - 1])
            && (at_end || !is_cn_punc(chars[i]));
        if b2 || b3 || b4 {
            let mut q = i;
            while q < n && is_py_ws(chars[q]) {
                q += 1;
            }
            segments.push(chars[seg_start..i].iter().collect());
            seg_start = q;
            i = if q > i { q } else { i + 1 };
            continue;
        }
        if at_end {
            break;
        }
        i += 1;
    }
    segments.push(chars[seg_start..].iter().collect());
    segments
}

pub fn split_sentences_impl(paragraph: &str) -> Vec<String> {
    let chars: Vec<char> = paragraph.chars().collect();
    let protected = protect_periods(&chars);
    let mut out = Vec::new();
    for segment in sentence_split_segments(&protected) {
        if segment.is_empty() {
            continue;
        }
        let restored = segment.replace(PLACEHOLDER, ".");
        if !restored.is_empty() {
            out.push(restored);
        }
    }
    // Mirror the Python fallback: only reachable for empty input.
    if out.is_empty() {
        out.push(paragraph.to_string());
    }
    out
}

#[pyfunction]
fn split_sentences(paragraph: &str) -> Vec<String> {
    split_sentences_impl(paragraph)
}

/// Batch form for whole-book passes (e.g. `total_sentences` at load time):
/// one FFI crossing instead of one per paragraph.
pub fn split_sentences_batch_impl(paragraphs: &[&str]) -> Vec<Vec<String>> {
    paragraphs.iter().map(|p| split_sentences_impl(p)).collect()
}

#[pyfunction]
fn split_sentences_batch(paragraphs: Vec<String>) -> Vec<Vec<String>> {
    let refs: Vec<&str> = paragraphs.iter().map(String::as_str).collect();
    split_sentences_batch_impl(&refs)
}

// ---------------------------------------------------------------------------
// clean_visual_text (content_parser.py:121-213)
// ---------------------------------------------------------------------------

const CLEAN_RULES_A: &[(&str, &str)] = &[
    (r"\s*\.\s*\.\s*\.\s*(\.\s*)*", "..."),
    (r"\s*\.\s*\.\s*(?!\s*\.)", ".."),
    (r"\.{4,}", "..."),
    (r"[-_=~`^]{3,}", ""),
    (r"[*]{4,}", ""),
    (r"[#]{4,}", ""),
    (r"[+]{3,}", ""),
    (r"[|]{3,}", ""),
    (r"[\\]{3,}", ""),
    (r"[/]{3,}", ""),
];

const UNICODE_REPLACEMENTS: &[(char, &str)] = &[
    ('×', " multiplied by "),
    ('÷', " divided by "),
    ('±', " plus or minus "),
    ('≤', " less than or equal to "),
    ('≥', " greater than or equal to "),
    ('≠', " not equal to "),
    ('≈', " approximately"),
    ('∞', "infinity "),
    ('%', " percent "),
    ('+', " plus "),
    ('=', " equals "),
    ('°', " degrees "),
    ('™', " trademark "),
    ('®', " registered "),
    ('©', " copyright "),
    ('§', " section "),
    ('’', "'"),
    ('\u{200b}', ""),
    ('\u{200c}', ""),
    ('\u{200d}', ""),
    ('\u{feff}', ""),
    ('\u{00ad}', ""),
];

const CLEAN_RULES_B: &[(&str, &str)] = &[
    (r"\.{4,}", "..."),
    (r"…+", "..."),
    (r"\s+", " "),
    (r"\n\s*\n\s*\n+", "\n\n"),
    (r"\.\.\.(?=\S)", "... "),
    (r"\*\*([^*]+)\*\*", "$1"),
    (r"\*([^*]+)\*", "$1"),
    (r"__([^_]+)__", "$1"),
    (r"_([^_]+)_", "$1"),
    (r"`([^`]+)`", "$1"),
    (r"~~([^~]+)~~", "$1"),
    (r"\[([^\]]+)\]\([^)]+\)", "$1"),
    (r"\[([^\]]+)\]\[[^\ ]*\]", "$1"),
    (r"(?m)^\s*\[[^\ ]+\]:\s*\S+.*$", ""),
    (r"(?m)^#{1,6}\s+", ""),
    (r"\s+([,!?;:])", "$1"),
    (r"([,!?;:])\s*([,!?;:])", "$1 $2"),
];

fn clean_rules_a() -> &'static Vec<(Regex, &'static str)> {
    static RULES: OnceLock<Vec<(Regex, &str)>> = OnceLock::new();
    RULES.get_or_init(|| compile_rules(CLEAN_RULES_A))
}

fn clean_rules_b() -> &'static Vec<(Regex, &'static str)> {
    static RULES: OnceLock<Vec<(Regex, &str)>> = OnceLock::new();
    RULES.get_or_init(|| compile_rules(CLEAN_RULES_B))
}

fn compile_rules(rules: &'static [(&'static str, &'static str)]) -> Vec<(Regex, &'static str)> {
    rules
        .iter()
        .map(|(pattern, replacement)| (Regex::new(pattern).unwrap(), *replacement))
        .collect()
}

pub fn clean_visual_text_impl(text: &str) -> String {
    if text.starts_with("__CODE_BLOCK__") {
        return text["__CODE_BLOCK__".len()..].to_string();
    }
    let mut t = text.to_string();
    for (re, replacement) in clean_rules_a() {
        t = re.replace_all(&t, *replacement).into_owned();
    }
    for (old, new) in UNICODE_REPLACEMENTS {
        t = t.replace(*old, *new);
    }
    for (re, replacement) in clean_rules_b() {
        t = re.replace_all(&t, *replacement).into_owned();
    }
    py_trim(&t).to_string()
}

#[pyfunction]
fn clean_visual_text(text: &str) -> String {
    clean_visual_text_impl(text)
}

/// Batch form of the per-line cleaning loop in `_extract_content_txt`:
/// `(line_no, clean_visual_text(line.strip()))` for every non-empty result.
pub fn clean_txt_doc_lines_impl(content: &str) -> Vec<(usize, String)> {
    content
        .split('\n')
        .enumerate()
        .map(|(i, line)| (i, clean_visual_text_impl(py_trim(line))))
        .filter(|(_, text)| !text.is_empty())
        .collect()
}

#[pyfunction]
fn clean_txt_doc_lines(content: &str) -> Vec<(usize, String)> {
    clean_txt_doc_lines_impl(content)
}

/// CJK line statistics for `_extract_content_txt`: `(cjk_line_count,
/// total_chars_in_cjk_lines)` in char (not byte) units, matching `len()`.
pub fn analyze_cjk_impl(content: &str) -> (usize, usize) {
    let mut count = 0usize;
    let mut total = 0usize;
    for line in content.split('\n') {
        if line.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)) {
            count += 1;
            total += line.chars().count();
        }
    }
    (count, total)
}

#[pyfunction]
fn analyze_cjk(content: &str) -> (usize, usize) {
    analyze_cjk_impl(content)
}

// ---------------------------------------------------------------------------
// TXT chapter (TOC) rules (content_parser.py:730-849, ported from legado)
// ---------------------------------------------------------------------------

const TXT_TOC_RULES: &[&str] = &[
    r"(?<=[　\s])(?:序章|楔子|正文(?!完|结)|终章|后记|尾声|番外|第\s{0,4}[\d〇零一二两三四五六七八九十百千万壹贰叁肆伍陆柒捌玖拾佰仟]+?\s{0,4}(?:章|节(?!课)|卷|集(?![合和]))).{0,30}$",
    r"^[ 　\t]{0,4}(?:序章|楔子|正文(?!完|结)|终章|后记|尾声|番外|第\s{0,4}[\d〇零一二两三四五六七八九十百千万壹贰叁肆伍陆柒捌玖拾佰仟]+?\s{0,4}(?:章|节(?!课)|卷|集(?![合和])|部(?![分赛游])|篇(?!张))).{0,30}$",
    r"^[ 　\t]{0,4}\d{1,5}[:：,.， 、_—\-].{1,30}$",
    r"^[ 　\t]{0,4}(?:序章|楔子|正文(?!完|结)|终章|后记|尾声|番外|[零一二两三四五六七八九十百千万壹贰叁肆伍陆柒捌玖拾佰仟]{1,8}章?)[ 、_—\-].{1,30}$",
    r"^[ 　\t]{0,4}正文[ 　]{1,4}.{0,20}$",
    r"^[ 　\t]{0,4}(?:[Cc]hapter|[Ss]ection|[Pp]art|ＰＡＲＴ|[Nn][oO][.、]|[Ee]pisode|(?:内容|文章)?简介|文案|前言|序章|楔子|正文(?!完|结)|终章|后记|尾声|番外)\s{0,4}\d{1,4}.{0,30}$",
    r"(?<=[\s　])[【〔〖「『〈［\[](?:第|[Cc]hapter)[\d零一二两三四五六七八九十百千万壹贰叁肆伍陆柒捌玖拾佰仟]{1,10}[章节].{0,20}$",
    r"^[ 　\t]{0,4}(?:[☆★✦✧].{1,30}|(?:内容|文章)?简介|文案|前言|序章|楔子|正文(?!完|结)|终章|后记|尾声|番外)[ 　]{0,4}$",
    r"^[ \t　]{0,4}(?:(?:内容|文章)?简介|文案|前言|序章|楔子|正文(?!完|结)|终章|后记|尾声|番外|[卷章][\d零一二两三四五六七八九十百千万壹贰叁肆伍陆柒捌玖拾佰仟]{1,8})[ 　]{0,4}.{0,30}$",
    r"^[一-龥]{1,20}[ 　\t]{0,4}[(（][\d〇零一二两三四五六七八九十百千万壹贰叁肆伍陆柒捌玖拾佰仟]{1,8}[)）][ 　\t]{0,4}$",
    r"^[一-龥]{1,20}[ 　\t]{0,4}[\d〇零一二两三四五六七八九十百千万壹贰叁肆伍陆柒捌玖拾佰仟]{1,8}[ 　\t]{0,4}$",
    r"^[ 　\t]{0,4}(?:.{0,15}分[页节章段]阅读[-_ ]|第\s{0,4}[\d零一二两三四五六七八九十百千万]{1,6}\s{0,4}[页节]).{0,30}$",
];

const TOC_VALID_CHAPTER_LENGTH: i64 = 1000;
const TOC_ERROR_CHAPTER_LENGTH: i64 = 100;
const TOC_GOOD_ENOUGH: i64 = 70;
const TOC_RULE_OVERSHOOT: i64 = 2;

fn toc_regexes() -> &'static Vec<Regex> {
    static RULES: OnceLock<Vec<Regex>> = OnceLock::new();
    RULES.get_or_init(|| {
        TXT_TOC_RULES
            .iter()
            // Python compiles these with re.MULTILINE.
            .map(|pattern| Regex::new(&format!("(?m){pattern}")).unwrap())
            .collect()
    })
}

/// Map the given byte offsets to char offsets (Python's `str` indices).
fn byte_to_char_map(text: &str, mut edges: Vec<usize>) -> HashMap<usize, usize> {
    edges.sort_unstable();
    edges.dedup();
    let mut map = HashMap::with_capacity(edges.len());
    let mut ei = 0usize;
    let mut ci = 0usize;
    for (bi, _) in text.char_indices() {
        while ei < edges.len() && edges[ei] == bi {
            map.insert(bi, ci);
            ei += 1;
        }
        ci += 1;
    }
    if ei < edges.len() && edges[ei] == text.len() {
        map.insert(text.len(), ci);
    }
    map
}

/// Score one rule's match spans exactly like the Python scorer: first match
/// always counts, later matches count when preceded by >1000 chars of content
/// else as a misfire when preceded by <100 chars. Returns `(cs_num, num_e)`.
fn toc_count(sample: &str, spans: &[(usize, usize)]) -> (i64, i64) {
    let edges: Vec<usize> = spans.iter().flat_map(|&(s, e)| [s, e]).collect();
    let cmap = byte_to_char_map(sample, edges);
    let mut cs_num: i64 = 0;
    let mut num_e: i64 = 0;
    let mut start: Option<usize> = None;
    for &(s, e) in spans {
        let sc = cmap.get(&s).copied().unwrap_or(0);
        let ec = cmap.get(&e).copied().unwrap_or(0);
        match start {
            None => {
                cs_num += 1;
                start = Some(ec);
            }
            Some(st) => {
                let content_length = sc as i64 - st as i64;
                if content_length > TOC_VALID_CHAPTER_LENGTH {
                    cs_num += 1;
                    start = Some(ec);
                } else if content_length < TOC_ERROR_CHAPTER_LENGTH {
                    num_e += 1;
                }
            }
        }
    }
    (cs_num, num_e)
}

/// NFA (backtracking-free) repro of the "第X章" branch of rule 0 (legado)
/// for the rule-selection fast path. The regex crate has no lookbehind, so
/// the required preceding whitespace/full-width-space is matched as a
/// literal prefix instead and callers advance the span start past it; the
/// match set then exactly equals the fancy matches of that branch.
fn fast_toc_regexes() -> &'static Vec<Option<regex::Regex>> {
    static RULES: OnceLock<Vec<Option<regex::Regex>>> = OnceLock::new();
    RULES.get_or_init(|| {
        let num = r"[\d〇零一二两三四五六七八九十百千万壹贰叁肆伍陆柒捌玖拾佰仟]{1,10}";
        vec![regex::Regex::new(&format!(
            r"(?m)(?:[　\s])第{num}(?:章|节|卷|集).{{0,30}}$"
        ))
        .ok()]
    })
}

/// Score every rule over the sample and return the winning rule index.
/// `m.start()`/`m.end()` must be compared in char units, as in Python.
pub fn pick_toc_rule_impl(sample: &str) -> Option<usize> {
    // Fast path: scan the "第X章" branch of rule 0 with a pure NFA. The
    // lookbehind is emulated by anchoring a literal ws/full-width-space
    // prefix and shifting each match start past it; the resulting span set
    // equals the fancy branch's, so winning here implies rule 0 wins the
    // full scan too (superset scoring), and the early exit is safe.
    if let Some(fast0) = fast_toc_regexes().get(0).and_then(|o| o.as_ref()) {
        let shifted: Vec<(usize, usize)> = fast0
            .find_iter(sample)
            .map(|m| {
                // 前缀 (?:[　\s]) 已匹配一个字符;“第”的起始 = 该字符的
                // 结束偏移。不能用 m.start()+1(全角空格 3 字节会踩到
                // 字符内部),按实际 UTF-8 长度推进。
                let head_len = sample[m.start()..]
                    .chars()
                    .next()
                    .map_or(0, |c| c.len_utf8());
                (m.start() + head_len, m.end())
            })
            .collect();
        if !shifted.is_empty() {
            let (cs, err) = toc_count(sample, &shifted);
            // Matches equal the full fancy rule-0 branch set (superset of the
            // exact lookahead), so a win here means rule 0 wins the full scan.
            if cs >= err * 3 && cs > 1 {
                return Some(0);
            }
        }
    }
    // Full fancy scan, identical scoring to the Python implementation.
    let mut best_count: i64 = -1;
    let mut best: Option<usize> = None;
    for (idx, re) in toc_regexes().iter().enumerate() {
        let spans: Vec<(usize, usize)> = re
            .find_iter(sample)
            .flatten()
            .map(|m| (m.start(), m.end()))
            .collect();
        let (cs_num, num_e) = toc_count(sample, &spans);
        if cs_num >= num_e * 3 && cs_num > best_count + TOC_RULE_OVERSHOOT {
            best_count = cs_num;
            best = Some(idx);
            if best_count > TOC_GOOD_ENOUGH {
                break;
            }
        }
    }
    best
}

#[pyfunction]
fn pick_toc_rule(sample: &str) -> Option<usize> {
    pick_toc_rule_impl(sample)
}

/// Sorted 0-based line numbers whose line contains a match of `rule_idx`
/// anywhere (the match start decides the line, as in `_collect_title_lines`).
/// Line starts are compared in byte offsets, which preserves the ordering of
/// Python's char-offset `bisect_right` exactly.
pub fn collect_title_lines_impl(content: &str, rule_idx: usize) -> Vec<u64> {
    let rules = toc_regexes();
    if rule_idx >= rules.len() {
        return Vec::new();
    }
    let re = &rules[rule_idx];
    let mut line_starts: Vec<usize> = Vec::new();
    line_starts.push(0);
    for (bi, c) in content.char_indices() {
        if c == '\n' {
            line_starts.push(bi + 1);
        }
    }
    let mut lines = BTreeSet::new();
    for m in re.find_iter(content).flatten() {
        let line = line_starts.partition_point(|&x| x <= m.start()) - 1;
        lines.insert(line as u64);
    }
    lines.into_iter().collect()
}

#[pyfunction]
fn collect_title_lines(content: &str, rule_idx: usize) -> Vec<u64> {
    collect_title_lines_impl(content, rule_idx)
}

// ---------------------------------------------------------------------------
// _split_long_text_into_paragraphs (content_parser.py:869-884)
// ---------------------------------------------------------------------------

pub fn split_long_text_paragraphs_impl(text: &str, max_chars: usize) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    // re.split(r'(?<=[。！？!?；;])') — zero-width cut after each sentence punct.
    let mut parts: Vec<String> = Vec::new();
    let mut start = 0usize;
    for i in 0..chars.len() {
        if matches!(chars[i], '。' | '！' | '？' | '!' | '?' | '；' | ';') {
            parts.push(chars[start..=i].iter().collect());
            start = i + 1;
        }
    }
    parts.push(chars[start..].iter().collect());

    let mut paragraphs = Vec::new();
    let mut buf = String::new();
    let mut buf_len = 0usize;
    for part in parts {
        let trimmed = py_trim(&part);
        if trimmed.is_empty() {
            continue;
        }
        buf.push_str(trimmed);
        buf_len += trimmed.chars().count();
        if buf_len >= max_chars {
            paragraphs.push(std::mem::take(&mut buf));
            buf_len = 0;
        }
    }
    if !buf.is_empty() {
        paragraphs.push(buf);
    }
    paragraphs
}

#[pyfunction]
fn split_long_text_paragraphs(text: &str, max_chars: usize) -> Vec<String> {
    split_long_text_paragraphs_impl(text, max_chars)
}

// ---------------------------------------------------------------------------
// Rich-compatible word wrap (ui.py:195 — update_document_layout's hot loop)
// ---------------------------------------------------------------------------

mod cell_table;

/// cell width of a single char, via the same 450-range binary search as
/// `rich.cells.get_character_cell_size` (width -1 -> 0).
pub fn char_cell_width(c: char) -> usize {
    let codepoint = c as u32;
    let table: &[(u32, u32, i32)] = cell_table::CELL_WIDTHS;
    let (mut lo, mut hi) = (0usize, table.len() - 1);
    while lo <= hi {
        let mid = (lo + hi) / 2;
        let (start, end, width) = table[mid];
        if codepoint < start {
            hi = mid - 1;
        } else if codepoint > end {
            lo = mid + 1;
        } else {
            return if width == -1 { 0 } else { width as usize };
        }
    }
    1
}

fn py_split_ws(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// `cell_len(text)` — sum of per-char cell widths.
fn cell_len(s: &str) -> usize {
    s.chars().map(char_cell_width).sum()
}

/// `rich._wrap.chop_cells`: greedily fold chars so each line fits `width`.
fn chop_cells(text: &str, width: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut total = 0usize;
    for c in text.chars() {
        let w = char_cell_width(c);
        if total + w > width {
            lines.push(c.to_string());
            total = w;
        } else {
            if lines.is_empty() {
                lines.push(String::new());
            }
            lines.last_mut().unwrap().push(c);
            total += w;
        }
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

/// `rich._wrap.divide_line(text, width, fold=true)` — word-wrap into cell
/// offsets. Words are `\s*\S+\s*` runs; a word that fits is appended; a word
/// longer than `width` is hard-folded; otherwise it moves to the next line.
fn divide_line(text: &str, width: usize, fold: bool) -> Vec<usize> {
    let mut breaks: Vec<usize> = Vec::new();
    let mut cell_offset: usize = 0;
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();

    // Emulate re_word (`\s*\S+\s*`): matches start at the scan position, so
    // leading whitespace (start of line) belongs to the first word, and
    // inter-word whitespace is consumed by the preceding word's `\s*`.
    // Trailing whitespace at the end of the text matches no word.
    let mut i = 0usize;
    while i < n {
        let start = i;
        // `\s*` then require a `\S+` — all-whitespace tail yields no word
        while i < n && py_split_ws(chars[i]) {
            i += 1;
        }
        if i >= n {
            break;
        }
        while i < n && !py_split_ws(chars[i]) {
            i += 1;
        }
        // trailing whitespace included in the word
        let mut end = i;
        while end < n && py_split_ws(chars[end]) {
            end += 1;
        }
        let word: String = chars[start..end].iter().collect();
        let word_stripped_len = cell_len(&chars[start..i].iter().collect::<String>());
        let remaining = width.saturating_sub(cell_offset);
        if remaining >= word_stripped_len {
            cell_offset += cell_len(&word);
        } else if word_stripped_len > width {
            if fold {
                let folded = chop_cells(&chars[start..end].iter().collect::<String>(), width);
                // rich pushes the (advancing) word start before every folded
                // sub-line and extends `start` by each sub-line's char length;
                // the outer loop then continues at the word's end.
                let mut break_start = start;
                for (idx, line) in folded.iter().enumerate() {
                    if break_start > 0 {
                        breaks.push(break_start);
                    }
                    if idx == folded.len() - 1 {
                        cell_offset = cell_len(line);
                    } else {
                        break_start += line.chars().count();
                    }
                }
                i = end;
                continue;
            } else {
                if start > 0 {
                    breaks.push(start);
                }
                cell_offset = cell_len(&word);
            }
        } else if cell_offset > 0 && start > 0 {
            breaks.push(start);
            cell_offset = cell_len(&word);
        }
        i = end;
    }
    breaks
}

/// `rich.text.Text.expand_tabs`: replace each tab with spaces so the cell
/// position advances to the next multiple of `tab_size` (default 8).
fn expand_tabs(text: &str, tab_size: usize) -> String {
    if !text.contains('\t') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len() + 8);
    let mut cell_pos = 0usize;
    for c in text.chars() {
        if c == '\t' {
            let tab_remainder = cell_pos % tab_size;
            // rich expands a tab to 1 space then pads to the next multiple of
            // all-space width; a tab at an exact tab stop still adds a full
            // tab stop (1 + (8 - 1)) = 8 spaces.
            let spaces = if tab_remainder == 0 {
                tab_size
            } else {
                tab_size - tab_remainder
            };
            for _ in 0..spaces {
                out.push(' ');
            }
            cell_pos += spaces;
        } else {
            out.push(c);
            cell_pos += char_cell_width(c);
        }
    }
    out
}

/// Post-wrap cleanup applied by `Text.wrap` to every divided line:
/// `rstrip_end(width)` strips trailing whitespace when the CHAR length
/// exceeds the width, then `truncate(width, "crop")` trims to width cells.
fn wrap_postprocess(line: &str, width: usize) -> String {
    let char_len = line.chars().count();
    let mut s = line.to_string();
    if char_len > width {
        // rstrip_end: strip up to `excess` trailing whitespace chars
        // (Python re \s — unicode whitespace only).
        let excess = char_len - width;
        let trail = s.chars().rev().take_while(|c| c.is_whitespace()).count();
        let cut = trail.min(excess);
        if cut > 0 {
            let total_chars = s.chars().count();
            if cut >= total_chars {
                s.clear();
            } else {
                // keep the first (total_chars - cut) chars
                let boundary = s
                    .char_indices()
                    .nth(total_chars - cut - 1)
                    .map(|(i, c)| i + c.len_utf8())
                    .unwrap_or(0);
                s.truncate(boundary);
            }
        }
    }
    // truncate(width, pad=True) as called by justify("left"):
    //   cell > width  -> set_cell_size (a wide char that would straddle the
    //                    boundary is dropped and replaced by one space)
    //   cell < width  -> pad with spaces
    // The pad decision uses the pre-crop cell length, like rich's truncate.
    let cells_now = cell_len(&s);
    if cells_now > width {
        s = set_cell_size(&s, width);
    } else if cells_now < width {
        for _ in 0..(width - cells_now) {
            s.push(' ');
        }
    }
    s
}

/// `rich.cells.set_cell_size` — fit text into exactly `total` cells.
fn set_cell_size(text: &str, total: usize) -> String {
    if total == 0 {
        return String::new();
    }
    let cell_size = cell_len(text);
    if cell_size == total {
        return text.to_string();
    }
    if cell_size < total {
        let mut out = String::from(text);
        for _ in 0..(total - cell_size) {
            out.push(' ');
        }
        return out;
    }
    // Accumulate cells; a 2-wide char straddling the boundary (cells+2 ==
    // total+1) is dropped and replaced by a single space, as in the binary
    // search in rich's set_cell_size.
    let mut cells = 0usize;
    let mut out = String::new();
    for c in text.chars() {
        let w = char_cell_width(c);
        if cells + w > total {
            if w == 2 && cells + 1 == total {
                out.push(' ');
            }
            break;
        }
        out.push(c);
        cells += w;
        if cells == total {
            break;
        }
    }
    out
}

/// `rich.text.Text.wrap` equivalent: split on newlines, divide each line,
/// slice by offsets — returns the plain line strings.
pub fn wrap_plain(text: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.split('\n') {
        let line = expand_tabs(line, 8);
        let offsets = divide_line(&line, width, true);
        if offsets.is_empty() {
            out.push(wrap_postprocess(&line, width));
            continue;
        }
        let chars: Vec<char> = line.chars().collect();
        let mut prev = 0usize;
        for &off in &offsets {
            let slice: String = chars[prev..off].iter().collect();
            out.push(wrap_postprocess(&slice, width));
            prev = off;
        }
        let last: String = chars[prev..].iter().collect();
        if !last.is_empty() {
            out.push(wrap_postprocess(&last, width));
        }
    }
    out
}

#[pyfunction]
fn char_cell_width_py(c: String) -> usize {
    // rich.cells.get_character_cell_size takes a single char
    match c.chars().next() {
        Some(ch) => char_cell_width(ch),
        None => 0,
    }
}

/// Diagnostic: cell_len(text) same as rich.cells.cell_len.
#[pyfunction]
fn cell_len_py(text: &str) -> usize {
    cell_len(text)
}

/// Diagnostic: expand_tabs(text, size) same as rich Text.expand_tabs.
#[pyfunction]
fn expand_tabs_py(text: &str, tab_size: usize) -> String {
    expand_tabs(text, tab_size)
}

/// Diagnostic: divide_line offsets (char indices), same as rich._wrap.divide_line.
#[pyfunction]
fn divide_line_py(text: &str, width: usize) -> Vec<usize> {
    divide_line(&expand_tabs(text, 8), width, true)
}

#[pyfunction]
fn wrap_paragraph(text: &str, width: usize) -> Vec<String> {
    wrap_plain(text, width)
}

// ---------------------------------------------------------------------------
// Python module
// ---------------------------------------------------------------------------

#[pymodule]
fn lue_rs(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(split_sentences, m)?)?;
    m.add_function(wrap_pyfunction!(split_sentences_batch, m)?)?;
    m.add_function(wrap_pyfunction!(clean_visual_text, m)?)?;
    m.add_function(wrap_pyfunction!(clean_txt_doc_lines, m)?)?;
    m.add_function(wrap_pyfunction!(analyze_cjk, m)?)?;
    m.add_function(wrap_pyfunction!(pick_toc_rule, m)?)?;
    m.add_function(wrap_pyfunction!(collect_title_lines, m)?)?;
    m.add_function(wrap_pyfunction!(split_long_text_paragraphs, m)?)?;
    m.add_function(wrap_pyfunction!(wrap_paragraph, m)?)?;
    m.add_function(wrap_pyfunction!(char_cell_width_py, m)?)?;
    m.add_function(wrap_pyfunction!(cell_len_py, m)?)?;
    m.add_function(wrap_pyfunction!(expand_tabs_py, m)?)?;
    m.add_function(wrap_pyfunction!(divide_line_py, m)?)?;
    m.add("__version__", "0.1.0")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests (mirrored from tests/test_rust_parity.py)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sentences_english() {
        assert_eq!(
            split_sentences_impl("One. Two. Three."),
            vec!["One.", "Two.", "Three."]
        );
        assert_eq!(
            split_sentences_impl("Mr. Smith arrived. He left."),
            vec!["Mr. Smith arrived.", "He left."]
        );
        assert_eq!(
            split_sentences_impl("J. F. Kennedy spoke."),
            vec!["J. F. Kennedy spoke."]
        );
        assert_eq!(split_sentences_impl("hello world"), vec!["hello world"]);
        assert_eq!(split_sentences_impl(""), vec![""]);
        assert_eq!(split_sentences_impl("End. "), vec!["End."]);
    }

    #[test]
    fn sentences_cjk() {
        assert_eq!(
            split_sentences_impl("今天天气很好。我们去公园吧!"),
            vec!["今天天气很好。", "我们去公园吧!"]
        );
        assert_eq!(split_sentences_impl("。。哈哈"), vec!["。。", "哈哈"]);
        // Adjacent zero-width cuts: quote run after CJK punctuation.
        assert_eq!(split_sentences_impl("。””汉"), vec!["。”", "”", "汉"]);
        // Trailing cut at end of text produces an empty tail that is dropped;
        // half-width "!?" without following whitespace do not split (branch 1
        // requires \s+, only CJK punctuation splits without whitespace).
        assert_eq!(
            split_sentences_impl("半。角!混?合。"),
            vec!["半。", "角!混?合。"]
        );
    }

    #[test]
    fn clean_basic() {
        // Rule 15 re-adds a space after "..." when followed by non-space.
        assert_eq!(clean_visual_text_impl("a . . . b"), "a... b");
        assert_eq!(clean_visual_text_impl("a . . b"), "a..b");
        assert_eq!(clean_visual_text_impl("a....b"), "a... b");
        assert_eq!(clean_visual_text_impl("----"), "");
        assert_eq!(clean_visual_text_impl("**bold** text"), "bold text");
        assert_eq!(clean_visual_text_impl("*em*"), "em");
        assert_eq!(clean_visual_text_impl("`code`"), "code");
        assert_eq!(clean_visual_text_impl("[label](http://x)"), "label");
        assert_eq!(clean_visual_text_impl("# Header"), "Header");
        assert_eq!(clean_visual_text_impl("a , b"), "a, b");
        assert_eq!(clean_visual_text_impl("a!!b"), "a! !b");
        assert_eq!(clean_visual_text_impl("3 × 4"), "3 multiplied by 4");
        assert_eq!(clean_visual_text_impl("don’t"), "don't");
        assert_eq!(clean_visual_text_impl("…"), "...");
        assert_eq!(
            clean_visual_text_impl("__CODE_BLOCK__    x = 1"),
            "    x = 1"
        );
    }

#[test]
    fn toc_picks_standard_rule() {
        // 120 chapters: the fast NFA path clears the 70-chapter bar and
        // must return rule 0 (same as the full fancy scan would).
        let mut book = String::from("测试书名\n\n");
        let digits = ['零', '一', '二', '三', '四', '五', '六', '七', '八', '九'];
        for i in 1..=120 {
            book.push_str(&format!("第{}章 试验\n\n", digits[i % 10]));
            for _ in 0..300 {
                book.push_str("这里的正文一行一句比较短。\n");
            }
            book.push('\n');
        }
        // Rule 0 wins: every title line is preceded by "\n" which satisfies
        // its (?<=[　\s]) lookbehind, and the count clears the early exit.
        assert_eq!(pick_toc_rule_impl(&book), Some(0));
        let lines = collect_title_lines_impl(&book, 0);
        assert_eq!(lines.len(), 120);
        // Line 0 = "测试书名", line 1 = "", line 2 = first title.
        assert_eq!(&lines[..3], &[2u64, 305, 608]);
    }

    #[test]
    fn toc_fast_path_compiles_and_hits() {
        let mut book = String::from("测试书名\n\n");
        let digits = ['零', '一', '二', '三', '四', '五', '六', '七', '八', '九'];
        for i in 1..=120 {
            book.push_str(&format!("第{}章 试验\n\n", digits[i % 10]));
            for _ in 0..30 {
                book.push_str("正文一行一句比较短。\n");
            }
            book.push('\n');
        }
        let fast = fast_toc_regexes().get(0).and_then(|o| o.as_ref());
        assert!(fast.is_some(), "fast NFA regex must compile");
        let count = fast.unwrap().find_iter(&book).count();
        eprintln!("fast TOC matches: {count}");
        assert!(count >= 80, "expected >=80 fast matches, got {count}");
    }

    #[test]
    fn long_text_paragraphs() {
        let text = "句子。".repeat(100);
        let paras = split_long_text_paragraphs_impl(&text, 200);
        assert!(!paras.is_empty());
        assert!(paras.iter().all(|p| p.chars().count() >= 200 || p == paras.last().unwrap()));
    }
}
