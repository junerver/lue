//! lue_core — pure-Rust document core for the lue reader.
//!
//! Extracted from the original PyO3 accelerator so both the Python binding
//! (`lue_rs`) and the native binary (`lue`) share one implementation. Every
//! function mirrors the observable behaviour of its Python counterpart in
//! `lue/content_parser.py` / `lue/ui.py`; parity is enforced from Python by
//! `tests/test_rust_parity.py` and `tests/test_lazy_index.py`.

pub mod book;
pub mod cell_table;
pub mod error;
pub mod layout;
pub mod text;
pub mod toc;
pub mod wrap;

pub use book::{build_txt_index, build_txt_index_lenient, source_fingerprint, TxtBook, TxtIndex};
pub use error::LueError;
pub use layout::{layout_document, layout_paragraph, DocumentLayout};
pub use text::{
    analyze_cjk_impl, clean_txt_doc_lines_impl, clean_visual_text_impl, py_trim,
    split_long_text_paragraphs_impl, split_sentences_batch_impl, split_sentences_impl,
};
pub use toc::{collect_title_lines_impl, pick_toc_rule_impl};
pub use wrap::{cell_len, char_cell_width, divide_line_offsets, expand_tabs, wrap_plain};

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
        assert_eq!(clean_visual_text_impl("__CODE_BLOCK__    x = 1"), "    x = 1");
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
    fn long_text_paragraphs() {
        let text = "句子。".repeat(100);
        let paras = split_long_text_paragraphs_impl(&text, 200);
        assert!(!paras.is_empty());
        assert!(paras.iter().all(|p| p.chars().count() >= 200 || p == paras.last().unwrap()));
    }

    #[test]
    fn wrap_cjk_pads_to_width() {
        // 6 cells: three CJK chars per line; the final short line is padded.
        let lines = wrap_plain("你好世界测试", 6);
        assert_eq!(lines, vec!["你好世", "界测试"]);
    }

    #[test]
    fn line_of_finds_exact_position() {
        let chapters = vec![
            vec!["第一章".to_string(), "内容甲。".to_string()],
            vec!["第二章".to_string(), "内容乙。".to_string()],
        ];
        let doc = layout_document(&chapters, 40);
        let (c0p0, _) = doc.position_to_line[0];
        assert_eq!(doc.line_of(c0p0), Some(doc.position_to_line[0].1));
        assert_eq!(doc.line_of((99, 0, 0)), None);
    }

    #[test]
    fn layout_window_maps_positions() {
        let chapters = vec![
            vec!["第一章 标题甲".to_string(), "第一段。第二段。".to_string()],
            vec!["第二章 标题乙".to_string(), "另一段。".to_string()],
        ];
        let doc = layout_document(&chapters, 40);
        // chapter separators + paragraph separators are None rows
        assert_eq!(doc.line_to_paragraph.len(), doc.lines.len());
        assert!(doc
            .line_to_paragraph
            .iter()
            .zip(doc.lines.iter())
            .all(|(owner, line)| match owner {
                Some(_) => !line.is_empty(),
                None => line.is_empty(),
            }));
        // sorted positions are line-ascending
        let lines: Vec<usize> = doc.lines_sorted_positions.iter().map(|(_, l)| *l).collect();
        let mut sorted = lines.clone();
        sorted.sort_unstable();
        assert_eq!(lines, sorted);
    }
}
