//! Layout engine: paragraph wrapping with sentence->line indexing and
//! whole-window document layout (mirrors ui.update_document_layout).

use crate::text::split_sentences_impl;
use crate::wrap::wrap_plain;

/// Wrap a paragraph and compute the sentence->line mapping in one pass,
/// mirroring the two-pointer scan in ui.update_document_layout. Sentences
/// are split internally (no FFI round-trip of the sentence list). Returns
/// (wrapped_lines, sentence_line_indices, sentence_count) where
/// sentence_line_indices[i] is the wrapped-line index of sentence i (-1 if
/// it falls past the last line).
pub fn layout_paragraph(text: &str, width: usize) -> (Vec<String>, Vec<i64>, usize) {
    let sentences = split_sentences_impl(text);
    let sentence_count = sentences.len();
    let wrapped = wrap_plain(text, width);
    let mut line_idx = 0usize;
    let mut line_offset = 0usize;
    let mut current_char_pos = 0usize;
    let mut out = Vec::with_capacity(sentence_count);
    let plain_lens: Vec<usize> = wrapped.iter().map(|l| l.chars().count()).collect();
    for sentence in &sentences {
        let sentence_start = current_char_pos;
        while line_idx < plain_lens.len()
            && sentence_start >= line_offset + plain_lens[line_idx]
        {
            line_offset += plain_lens[line_idx];
            line_idx += 1;
        }
        if line_idx < plain_lens.len() {
            out.push(line_idx as i64);
        } else {
            out.push(-1);
        }
        current_char_pos = sentence_start + sentence.chars().count() + 1;
    }
    (wrapped, out, sentence_count)
}

/// Everything the reader needs about a laid-out window, in structured form
/// (the Python binding flattens these into parallel vectors).
#[derive(Debug, Default, Clone)]
pub struct DocumentLayout {
    pub lines: Vec<String>,
    /// (chapter, paragraph, sentence) -> document line index.
    pub position_to_line: Vec<((i64, i64, i64), usize)>,
    /// Dense per-line owner (chapter, paragraph); blank separator lines are
    /// None. Index = document line number.
    pub line_to_paragraph: Vec<Option<(i64, i64)>>,
    /// (chapter, paragraph) -> inclusive [start_line, end_line] range.
    pub paragraph_line_ranges: Vec<((i64, i64), (usize, usize))>,
    /// positions sorted by line (for "topmost visible sentence" lookups).
    pub lines_sorted_positions: Vec<((i64, i64, i64), usize)>,
    pub total_sentences: usize,
}

impl DocumentLayout {
    pub fn line_position(&self, line: usize) -> Option<(i64, i64, i64)> {
        self.line_to_paragraph
            .get(line)
            .and_then(|o| o.map(|(c, p)| (c, p, 0)))
    }

    /// Find the window line of an exact (c,p,s) position, if present.
    pub fn line_of(&self, pos: (i64, i64, i64)) -> Option<usize> {
        self.position_to_line
            .iter()
            .find(|(p, _)| *p == pos)
            .map(|(_, l)| *l)
    }
}

/// Whole-window layout in one pass: wraps every paragraph, builds the
/// sentence->line index, line->paragraph index, paragraph ranges and the
/// line-sorted position index.
///
/// Mirrors ui.update_document_layout's rebuild exactly: a blank separator
/// line between chapters and between paragraphs of the same chapter;
/// positions whose sentence start falls past the last wrapped line are
/// omitted; line_to_position maps every line to (chapter, para, 0).
pub fn layout_document(chapters: &[Vec<String>], width: usize) -> DocumentLayout {
    let mut doc = DocumentLayout::default();

    for (chap_idx, chapter) in chapters.iter().enumerate() {
        if chap_idx > 0 {
            doc.lines.push(String::new());
            doc.line_to_paragraph.push(None);
        }
        let chap = chap_idx as i64;
        for (para_idx, paragraph) in chapter.iter().enumerate() {
            let paragraph_start_line = doc.lines.len();
            let (wrapped, sentence_lines, sentence_count) =
                layout_paragraph(paragraph, width);
            doc.total_sentences += sentence_count;
            let para = para_idx as i64;
            doc.paragraph_line_ranges.push((
                (chap, para),
                (
                    paragraph_start_line,
                    paragraph_start_line + wrapped.len() - 1,
                ),
            ));
            for (sent_idx, line_in_para) in sentence_lines.iter().enumerate() {
                if *line_in_para >= 0 {
                    doc.position_to_line.push((
                        (chap, para, sent_idx as i64),
                        paragraph_start_line + *line_in_para as usize,
                    ));
                }
            }
            for _ in 0..wrapped.len() {
                doc.line_to_paragraph.push(Some((chap, para)));
            }
            doc.lines.extend(wrapped);
            if para_idx < chapter.len() - 1 {
                doc.lines.push(String::new());
                doc.line_to_paragraph.push(None);
            }
        }
    }

    // line-sorted position index: sort (line, insertion_order) pairs. The
    // insertion order of position_to_line is (chapter, para, sentence)
    // ascending and lines are non-decreasing along it, so a stable sort by
    // line preserves the first-inserted-per-line semantics of the original
    // scan.
    let mut pairs: Vec<&((i64, i64, i64), usize)> = doc.position_to_line.iter().collect();
    pairs.sort_by_key(|&(_, line)| *line);
    doc.lines_sorted_positions =
        pairs.into_iter().cloned().collect();
    doc
}
