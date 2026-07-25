//! Play-stats data model: `config/stats.json`.
//!
//! Mirrors `anime_macro/stats.py::PlayStats`. On-disk shape is
//! `{"stats": {name: {count, first_played, last_played}}, "history": [...]}`.
//! A legacy file may instead be a bare `{name: stat}` map with no "stats" key;
//! `load` accepts both but always writes the wrapped form. History is capped at
//! 100 entries, newest-first.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

pub const MAX_HISTORY: usize = 100;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MacroStat {
    #[serde(default)]
    pub count: i64,
    #[serde(default)]
    pub first_played: f64,
    #[serde(default)]
    pub last_played: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub name: String,
    #[serde(default)]
    pub timestamp: f64,
    #[serde(default)]
    pub duration: f64,
    #[serde(default = "default_status")]
    pub status: String,
}

fn default_status() -> String {
    "completed".to_string()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StatsFile {
    // BTreeMap (not HashMap) so serialized key order is deterministic. Python's
    // dict preserves insertion order; sorted output is a stable, diff-friendly
    // equivalent for a file only this app reads.
    #[serde(default)]
    pub stats: BTreeMap<String, MacroStat>,
    #[serde(default)]
    pub history: Vec<HistoryEntry>,
}

impl StatsFile {
    pub fn load(path: &Path) -> Self {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(_) => return Self::default(),
        };
        let value: serde_json::Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => return Self::default(),
        };
        if value.get("stats").is_some() {
            serde_json::from_value(value).unwrap_or_default()
        } else if value.is_object() {
            // Legacy: the whole document is the stats map, no history.
            let stats = serde_json::from_value(value).unwrap_or_default();
            Self {
                stats,
                history: Vec::new(),
            }
        } else {
            Self::default()
        }
    }

    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(self).expect("stats always serialize");
        crate::util::write_atomic(path, json.as_bytes())
    }
}
