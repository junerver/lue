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
}

#[derive(Serialize, Deserialize)]
struct CachedIndex {
    encoding: String,
    intro_first: bool,
    starts: Vec<u64>,
    ends: Vec<u64>,
    size: u64,
    mtime_ns: u128,
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

fn load_cached_index(path: &Path) -> Option<TxtIndex> {
    let cache = cache_file(path).ok()?;
    let text = fs::read_to_string(cache).ok()?;
    let cached: CachedIndex = serde_json::from_str(&text).ok()?;
    let fp = source_fingerprint(&path.to_string_lossy()).ok()?;
    if fp.size != cached.size || fp.mtime_ns != cached.mtime_ns {
        return None;
    }
    Some(TxtIndex {
        encoding: cached.encoding,
        intro_first: cached.intro_first,
        starts: cached.starts,
        ends: cached.ends,
    })
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
    };
    let tmp = cache.with_extension("tmp");
    if fs::write(&tmp, serde_json::to_vec(&payload).unwrap_or_default()).is_ok() {
        let _ = fs::rename(&tmp, &cache);
    }
}

/// Open a TXT book by building or loading its chapter index. Books outside
/// the lazy-eligible profile (non-TXT, non-Chinese layout, no chapter
/// rules) are rejected with a user-facing message.
pub fn open(path: &Path) -> Result<IndexedBook> {
    if !path.is_file() {
        bail!("file not found: {}", path.display());
    }
    if path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| !e.eq_ignore_ascii_case("txt"))
        .unwrap_or(false)
    {
        bail!(
            "the native binary currently opens TXT books only; \
             use the Python build for EPUB/PDF/DOCX/HTML/RTF/MD"
        );
    }
    let path_str = path.to_string_lossy().into_owned();
    let index = match load_cached_index(path) {
        Some(idx) if idx.is_eligible() => idx,
        _ => {
            let built = lue_core::build_txt_index(&path_str)
                .map_err(|e| anyhow::anyhow!("indexing failed: {e}"))?;
            let Some(index) = built else {
                bail!(
                    "no chapter structure detected in {}; \
                     the native binary needs titled Chinese TXT books",
                    path.display()
                );
            };
            store_cached_index(path, &index);
            index
        }
    };
    let book = TxtBook::from_index(
        path_str.clone(),
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
    Ok(IndexedBook {
        book,
        path: path.to_path_buf(),
        title,
    })
}
