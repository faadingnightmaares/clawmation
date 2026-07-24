//! Guard data model — `macros/guards/<macro_name>.json` as `{"guards": [...]}`.
//!
//! Mirrors `anime_macro/guards.py::Guard`. `from_dict` preserves persisted
//! `0`/`""`/`[]` (no falsy coercion), so container-level `#[serde(default)]`
//! plus a manual `Default` reproduces it exactly: present values are kept,
//! missing keys fall back to the canonical defaults. Field order matches
//! Python's `asdict`.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// One entry of a guard's `macro_sequence` (run only by the VisionAgent runtime).
/// `repeat` of 0 means infinite; a missing `repeat` defaults to 1.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MacroSeqItem {
    #[serde(default)]
    pub name: String,
    #[serde(default = "one_i64")]
    pub repeat: i64,
}

fn one_i64() -> i64 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Guard {
    pub id: String,
    pub name: String,
    /// Detection method: `"color"`, `"template"`, or `"ocr"`.
    pub method: String,
    pub hsv_low: [i64; 3],
    pub hsv_high: [i64; 3],
    /// Region as percentages `[x, y, w, h]` of the frame.
    pub region: [f64; 4],
    pub min_area: i64,
    pub template_path: String,
    pub threshold: f64,
    pub click_offset: Vec<i64>,
    pub click_line: Vec<i64>,
    pub click_lines: Vec<Vec<i64>>,
    pub ocr_text: String,
    /// Response when triggered: `"click"` or `"key"`.
    pub action: String,
    pub key: String,
    pub macro_sequence: Vec<MacroSeqItem>,
    pub resume_delay: f64,
    pub cooldown: f64,
    pub enabled: bool,
}

impl Default for Guard {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: "Guard".to_string(),
            method: "color".to_string(),
            hsv_low: [0, 0, 0],
            hsv_high: [179, 255, 255],
            region: [0.0, 0.0, 100.0, 100.0],
            min_area: 40,
            template_path: String::new(),
            threshold: 0.8,
            click_offset: Vec::new(),
            click_line: Vec::new(),
            click_lines: Vec::new(),
            ocr_text: String::new(),
            action: "click".to_string(),
            key: String::new(),
            macro_sequence: Vec::new(),
            resume_delay: 3.0,
            cooldown: 2.0,
            enabled: true,
        }
    }
}

/// On-disk container: `{"guards": [Guard, ...]}`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GuardFile {
    #[serde(default)]
    pub guards: Vec<Guard>,
}

impl GuardFile {
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(self).expect("GuardFile always serializes");
        crate::util::write_atomic(path, json.as_bytes())
    }
}
