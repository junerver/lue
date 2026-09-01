//! Diagnostics: print the lazy-index gate values for a TXT file.
//! Usage: cargo run -p lue_core --example probe -- path/to/book.txt

use lue_core::{analyze_cjk_impl, build_txt_index, pick_toc_rule_impl};

fn main() {
    let path = std::env::args().nth(1).expect("book path");
    let started = std::time::Instant::now();
    let bytes = std::fs::read(&path).expect("read book");
    let read_done = started.elapsed();

    let content = String::from_utf8_lossy(&bytes)
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let raw_lines: Vec<&str> = content.split('\n').collect();
    let (cjk_count, cjk_total) = analyze_cjk_impl(&content);
    let sample_chars: usize = content
        .char_indices()
        .map(|(i, _)| i)
        .find(|&i| i > 500 * 1024)
        .unwrap_or(content.len());
    let rule = pick_toc_rule_impl(&content[..sample_chars]);
    let scanned = started.elapsed();

    let index = build_txt_index(&path);
    println!(
        "file: {path}\nbytes={} lines={} nonempty={} cjk_lines={cjk_count} cjk_avg={:.1} rule={rule:?}",
        bytes.len(),
        raw_lines.len(),
        raw_lines.iter().filter(|l| !l.trim().is_empty()).count(),
        cjk_total as f64 / cjk_count.max(1) as f64,
    );
    match index {
        Ok(Some(idx)) => println!(
            "eligible: encoding={} chapters={} preface={} (read {read_done:?}, scan+index {:?})",
            idx.encoding,
            idx.starts.len(),
            idx.intro_first,
            scanned
        ),
        Ok(None) => println!("eligible: NO (book profile not lazily indexable)"),
        Err(e) => println!("index error: {e}"),
    }
}
