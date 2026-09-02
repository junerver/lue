//! Keyboard mappings. Layouts are parsed from the same JSON files the
//! Python build ships (`keys_default.json` / `keys_vim.json`), so both
//! builds share one definition of every action; TTS start/pause are wired
//! natively, highlight/speed keys stay `Action::Ignored`.

use anyhow::{bail, Result};
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    NextParagraph,
    PrevParagraph,
    NextLine,
    PrevLine,
    PageUp,
    PageDown,
    ScrollUp,
    ScrollDown,
    NextChapter,
    PrevChapter,
    MoveToBeginning,
    MoveToEnd,
    CycleUiComplexity,
    ToggleChapterIndex,
    ToggleRecentMenu,
    SelectMenuItem,
    TtsToggleRead,
    TtsPlayPause,
    Quit,
    Ignored,
}

/// Layout values are a single key (`"p"`) or a list of keys
/// (`["p", " "]`, JSON sequence).
#[derive(Deserialize)]
#[serde(untagged)]
enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

#[derive(Deserialize)]
struct KeyLayoutFile {
    navigation: HashMap<String, OneOrMany>,
    #[serde(default)]
    tts_controls: HashMap<String, OneOrMany>,
    #[serde(default)]
    display_controls: HashMap<String, OneOrMany>,
    application: HashMap<String, OneOrMany>,
}

const DEFAULT_LAYOUT: &str = include_str!("assets/keys_default.json");
const VIM_LAYOUT: &str = include_str!("assets/keys_vim.json");

pub struct KeyMap {
    bindings: HashMap<String, Action>,
}

pub fn load(layout: &str) -> Result<KeyMap> {
    let text = match layout {
        "default" => DEFAULT_LAYOUT,
        "vim" => VIM_LAYOUT,
        other => bail!("unknown keyboard layout '{other}' (expected default|vim)"),
    };
    let parsed: KeyLayoutFile = serde_json::from_str(text)?;

    let nav: &[(&str, Action)] = &[
        ("next_paragraph", Action::NextParagraph),
        ("prev_paragraph", Action::PrevParagraph),
        ("next_sentence", Action::NextLine),
        ("prev_sentence", Action::PrevLine),
        ("scroll_page_up", Action::PageUp),
        ("scroll_page_down", Action::PageDown),
        ("scroll_up", Action::ScrollUp),
        ("scroll_down", Action::ScrollDown),
        ("move_to_top_visible", Action::Ignored),
        ("next_chapter", Action::NextChapter),
        ("prev_chapter", Action::PrevChapter),
        ("move_to_beginning", Action::MoveToBeginning),
        ("move_to_end", Action::MoveToEnd),
    ];
    let display: &[(&str, Action)] = &[
        ("toggle_auto_scroll", Action::TtsToggleRead),
        ("cycle_ui_complexity", Action::CycleUiComplexity),
        ("toggle_chapter_index", Action::ToggleChapterIndex),
    ];
    let tts: &[(&str, Action)] = &[
        ("play_pause", Action::TtsPlayPause),
        ("decrease_speed", Action::Ignored),
        ("increase_speed", Action::Ignored),
        ("toggle_sentence_highlight", Action::Ignored),
        ("toggle_word_highlight", Action::Ignored),
    ];
    let app: &[(&str, Action)] = &[
        ("quit", Action::Quit),
        ("toggle_recent_menu", Action::ToggleRecentMenu),
        ("select_menu_item", Action::SelectMenuItem),
    ];

    let mut bindings = HashMap::new();
    let mut insert = |table: &HashMap<String, OneOrMany>, spec: &[(&str, Action)]| {
        for (name, action) in spec {
            if let Some(value) = table.get(*name) {
                let keys = match value {
                    OneOrMany::One(key) => vec![key.clone()],
                    OneOrMany::Many(keys) => keys.clone(),
                };
                for key in keys {
                    // JSON encodes Enter as "\n", which must not be trimmed
                    if key == "\n" {
                        bindings.insert("enter".to_string(), action.clone());
                        continue;
                    }
                    let key = key.trim();
                    if key.is_empty() {
                        continue;
                    }
                    bindings.insert(key.to_string(), action.clone());
                }
            }
        }
    };
    insert(&parsed.navigation, nav);
    insert(&parsed.display_controls, display);
    insert(&parsed.tts_controls, tts);
    insert(&parsed.application, app);
    Ok(KeyMap { bindings })
}

impl KeyMap {
    /// Decode a normalized key name (e.g. "j", "enter", "pagedown").
    pub fn get(&self, key: &str) -> Option<&Action> {
        self.bindings.get(key)
    }
}

/// Map a crossterm KeyEvent to the normalized key name used by the layout
/// files, then to its action. Arrow keys and paging keys work in every
/// layout regardless of the JSON bindings.
pub fn decode_key(code: crossterm::event::KeyCode, map: &KeyMap) -> Option<Action> {
    use crossterm::event::KeyCode;
    let name = match code {
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Enter => "enter".to_string(),
        KeyCode::Esc => "esc".to_string(),
        KeyCode::Tab => "tab".to_string(),
        KeyCode::Backspace => "backspace".to_string(),
        KeyCode::Up => return Some(Action::PrevLine),
        KeyCode::Down => return Some(Action::NextLine),
        KeyCode::PageUp => return Some(Action::PageUp),
        KeyCode::PageDown => return Some(Action::PageDown),
        KeyCode::Home => return Some(Action::MoveToBeginning),
        KeyCode::End => return Some(Action::MoveToEnd),
        KeyCode::Left => return Some(Action::PrevParagraph),
        KeyCode::Right => return Some(Action::NextParagraph),
        _ => return None,
    };
    map.get(&name).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_layout_maps_core_keys() {
        let map = load("default").unwrap();
        assert_eq!(map.get("j"), Some(&Action::PrevLine)); // next_sentence per file
        assert_eq!(map.get("k"), Some(&Action::NextLine));
        assert_eq!(map.get("x"), Some(&Action::NextChapter));
        assert_eq!(map.get("z"), Some(&Action::PrevChapter));
        assert_eq!(map.get("c"), Some(&Action::ToggleChapterIndex));
        assert_eq!(map.get("v"), Some(&Action::CycleUiComplexity));
        assert_eq!(map.get("a"), Some(&Action::TtsToggleRead));
        assert_eq!(map.get("p"), Some(&Action::TtsPlayPause));
        assert_eq!(map.get("q"), Some(&Action::Quit));
    }

    #[test]
    fn enter_decodes_to_select() {
        let map = load("default").unwrap();
        assert_eq!(map.get("enter"), Some(&Action::SelectMenuItem));
    }

    #[test]
    fn unknown_layout_rejected() {
        assert!(load("colemak").is_err());
    }
}
