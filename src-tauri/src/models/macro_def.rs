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

/// Current on-disk macro schema. Missing versions deserialize as v1 so startup
/// migration can distinguish legacy recordings from files created by this app.
pub const CURRENT_MACRO_FORMAT_VERSION: u32 = 2;
pub const LEGACY_MACRO_FORMAT_VERSION: u32 = 1;

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

/// How a recorded mouse move must be replayed.
///
/// Older macros omit this field and keep the legacy hybrid replay path. New
/// recordings explicitly separate ordinary pointer positioning from raw camera
/// deltas so Roblox cursor locking cannot erase camera movement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MouseMotionMode {
    Pointer,
    Camera,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mouse_motion: Option<MouseMotionMode>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub dx: i64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub dy: i64,
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

fn is_zero(value: &i64) -> bool {
    *value == 0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Macro {
    // Required: Python reads `d["name"]`, so a nameless file fails to load and
    // `list_macros` skips it. No serde default keeps that behavior.
    pub name: String,
    #[serde(default = "legacy_format_version")]
    pub format_version: u32,
    #[serde(default = "default_resolution")]
    pub record_resolution: (u32, u32),
    #[serde(default)]
    pub created_at: f64,
    /// Exact active recording length for v2+ files. Legacy files have only the
    /// final event timestamp and are repaired conservatively during migration.
    #[serde(default)]
    pub recording_duration: Option<f64>,
    // Python coerces a hand-written `null` in these four back to the default
    // (`if not isinstance(x, ...): x = default`); `null_default` reproduces that.
    #[serde(
        default,
        deserialize_with = "crate::util::null_default",
        rename = "loop"
    )]
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

fn legacy_format_version() -> u32 {
    LEGACY_MACRO_FORMAT_VERSION
}

impl Default for Macro {
    fn default() -> Self {
        Self {
            name: String::new(),
            format_version: CURRENT_MACRO_FORMAT_VERSION,
            record_resolution: DEFAULT_RESOLUTION,
            created_at: 0.0,
            recording_duration: None,
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
        let recorded = self
            .recording_duration
            .filter(|d| d.is_finite() && *d >= 0.0)
            .unwrap_or(0.0);
        let edited = self.events.last().map(|e| e.timestamp).unwrap_or(0.0);
        recorded.max(edited)
    }

    /// Reject a corrupt or unsupported timeline before it can inject input.
    /// Errors include the event index so a hand-edited file is repairable.
    pub fn validate_for_playback(&self) -> Result<(), String> {
        const MAX_EVENTS: usize = 2_000_000;
        const MAX_DURATION_SECONDS: f64 = 7.0 * 24.0 * 60.0 * 60.0;

        if self.format_version == 0 || self.format_version > CURRENT_MACRO_FORMAT_VERSION {
            return Err(format!(
                "unsupported macro format {} (this app supports up to {})",
                self.format_version, CURRENT_MACRO_FORMAT_VERSION
            ));
        }
        if self.record_resolution.0 == 0 || self.record_resolution.1 == 0 {
            return Err("recording resolution must be non-zero".to_string());
        }
        if self.events.is_empty() {
            return Err("macro has no events".to_string());
        }
        if self.events.len() > MAX_EVENTS {
            return Err(format!(
                "macro has too many events ({}; maximum {})",
                self.events.len(),
                MAX_EVENTS
            ));
        }

        let mut previous = 0.0;
        for (index, event) in self.events.iter().enumerate() {
            if !event.timestamp.is_finite() || event.timestamp < 0.0 {
                return Err(format!("event {index} has an invalid timestamp"));
            }
            if index > 0 && event.timestamp < previous {
                return Err(format!(
                    "event {index} timestamp {:.4} is before event {} timestamp {:.4}",
                    event.timestamp,
                    index - 1,
                    previous
                ));
            }
            if event.timestamp > MAX_DURATION_SECONDS {
                return Err(format!(
                    "event {index} exceeds the maximum seven-day timeline"
                ));
            }
            if !event.duration.is_finite() || event.duration < 0.0 {
                return Err(format!("event {index} has an invalid duration"));
            }
            if let Some(cfg) = event.checkpoint.as_ref() {
                for field in ["timeout", "poll"] {
                    if let Some(value) = cfg.get(field) {
                        let Some(number) = value.as_f64() else {
                            return Err(format!(
                                "event {index} checkpoint field '{field}' must be a number"
                            ));
                        };
                        if !number.is_finite() || number < 0.0 {
                            return Err(format!(
                                "event {index} checkpoint field '{field}' is invalid"
                            ));
                        }
                    }
                }
            }
            previous = event.timestamp;
        }

        if let Some(duration) = self.recording_duration {
            if !duration.is_finite() || duration < 0.0 {
                return Err("recording duration must be finite and non-negative".to_string());
            }
        }
        Ok(())
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

    #[test]
    fn missing_version_loads_as_legacy_but_new_macros_are_current() {
        let legacy: Macro =
            serde_json::from_str(r#"{"name":"old","record_resolution":[1920,1080],"events":[]}"#)
                .unwrap();
        assert_eq!(legacy.format_version, LEGACY_MACRO_FORMAT_VERSION);
        assert_eq!(
            Macro::default().format_version,
            CURRENT_MACRO_FORMAT_VERSION
        );
    }

    #[test]
    fn mouse_motion_metadata_is_backward_compatible_and_roundtrips() {
        let legacy: MacroEvent =
            serde_json::from_str(r#"{"type":"MOUSE_MOVE","timestamp":0.1,"x":12,"y":34}"#).unwrap();
        assert_eq!(legacy.mouse_motion, None);
        assert_eq!((legacy.dx, legacy.dy), (0, 0));
        let legacy_json = serde_json::to_string(&legacy).unwrap();
        assert!(!legacy_json.contains("mouse_motion"));
        assert!(!legacy_json.contains("\"dx\""));
        assert!(!legacy_json.contains("\"dy\""));

        let mut camera = legacy;
        camera.mouse_motion = Some(MouseMotionMode::Camera);
        camera.dx = 17;
        camera.dy = -9;
        let roundtrip: MacroEvent =
            serde_json::from_str(&serde_json::to_string(&camera).unwrap()).unwrap();
        assert_eq!(roundtrip.mouse_motion, Some(MouseMotionMode::Camera));
        assert_eq!((roundtrip.dx, roundtrip.dy), (17, -9));
    }

    #[test]
    fn playback_validation_rejects_non_monotonic_and_non_finite_timelines() {
        let event = |timestamp| MacroEvent {
            event_type: InputEventType::Wait,
            timestamp,
            x: 0,
            y: 0,
            mouse_motion: None,
            dx: 0,
            dy: 0,
            button: "left".to_string(),
            key: String::new(),
            delta: 0,
            duration: 0.0,
            checkpoint: None,
        };
        let backwards = Macro {
            name: "bad".to_string(),
            events: vec![event(2.0), event(1.0)],
            ..Default::default()
        };
        assert!(backwards
            .validate_for_playback()
            .unwrap_err()
            .contains("event 1 timestamp"));

        let non_finite = Macro {
            name: "bad".to_string(),
            events: vec![event(f64::NAN)],
            ..Default::default()
        };
        assert!(non_finite
            .validate_for_playback()
            .unwrap_err()
            .contains("invalid timestamp"));
    }
}
