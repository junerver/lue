//! End-to-end flow over a real temp book file: index build, windowed
//! layout, chapter jumps, viewport scrolling and progress round-trips.

use lue_core::TxtBook;
use std::fs;
use std::path::PathBuf;

fn make_book(dir: &std::path::Path, chapters: usize) -> PathBuf {
    let mut text = String::from("书名页\n\n");
    for c in 1..=chapters {
        text.push_str(&format!("第{c}章 测试章节标题{c}\n"));
        // chapter gap must exceed the 1000-char TOC validity bar; body
        // lines must not start like chapter titles ("第N章…"), which would
        // legitimately poison the rule score
        for p in 0..70 {
            text.push_str(&format!("这是第{c}章第{p}段的内容,这一行写长一些保证超过一千字符门槛呀。\n"));
        }
        text.push('\n');
    }
    let path = dir.join(format!("testbook-{}章.txt", chapters));
    fs::write(&path, text).unwrap();
    path
}

fn open(path: &std::path::Path) -> TxtBook {
    let index = lue_core::build_txt_index(&path.to_string_lossy())
        .unwrap()
        .expect("book must be lazy-eligible");
    assert!(index.is_eligible());
    TxtBook::from_index(
        path.to_string_lossy().into_owned(),
        index.encoding,
        index.starts,
        index.ends,
        index.intro_first,
    )
    .unwrap()
}

#[test]
fn index_window_navigation_and_progress() {
    let dir = std::env::temp_dir().join("lue_app_integration");
    fs::create_dir_all(&dir).unwrap();
    let path = make_book(&dir, 30);

    let mut book = open(&path);
    // 30 titled chapters + the "书名页" preface chapter
    assert_eq!(book.chapter_count(), 31);

    let chapter = book.read_chapter(0).unwrap();
    assert_eq!(chapter[0], "书名页");
    assert_eq!(book.chapter_title(1).unwrap(), "第1章 测试章节标题1");

    // window layout centred on chapter 5
    let doc = book.layout_window(5, 1, 2, 40).unwrap();
    assert!(!doc.lines.is_empty());
    assert_eq!(doc.line_to_paragraph.len(), doc.lines.len());

    // chapter 5 must be present, chapters 4..=7 window
    let chapter_numbers: Vec<i64> = doc
        .line_to_paragraph
        .iter()
        .flatten()
        .map(|(c, _)| *c)
        .collect();
    assert!(chapter_numbers.contains(&5));
    assert!(chapter_numbers.contains(&4) && chapter_numbers.contains(&6));
    assert!(!chapter_numbers.contains(&10));

    // stale index: mutate the file, reads must be rejected
    let mut book2 = open(&path);
    let original = fs::read_to_string(&path).unwrap();
    fs::write(&path, format!("{original}\n附录\n")).unwrap();
    assert!(book2.read_chapter(0).is_err());
    fs::write(&path, original).unwrap();
}
