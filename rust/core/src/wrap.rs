//! Rich-compatible word wrap (ui.py:195 — update_document_layout's hot loop).

use crate::cell_table;

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
pub fn cell_len(s: &str) -> usize {
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
pub fn expand_tabs(text: &str, tab_size: usize) -> String {
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

/// Diagnostic/interop: the `divide_line(text, width, fold=true)` break
/// offsets in char units (mirrors the Python binding's divide_line export).
pub fn divide_line_offsets(text: &str, width: usize) -> Vec<usize> {
    divide_line(text, width, true)
}

/// `rich.text.Text.wrap` equivalent: split on newlines, divide each line,
/// slice by offsets — returns the plain line strings.
pub fn wrap_plain(text: &str, width: usize) -> Vec<String> {    let mut out = Vec::new();
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
