//! Persistent data: directories, progress records and history management.
//!
//! Layouts and record formats mirror the Python implementation
//! (`lue/config.py`, `lue/progress_manager.py`) so both builds read and
//! write the same reading history interchangeably.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Same location as `platformdirs.user_data_dir("lue")`: on Windows the
/// author segment defaults to the app name, giving `<APPDATA>\lue\lue`.
pub fn data_root() -> PathBuf {
    let base = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    if cfg!(windows) {
        base.join("lue").join("lue")
    } else {
        base.join("lue")
    }
}

pub fn progress_dir() -> PathBuf {
    data_root()
}

/// Native-binary cache directory (TXT index cache). Kept separate from the
/// Python build's caches so neither build can misread the other's format.
pub fn cache_dir() -> PathBuf {
    data_root().join("cache")
}

/// `re.sub(r'[^0-9A-Za-z\u4e00-\u9fff_]+', '', title)` — CJK titles keep
/// their characters; an empty result falls back to `book` so the record
/// never becomes an invisible dotfile.
pub fn sanitize_title(title: &str) -> String {
    let mut safe = String::new();
    for c in title.chars() {
        if c.is_ascii_alphanumeric() || ('\u{4e00}'..='\u{9fff}').contains(&c) || c == '_' {
            safe.push(c);
        }
    }
    if safe.is_empty() {
        safe.push_str("book");
    }
    safe
}

pub fn progress_path(book_title: &str) -> PathBuf {
    progress_dir().join(format!("{}.progress.json", sanitize_title(book_title)))
}

/// Reading-state record, field-compatible with
/// `progress_manager.save_extended_progress`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressRecord {
    pub c: usize,
    pub p: usize,
    pub s: usize,
    #[serde(default)]
    pub scroll_offset: f64,
    #[serde(default = "default_true")]
    pub tts_enabled: bool,
    #[serde(default = "default_true")]
    pub auto_scroll_enabled: bool,
    #[serde(default)]
    pub speed_reading_enabled: bool,
    #[serde(default = "default_speed")]
    pub playback_speed: f64,
    #[serde(default)]
    pub completion_percentage: f64,
    /// `[chapter, paragraph, sentence]` of the topmost visible sentence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manual_scroll_anchor: Option<[i64; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_file_path: Option<String>,
}

fn default_true() -> bool {
    true
}

fn default_speed() -> f64 {
    1.0
}

impl Default for ProgressRecord {
    fn default() -> Self {
        Self {
            c: 0,
            p: 0,
            s: 0,
            scroll_offset: 0.0,
            tts_enabled: false,
            auto_scroll_enabled: false,
            speed_reading_enabled: false,
            playback_speed: 1.0,
            completion_percentage: 0.0,
            manual_scroll_anchor: None,
            original_file_path: None,
        }
    }
}

pub fn load_progress(path: &Path) -> ProgressRecord {
    let Ok(text) = fs::read_to_string(path) else {
        return ProgressRecord::default();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

/// Atomic write (tmp + rename), matching the Python implementation.
pub fn save_progress(path: &Path, record: &ProgressRecord) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_string_pretty(record)?)?;
    fs::rename(&tmp, path).with_context(|| format!("replace {}", path.display()))?;
    Ok(())
}

pub struct BookRecord {
    pub title: String,
    pub path: Option<PathBuf>,
    pub percentage: f64,
    pub modified: SystemTime,
}

/// Every `.progress.json` in the progress dir (dotfiles included), newest
/// first — mirrors `progress_manager.list_read_books`.
pub fn list_books() -> Vec<BookRecord> {
    let dir = progress_dir();
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut records = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if !path.file_name().and_then(|n| n.to_str()).unwrap_or("").ends_with(".progress.json") {
            continue;
        }
        let title = path
            .file_stem()
            .and_then(|n| n.to_str())
            .unwrap_or("book")
            .trim_end_matches(".progress")
            .to_string();
        let modified = entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        let data: serde_json::Value = fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or(serde_json::Value::Null);
        let book_path = data
            .get("original_file_path")
            .and_then(|v| v.as_str())
            .map(PathBuf::from);
        let percentage = data
            .get("completion_percentage")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        records.push(BookRecord {
            title,
            path: book_path,
            percentage,
            modified,
        });
    }
    records.sort_by_key(|r| std::cmp::Reverse(r.modified));
    records
}

pub fn most_recent_path() -> Option<PathBuf> {
    list_books()
        .into_iter()
        .find_map(|r| r.path.filter(|p| p.is_file()))
}

pub struct ClearResult {
    pub progress: usize,
    pub cache: usize,
    pub errors: Vec<(PathBuf, String)>,
}

/// Delete reading records and the native cache, reporting counts. Book
/// files, settings and audio caches are left untouched.
pub fn clear() -> Result<ClearResult> {
    let mut progress = 0usize;
    let mut errors = Vec::new();
    if let Ok(entries) = fs::read_dir(progress_dir()) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file()
                && path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .ends_with(".progress.json")
            {
                match fs::remove_file(&path) {
                    Ok(()) => progress += 1,
                    Err(e) => errors.push((path, e.to_string())),
                }
            }
        }
    }
    let mut cache = 0usize;
    remove_tree(&cache_dir(), &mut cache, &mut errors);
    Ok(ClearResult {
        progress,
        cache,
        errors,
    })
}

fn remove_tree(dir: &Path, count: &mut usize, errors: &mut Vec<(PathBuf, String)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            remove_tree(&path, count, errors);
        } else {
            match fs::remove_file(&path) {
                Ok(()) => *count += 1,
                Err(e) => errors.push((path, e.to_string())),
            }
        }
    }
    if fs::remove_dir(dir).is_ok() {
        // counted via its files only; the empty dir itself is not a datum
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_keeps_cjk_and_ascii() {
        assert_eq!(sanitize_title("魅力"), "魅力");
        assert_eq!(sanitize_title("Book: Vol.2"), "BookVol2");
        assert_eq!(sanitize_title("!!!"), "book");
        assert_eq!(sanitize_title(""), "book");
        assert_eq!(sanitize_title("a_b-c"), "a_bc");
    }

    #[test]
    fn record_json_roundtrip_matches_python_fields() {
        let rec = ProgressRecord {
            c: 3,
            p: 1,
            s: 0,
            scroll_offset: 12.0,
            tts_enabled: false,
            auto_scroll_enabled: false,
            speed_reading_enabled: false,
            playback_speed: 1.0,
            completion_percentage: 64.2,
            manual_scroll_anchor: Some([3, 1, 0]),
            original_file_path: Some("/books/魅力.txt".into()),
        };
        let json = serde_json::to_string(&rec).unwrap();
        assert!(json.contains("\"manual_scroll_anchor\":[3,1,0]"));
        assert!(json.contains("\"completion_percentage\":64.2"));
        let back: ProgressRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(back.c, 3);
        assert_eq!(back.manual_scroll_anchor, Some([3, 1, 0]));
    }
}
