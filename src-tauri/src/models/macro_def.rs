//! Recorded macro data model: `macros/<name>.json`.
//!
//! Mirrors `anime_macro/recorder.py::{Macro, InputEvent, InputEventType}`.
//! Python's `from_dict` reads some keys by subscript (`d["name"]`, `d["type"]`,
//! `d["timestamp"]`) and the rest with `d.get(key, default)`. Subscript access
//! raises `KeyError` when the key is absent, so the load fails and the caller
//! skips the record; we reproduce that by leaving those fields without a serde
//! default (a missing key becomes a deserialize error) while every `d.get`
//! field carries a default. Field declaration order matches Python's `to_dict`
//! so pretty-printed output is byte-identical.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::models::config::DEFAULT_RESOLUTION;

/// Input event kinds, serialized by name (e.g. `"MOUSE_MOVE"`), matching
/// Python's `InputEventType.name`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum InputEventType {
    MouseMove,
    MouseClick,
    MouseDown,
    MouseUp,
    KeyPress,
    KeyDown,
    KeyUp,
    Scroll,
    Wait,
    Checkpoint,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MacroEvent {
    // `type` and `timestamp` are read by subscript in Python (`d["type"]`,
    // `d["timestamp"]`), so a missing key is a hard error, not a default.
    #[serde(rename = "type")]
    pub event_type: InputEventType,
    // Python's `InputEvent.to_dict` writes `round(timestamp, 4)`. Files on disk are
    // therefore already 4-decimal, so load->save round-trips byte-identically here.
    // The recorder must round to 4 decimals when it saves freshly-captured events
    // to match Python's on-disk format (see MIGRATION-NOTES).
    pub timestamp: f64,
    #[serde(default)]
    pub x: i64,
    #[serde(default)]
    pub y: i64,
    #[serde(default = "default_button")]
    pub button: String,
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub delta: i64,
    #[serde(default)]
    pub duration: f64,
    // Emitted even when null to match the current `to_dict` output.
    #[serde(default)]
    pub checkpoint: Option<serde_json::Value>,
}

fn default_button() -> String {
    "left".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Macro {
    // Required: Python reads `d["name"]`, so a nameless file fails to load and
    // `list_macros` skips it. No serde default keeps that behavior.
    pub name: String,
    #[serde(default = "default_resolution")]
    pub record_resolution: (u32, u32),
    #[serde(default)]
    pub created_at: f64,
    // Python coerces a hand-written `null` in these four back to the default
    // (`if not isinstance(x, ...): x = default`); `null_default` reproduces that.
    #[serde(default, deserialize_with = "crate::util::null_default", rename = "loop")]
    pub loop_enabled: bool,
    #[serde(default, deserialize_with = "crate::util::null_default")]
    pub loop_count: i64,
    #[serde(default, deserialize_with = "crate::util::null_default")]
    pub category: String,
    #[serde(default, deserialize_with = "crate::util::null_default")]
    pub notes: String,
    #[serde(default)]
    pub events: Vec<MacroEvent>,
}

fn default_resolution() -> (u32, u32) {
    DEFAULT_RESOLUTION
}

impl Default for Macro {
    fn default() -> Self {
        Self {
            name: String::new(),
            record_resolution: DEFAULT_RESOLUTION,
            created_at: 0.0,
            loop_enabled: false,
            loop_count: 0,
            category: String::new(),
            notes: String::new(),
            events: Vec::new(),
        }
    }
}

impl Macro {
    /// Duration in seconds = timestamp of the last event (0.0 when empty).
    pub fn duration(&self) -> f64 {
        self.events.last().map(|e| e.timestamp).unwrap_or(0.0)
    }

    pub fn load(path: &Path) -> serde_json::Result<Self> {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        serde_json::from_str(&text)
    }

    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(self).expect("Macro always serializes");
        crate::util::write_atomic(path, json.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name)
    }

    #[test]
    fn notes_and_category_roundtrip_and_null_coerces_to_empty() {
        let m = Macro {
            name: "__test___notes".to_string(),
            notes: "my description".to_string(),
            category: "Fisch".to_string(),
            ..Default::default()
        };
        let text = serde_json::to_string(&m).unwrap();
        let loaded: Macro = serde_json::from_str(&text).unwrap();
        assert_eq!(loaded.notes, "my description");
        assert_eq!(loaded.category, "Fisch");

        // A hand-written `null` for notes coerces to "" (Python's isinstance guard).
        let mut v: serde_json::Value = serde_json::from_str(&text).unwrap();
        v["notes"] = serde_json::Value::Null;
        let coerced: Macro = serde_json::from_value(v).unwrap();
        assert_eq!(coerced.notes, "", "null notes coerced to empty string");
    }

    #[test]
    fn real_mouse_macro_fixture_roundtrips_created_at_exactly() {
        let m = Macro::load(&fixture("macro_1784601329.json")).expect("fixture loads");
        assert_eq!(m.name, "macro_1784601329");
        assert!(!m.events.is_empty(), "has events");
        assert_eq!(m.events[0].event_type, InputEventType::MouseMove);

        // Rust's Ryū and Python's repr both emit the shortest round-tripping
        // decimal, so the float survives re-serialization character-for-character.
        let text = serde_json::to_string_pretty(&m).unwrap();
        assert!(
            text.contains("\"created_at\": 1784601329.9119852"),
            "created_at preserved exactly"
        );
    }

    #[test]
    fn real_key_macro_fixture_roundtrips_flags_and_created_at() {
        let m = Macro::load(&fixture("repeat_hotkey_test.json")).expect("fixture loads");
        assert_eq!(m.name, "repeat_hotkey_test");
        assert!(m.loop_enabled, "loop flag preserved");
        assert_eq!(m.loop_count, 3);
        assert_eq!(m.events.len(), 2);
        assert_eq!(m.events[0].event_type, InputEventType::KeyPress);
        assert_eq!(m.events[0].key, "a");
        assert_eq!(m.events[1].key, "b");

        let text = serde_json::to_string_pretty(&m).unwrap();
        assert!(
            text.contains("\"created_at\": 1784659227.4523795"),
            "created_at preserved exactly"
        );
    }
}
