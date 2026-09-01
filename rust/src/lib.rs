//! lue_rs — PyO3 binding over the pure-Rust core (`lue_core`).
//!
//! This crate is now a thin compatibility layer: every exported function
//! and class forwards to `lue_core`, keeping the wheel surface used by the
//! Python package unchanged (see `lue/_rust.py` and `tests/test_lazy_index.py`).

use lue_core as core;
use pyo3::exceptions::{PyIndexError, PyOSError, PyUnicodeDecodeError, PyValueError};
use pyo3::prelude::*;

fn map_err(e: core::LueError) -> PyErr {
    match e {
        core::LueError::Io(err) => PyOSError::new_err(err.to_string()),
        core::LueError::Decode(msg) => PyUnicodeDecodeError::new_err(msg),
        core::LueError::Invalid(msg) => PyValueError::new_err(msg),
    }
}

// ---------------------------------------------------------------------------
// Text pipeline
// ---------------------------------------------------------------------------

#[pyfunction]
fn split_sentences(paragraph: &str) -> Vec<String> {
    core::split_sentences_impl(paragraph)
}

#[pyfunction]
fn split_sentences_batch(paragraphs: Vec<String>) -> Vec<Vec<String>> {
    let refs: Vec<&str> = paragraphs.iter().map(String::as_str).collect();
    core::split_sentences_batch_impl(&refs)
}

#[pyfunction]
fn clean_visual_text(text: &str) -> String {
    core::clean_visual_text_impl(text)
}

#[pyfunction]
fn clean_txt_doc_lines(content: &str) -> Vec<(usize, String)> {
    core::clean_txt_doc_lines_impl(content)
}

#[pyfunction]
fn analyze_cjk(content: &str) -> (usize, usize) {
    core::analyze_cjk_impl(content)
}

#[pyfunction]
fn pick_toc_rule(sample: &str) -> Option<usize> {
    core::pick_toc_rule_impl(sample)
}

#[pyfunction]
fn collect_title_lines(content: &str, rule_idx: usize) -> Vec<u64> {
    core::collect_title_lines_impl(content, rule_idx)
}

#[pyfunction]
fn split_long_text_paragraphs(text: &str, max_chars: usize) -> Vec<String> {
    core::split_long_text_paragraphs_impl(text, max_chars)
}

#[pyfunction]
fn wrap_paragraph(text: &str, width: usize) -> Vec<String> {
    core::wrap_plain(text, width)
}

#[pyfunction]
fn layout_paragraph(text: &str, width: usize) -> (Vec<String>, Vec<i64>, usize) {
    core::layout_paragraph(text, width)
}

/// Diagnostic: char cell width, same as rich.cells.get_character_cell_size.
#[pyfunction]
fn char_cell_width_py(c: String) -> usize {
    match c.chars().next() {
        Some(ch) => core::char_cell_width(ch),
        None => 0,
    }
}

/// Diagnostic: cell_len(text) same as rich.cells.cell_len.
#[pyfunction]
fn cell_len_py(text: &str) -> usize {
    core::cell_len(text)
}

/// Diagnostic: expand_tabs(text, size) same as rich Text.expand_tabs.
#[pyfunction]
fn expand_tabs_py(text: &str, tab_size: usize) -> String {
    core::expand_tabs(text, tab_size)
}

/// Diagnostic: divide_line offsets (char indices), same as rich._wrap.divide_line.
#[pyfunction]
fn divide_line_py(text: &str, width: usize) -> Vec<usize> {
    let expanded = core::expand_tabs(text, 8);
    core::divide_line_offsets(&expanded, width)
}

/// TXT random-access index: `(encoding, intro_first, starts, ends)` — byte
/// offsets into the raw file. `intro_first` marks the leading preface
/// chapter. Returns `encoding = ""` when the book does not match the
/// lazy-eligible profile; callers fall back to the full in-memory parse.
#[pyfunction]
fn build_txt_index(path: &str) -> (String, bool, Vec<u64>, Vec<u64>) {
    match core::build_txt_index(path) {
        Ok(Some(idx)) => (idx.encoding, idx.intro_first, idx.starts, idx.ends),
        _ => (String::new(), false, Vec::new(), Vec::new()),
    }
}

// ---------------------------------------------------------------------------
// Document layout (flat parallel vectors for dict(zip(...)))
// ---------------------------------------------------------------------------

#[allow(clippy::type_complexity)]
fn flatten_layout(doc: core::DocumentLayout) -> (
    Vec<String>,
    Vec<(i64, i64, i64)>,
    Vec<i64>,
    Vec<i64>,
    Vec<(i64, i64, i64)>,
    Vec<(i64, i64)>,
    Vec<(i64, i64)>,
    Vec<(i64, i64, i64)>,
    Vec<i64>,
    usize,
) {
    let mut pos_keys = Vec::with_capacity(doc.position_to_line.len());
    let mut pos_vals = Vec::with_capacity(doc.position_to_line.len());
    for (pos, line) in doc.position_to_line {
        pos_keys.push(pos);
        pos_vals.push(line as i64);
    }
    let mut l2p_keys = Vec::with_capacity(doc.line_to_paragraph.len());
    let mut l2p_vals = Vec::with_capacity(doc.line_to_paragraph.len());
    for (line, para) in doc.line_to_paragraph.iter().enumerate() {
        if let Some((c, p)) = para {
            l2p_keys.push(line as i64);
            l2p_vals.push((*c, *p, 0));
        }
    }
    let mut range_keys = Vec::with_capacity(doc.paragraph_line_ranges.len());
    let mut range_vals = Vec::with_capacity(doc.paragraph_line_ranges.len());
    for (key, (start, end)) in doc.paragraph_line_ranges {
        range_keys.push(key);
        range_vals.push((start as i64, end as i64));
    }
    let mut sorted_positions = Vec::with_capacity(doc.lines_sorted_positions.len());
    let mut sorted_lines = Vec::with_capacity(doc.lines_sorted_positions.len());
    for (pos, line) in doc.lines_sorted_positions {
        sorted_positions.push(pos);
        sorted_lines.push(line as i64);
    }
    (
        doc.lines,
        pos_keys,
        pos_vals,
        l2p_keys,
        l2p_vals,
        range_keys,
        range_vals,
        sorted_positions,
        sorted_lines,
        doc.total_sentences,
    )
}

/// Whole-document layout in one Rust call. Mirrors
/// ui.update_document_layout's rebuild exactly; see `lue_core::layout_document`.
#[pyfunction]
#[allow(clippy::type_complexity)]
fn layout_document(
    chapters: Vec<Vec<String>>,
    width: usize,
) -> (
    Vec<String>,
    Vec<(i64, i64, i64)>,
    Vec<i64>,
    Vec<i64>,
    Vec<(i64, i64, i64)>,
    Vec<(i64, i64)>,
    Vec<(i64, i64)>,
    Vec<(i64, i64, i64)>,
    Vec<i64>,
    usize,
) {
    flatten_layout(core::layout_document(&chapters, width))
}

// ---------------------------------------------------------------------------
// IndexedTxtBook (thin wrapper over lue_core::TxtBook)
// ---------------------------------------------------------------------------

#[pyclass]
struct IndexedTxtBook {
    book: core::TxtBook,
}

#[pymethods]
impl IndexedTxtBook {
    #[staticmethod]
    fn from_index(
        path: String,
        encoding: String,
        starts: Vec<u64>,
        ends: Vec<u64>,
        intro_first: bool,
    ) -> PyResult<Self> {
        let book = core::TxtBook::from_index(path, encoding, starts, ends, intro_first)
            .map_err(map_err)?;
        Ok(Self { book })
    }

    fn chapter_count(&self) -> usize {
        self.book.chapter_count()
    }

    fn source_fingerprint(&self) -> (u64, u128) {
        let fp = self.book.fingerprint();
        (fp.size, fp.mtime_ns)
    }

    fn read_chapter(&mut self, index: isize) -> PyResult<Vec<String>> {
        let count = self.book.chapter_count() as isize;
        let normalized = if index < 0 { count + index } else { index };
        if normalized < 0 || normalized >= count {
            return Err(PyIndexError::new_err(index));
        }
        self.book
            .read_chapter(normalized as usize)
            .map_err(map_err)
    }

    #[allow(clippy::type_complexity)]
    fn layout_window(
        &mut self,
        center: usize,
        before: usize,
        after: usize,
        width: usize,
    ) -> PyResult<(
        Vec<String>,
        Vec<(i64, i64, i64)>,
        Vec<i64>,
        Vec<i64>,
        Vec<(i64, i64, i64)>,
        Vec<(i64, i64)>,
        Vec<(i64, i64)>,
        Vec<(i64, i64, i64)>,
        Vec<i64>,
        usize,
    )> {
        if center >= self.book.chapter_count() {
            return Err(PyIndexError::new_err(center));
        }
        let doc = self
            .book
            .layout_window(center, before, after, width)
            .map_err(map_err)?;
        Ok(flatten_layout(doc))
    }
}

// ---------------------------------------------------------------------------
// Python module
// ---------------------------------------------------------------------------

#[pymodule]
fn lue_rs(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<IndexedTxtBook>()?;
    m.add_function(wrap_pyfunction!(split_sentences, m)?)?;
    m.add_function(wrap_pyfunction!(split_sentences_batch, m)?)?;
    m.add_function(wrap_pyfunction!(clean_visual_text, m)?)?;
    m.add_function(wrap_pyfunction!(clean_txt_doc_lines, m)?)?;
    m.add_function(wrap_pyfunction!(analyze_cjk, m)?)?;
    m.add_function(wrap_pyfunction!(pick_toc_rule, m)?)?;
    m.add_function(wrap_pyfunction!(collect_title_lines, m)?)?;
    m.add_function(wrap_pyfunction!(split_long_text_paragraphs, m)?)?;
    m.add_function(wrap_pyfunction!(wrap_paragraph, m)?)?;
    m.add_function(wrap_pyfunction!(layout_paragraph, m)?)?;
    m.add_function(wrap_pyfunction!(layout_document, m)?)?;
    m.add_function(wrap_pyfunction!(build_txt_index, m)?)?;
    m.add_function(wrap_pyfunction!(char_cell_width_py, m)?)?;
    m.add_function(wrap_pyfunction!(cell_len_py, m)?)?;
    m.add_function(wrap_pyfunction!(expand_tabs_py, m)?)?;
    m.add_function(wrap_pyfunction!(divide_line_py, m)?)?;
    m.add("__version__", "0.1.0")?;
    Ok(())
}
