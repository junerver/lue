//! Text pipeline: sentence splitting, visual cleaning and CJK analysis.
//! Every function mirrors the observable behaviour of its counterpart in
//! `lue/content_parser.py`; parity is enforced by tests in both crates and
//! from Python by `tests/test_rust_parity.py`.

use fancy_regex::Regex;
use std::sync::OnceLock;

// ---------------------------------------------------------------------------
// Character classes (kept aligned with Python's `\s` / `str.strip()` sets)
// ---------------------------------------------------------------------------

/// Python's `str.isspace()` set: Unicode White_Space plus \x1c-\x1f,
/// which `char::is_whitespace()` alone does not cover.
pub fn is_py_ws(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

pub fn py_trim(s: &str) -> &str {
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

/// Batch form for whole-book passes (e.g. `total_sentences` at load time):
/// one FFI crossing instead of one per paragraph.
pub fn split_sentences_batch_impl(paragraphs: &[&str]) -> Vec<Vec<String>> {
    paragraphs.iter().map(|p| split_sentences_impl(p)).collect()
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
    if let Some(rest) = text.strip_prefix("__CODE_BLOCK__") {
        return rest.to_string();
    }
    let mut t = text.to_string();
    for (re, replacement) in clean_rules_a() {
        t = re.replace_all(&t, *replacement).into_owned();
    }
    for (old, new) in UNICODE_REPLACEMENTS {
        t = t.replace(*old, new);
    }
    for (re, replacement) in clean_rules_b() {
        t = re.replace_all(&t, *replacement).into_owned();
    }
    py_trim(&t).to_string()
}

/// Fast-path precheck: if a line contains no character that any cleaning
/// rule can change, `clean_visual_text(line) == line` for an already-trimmed
/// line. Verified against the real 11MB book: 92% of lines take this path
/// and zero lines are misclassified (see benchmarks/clean_precheck_probe.py).
fn clean_precheck_safe(text: &str) -> bool {
    let mut prev_ws = false;
    for c in text.chars() {
        if matches!(
            c,
            '!' | '#' | '%' | '*' | '+' | ',' | '-' | '.' | '/' | ':' | ';' | '='
                | '?' | '[' | '\\' | ']' | '^' | '_' | '`' | '{' | '|' | '}' | '~'
                | '×' | '÷' | '±' | '≤' | '≥' | '≠' | '≈' | '∞' | '°' | '™' | '®'
                | '©' | '§' | '’' | '\u{200b}' | '\u{200c}' | '\u{200d}' | '\u{feff}'
                | '\u{00ad}' | '\u{2026}'
        ) {
            return false;
        }
        if c.is_whitespace() {
            // any non-ASCII-space whitespace (e.g. U+3000) is normalized by
            // \s+ -> ' '; consecutive ASCII spaces also collapse.
            if c != ' ' || prev_ws {
                return false;
            }
            prev_ws = true;
        } else {
            prev_ws = false;
        }
    }
    true
}

/// Batch form of the per-line cleaning loop in `_extract_content_txt`:
/// `(line_no, clean_visual_text(line.strip()))` for every non-empty result.
pub fn clean_txt_doc_lines_impl(content: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    for (i, line) in content.split('\n').enumerate() {
        let trimmed = py_trim(line);
        if trimmed.is_empty() {
            continue;
        }
        let text = if clean_precheck_safe(trimmed) {
            trimmed.to_string()
        } else {
            clean_visual_text_impl(trimmed)
        };
        if !text.is_empty() {
            out.push((i, text));
        }
    }
    out
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
