//! Opening books for the native reader: TXT index discovery with a local
//! disk cache (native JSON format, independent of the Python build's
//! pickled caches).

use anyhow::{bail, Context, Result};
use lue_core::{source_fingerprint, TxtBook, TxtIndex};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use crate::store;

pub struct IndexedBook {
    pub book: TxtBook,
    pub path: PathBuf,
    pub title: String,
    /// Chapter titles restored from the index cache, so the `c` menu opens
    /// instantly on warm starts. `None` lets the reader warm them in the
    /// background (and write them back through [`cache_titles`]).
    pub titles: Option<Vec<String>>,
}

#[derive(Serialize, Deserialize)]
struct CachedIndex {
    encoding: String,
    intro_first: bool,
    starts: Vec<u64>,
    ends: Vec<u64>,
    size: u64,
    mtime_ns: u128,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    titles: Option<Vec<String>>,
}

fn cache_file(path: &Path) -> Result<PathBuf> {
    let fp = source_fingerprint(&path.to_string_lossy())?;
    let stem = path
        .file_stem()
        .and_then(|n| n.to_str())
        .unwrap_or("book")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();
    let dir = store::cache_dir().join("txt-index");
    fs::create_dir_all(&dir)?;
    Ok(dir.join(format!("{stem}-{}.json", fp.size)))
}

fn load_cached_index(path: &Path) -> Option<(TxtIndex, Option<Vec<String>>)> {
    let cache = cache_file(path).ok()?;
    let text = fs::read_to_string(cache).ok()?;
    let cached: CachedIndex = serde_json::from_str(&text).ok()?;
    let fp = source_fingerprint(&path.to_string_lossy()).ok()?;
    if fp.size != cached.size || fp.mtime_ns != cached.mtime_ns {
        return None;
    }
    Some((
        TxtIndex {
            encoding: cached.encoding,
            intro_first: cached.intro_first,
            starts: cached.starts,
            ends: cached.ends,
        },
        cached.titles,
    ))
}

fn store_cached_index(path: &Path, index: &TxtIndex) {
    let Ok(fp) = source_fingerprint(&path.to_string_lossy()) else {
        return;
    };
    let Ok(cache) = cache_file(path) else {
        return;
    };
    let payload = CachedIndex {
        encoding: index.encoding.clone(),
        intro_first: index.intro_first,
        starts: index.starts.clone(),
        ends: index.ends.clone(),
        size: fp.size,
        mtime_ns: fp.mtime_ns,
        titles: None,
    };
    let tmp = cache.with_extension("tmp");
    if fs::write(&tmp, serde_json::to_vec(&payload).unwrap_or_default()).is_ok() {
        let _ = fs::rename(&tmp, &cache);
    }
}

/// Merge freshly warmed chapter titles into the on-disk index cache so the
/// next open of this book skips the warm-up entirely. Best-effort: a stale
/// fingerprint (the book changed mid-warm-up) or a missing cache file just
/// skips the write.
pub fn cache_titles(path: &Path, titles: &[String]) {
    let Ok(cache) = cache_file(path) else {
        return;
    };
    let Ok(text) = fs::read_to_string(&cache) else {
        return;
    };
    let Ok(mut cached) = serde_json::from_str::<CachedIndex>(&text) else {
        return;
    };
    let Ok(fp) = source_fingerprint(&path.to_string_lossy()) else {
        return;
    };
    if fp.size != cached.size || fp.mtime_ns != cached.mtime_ns {
        return;
    }
    cached.titles = Some(titles.to_vec());
    let tmp = cache.with_extension("tmp");
    if fs::write(&tmp, serde_json::to_vec(&cached).unwrap_or_default()).is_ok() {
        let _ = fs::rename(&tmp, &cache);
    }
}

/// Open a book by extension: TXT through its byte-level chapter index;
/// EPUB/DOCX/HTML/MD/RTF through extraction into a cached normalized text
/// file. Only PDF stays Python-side (no Rust text-layer parity yet).
pub fn open(path: &Path) -> Result<IndexedBook> {
    if !path.is_file() {
        bail!("file not found: {}", path.display());
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    let extraction = match ext.as_str() {
        "epub" => Some((
            "EPUB",
            lue_core::extract_epub_docs(path) as Result<Vec<String>, _>,
        )),
        "docx" => Some((
            "DOCX",
            lue_core::extract_docx_docs(path) as Result<Vec<String>, _>,
        )),
        "html" => Some((
            "HTML",
            lue_core::extract_html_docs(path) as Result<Vec<String>, _>,
        )),
        "md" => Some((
            "Markdown",
            lue_core::extract_markdown_docs(path) as Result<Vec<String>, _>,
        )),
        "rtf" => Some((
            "RTF",
            lue_core::extract_rtf_docs(path) as Result<Vec<String>, _>,
        )),
        "pdf" => Some((
            "PDF",
            lue_core::extract_pdf_docs(path) as Result<Vec<String>, _>,
        )),
        "txt" | "" => return open_txt(path),
        _ => bail!(
            "unsupported file type '.{ext}'; the native binary opens \
             TXT/EPUB/DOCX/HTML/MD/RTF/PDF books"
        ),
    };
    let (format, docs) = extraction.unwrap();
    finish_extraction(path, format, docs)
}

fn finish(
    path: &Path,
    text_path: &str,
    index: TxtIndex,
    titles: Option<Vec<String>>,
) -> Result<IndexedBook> {
    let book = TxtBook::from_index(
        text_path.to_string(),
        index.encoding,
        index.starts,
        index.ends,
        index.intro_first,
    )
    .with_context(|| format!("opening {}", path.display()))?;
    let title = path
        .file_stem()
        .and_then(|n| n.to_str())
        .unwrap_or("book")
        .to_string();
    let titles = titles.filter(|t| t.len() == book.chapter_count());
    Ok(IndexedBook {
        book,
        path: path.to_path_buf(),
        title,
        titles,
    })
}

/// Open a TXT book by building or loading its chapter index. Books outside
/// the lazy-eligible profile (no chapter rules, non-Chinese layout, wall of
/// text…) fall back to whole-book size chunks so untitled TXT still opens.
fn open_txt(path: &Path) -> Result<IndexedBook> {
    let path_str = path.to_string_lossy().into_owned();
    let (index, titles) = match load_cached_index(path) {
        Some((idx, titles)) if idx.is_eligible() => (idx, titles),
        _ => {
            let built = lue_core::build_txt_index(&path_str)
                .map_err(|e| anyhow::anyhow!("indexing failed: {e}"))?;
            let index = match built {
                Some(index) => index,
                None => lue_core::build_whole_book_index(&path_str)
                    .map_err(|e| anyhow::anyhow!("indexing failed: {e}"))?
                    .ok_or_else(|| {
                        anyhow::anyhow!("no text content found in {}", path.display())
                    })?,
            };
            store_cached_index(path, &index);
            (index, None)
        }
    };
    finish(path, &path_str, index, titles)
}

/// Materialize extracted chapters (one blob per chapter, lines joined with
/// `\n`) into a cached normalized text file and index that file directly —
/// extraction already knows the chapter boundaries, so no title rules are
/// involved. Shared by every extracted format (EPUB/DOCX/HTML/MD/RTF).
fn finish_extraction(
    path: &Path,
    format: &str,
    docs: Result<Vec<String>, lue_core::LueError>,
) -> Result<IndexedBook> {
    let docs =
        docs.map_err(|e| anyhow::anyhow!("{format} extraction failed: {e}"))?;
    if docs.is_empty() || docs.iter().all(String::is_empty) {
        bail!("no readable content found in {}", path.display());
    }
    let fp = lue_core::source_fingerprint(&path.to_string_lossy())?;
    let extracted = extracted_file(path, fp)?;
    if let Some((index, titles)) = load_cached_index(path) {
        if index.is_eligible() && extracted.is_file() {
            return finish(path, &extracted.to_string_lossy(), index, titles);
        }
    }
    let mut text = String::new();
    let mut starts: Vec<u64> = Vec::with_capacity(docs.len());
    for doc in &docs {
        starts.push(text.len() as u64);
        text.push_str(doc);
        text.push('\n');
    }
    let dir = extracted.parent().expect("extracted file has a parent");
    fs::create_dir_all(dir)?;
    fs::write(&extracted, &text)?;
    prune_old_extractions(dir, &extracted);
    let ends: Vec<u64> = starts
        .iter()
        .skip(1)
        .copied()
        .chain(std::iter::once(text.len() as u64))
        .collect();
    let index = TxtIndex {
        encoding: "utf-8".to_string(),
        intro_first: false,
        starts,
        ends,
    };
    store_cached_index(path, &index);
    finish(path, &extracted.to_string_lossy(), index, None)
}

/// Cache file for an EPUB's extracted text: `<stem>-<path-hash>-<size>-
/// <mtime>.txt`. The path hash isolates same-stem books (CJK names sanitize
/// to the same stem) and the source fingerprint makes a changed book
/// re-extract instead of serving stale content.
fn extracted_file(path: &Path, fp: lue_core::SourceFingerprint) -> Result<PathBuf> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let stem: String = path
        .file_stem()
        .and_then(|n| n.to_str())
        .unwrap_or("book")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    Ok(store::cache_dir().join("extracted").join(format!(
        "{stem}-{:016x}-{}-{}.txt",
        hasher.finish(),
        fp.size,
        fp.mtime_ns
    )))
}

/// Drop other extractions of the same book (older fingerprints). Extraction
/// is derived state, so removing stale files is always safe.
fn prune_old_extractions(dir: &Path, keep: &Path) {
    let Some(keep_name) = keep.file_name().and_then(|n| n.to_str()) else {
        return;
    };
    // keep_name = "<stem>-<path-hash>-<size>-<mtime>.txt"; the path hash
    // alone identifies this book's extraction family.
    let mut parts = keep_name.rsplit('-');
    let _mtime = parts.next();
    let _size = parts.next();
    let Some(hash) = parts.next() else {
        return;
    };
    let marker = format!("-{hash}-");
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if name != keep_name && name.contains(&marker) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod open_tests {
    use super::*;
    use std::io::Write;

    fn write_fixture_epub(path: &Path) {
        let file = std::fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options: zip::write::SimpleFileOptions = Default::default();
        zip.start_file("META-INF/container.xml", options).unwrap();
        zip.write_all(
            br#"<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#,
        )
        .unwrap();
        zip.start_file("OEBPS/content.opf", options).unwrap();
        zip.write_all(
            br#"<package xmlns="http://www.idpf.org/2007/opf"><manifest><item id="c1" href="ch1.xhtml" media-type="application/xhtml+xml"/><item id="c2" href="ch2.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="c1"/><itemref idref="c2"/></spine></package>"#,
        )
        .unwrap();
        zip.start_file("OEBPS/ch1.xhtml", options).unwrap();
        zip.write_all(
            "<html><body><h2>第一章 风起</h2><p>山里的雾还没散。</p></body></html>".as_bytes(),
        )
        .unwrap();
        zip.start_file("OEBPS/ch2.xhtml", options).unwrap();
        zip.write_all(
            "<html><body><h2>第二章 云涌</h2><p>事情开始起变化。</p></body></html>".as_bytes(),
        )
        .unwrap();
        zip.finish().unwrap();
    }

    /// Tests share the real cache dir; isolate per-book names via the temp
    /// path so these never collide with a user's actual cached books.
    #[test]
    fn epub_opens_with_one_chapter_per_spine_document() {
        let dir = std::env::temp_dir().join("lue_app_epub_open");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("fixture-novel.epub");
        write_fixture_epub(&path);

        let mut book = open(&path).unwrap();
        assert_eq!(book.book.chapter_count(), 2);
        assert_eq!(book.title, "fixture-novel");
        assert_eq!(
            book.book.collect_titles().unwrap(),
            vec!["第一章 风起", "第二章 云涌"]
        );
        // Second open takes the cached-index + extracted-file path.
        let warm = open(&path).unwrap();
        assert_eq!(warm.book.chapter_count(), 2);
    }

    #[test]
    fn untitled_txt_falls_back_to_size_chunks() {
        let dir = std::env::temp_dir().join("lue_app_txt_fallback");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("untitled-wall.txt");
        let line = "这一行正文没有章节标题用来验证兜底分块逻辑呀。".repeat(1);
        let per_chunk = lue_core::TOC_FALLBACK_CHAPTER_CHARS / line.chars().count();
        let mut text = String::new();
        for i in 0..(per_chunk * 3 + 7) {
            text.push_str(&line);
            text.push_str(&format!("行{i}\n"));
        }
        fs::write(&path, &text).unwrap();

        let mut book = open(&path).unwrap();
        assert!(book.book.chapter_count() >= 3, "chunks: {}", book.book.chapter_count());
        book.book.read_chapter(book.book.chapter_count() - 1).unwrap();
    }
}
