//! TXT chapter (TOC) rules (content_parser.py:730-849, ported from legado).

use fancy_regex::Regex;
use std::collections::{BTreeSet, HashMap};
use std::sync::OnceLock;

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
    if let Some(fast0) = fast_toc_regexes().first().and_then(|o| o.as_ref()) {
        let shifted: Vec<(usize, usize)> = fast0
            .find_iter(sample)
            // regex crate 在此输入上可能返回非字符边界的 match 偏移
            // (全角空格与 \s 组合的已知边界问题),跳过而非切片 panic
            .filter_map(|m| {
                if !sample.is_char_boundary(m.start()) || !sample.is_char_boundary(m.end()) {
                    return None;
                }
                // 前缀 (?:[　\s]) 已匹配一个字符;“第”的起始 = 该字符的
                // 结束偏移。不能用 m.start()+1(全角空格 3 字节会踩到
                // 字符内部),按实际 UTF-8 长度推进。
                let head_len = sample[m.start()..]
                    .chars()
                    .next()
                    .map_or(0, |c| c.len_utf8());
                Some((m.start() + head_len, m.end()))
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
