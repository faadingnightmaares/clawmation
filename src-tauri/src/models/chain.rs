//! Chain data model: `config/chains.json` as a bare array.
//!
//! Mirrors `anime_macro/chains.py::Chain`. Python's `from_dict` uses
//! `float(d.get("delay_between", 1.0) or 1.0)` and `int(d.get("repeat", 1) or 1)`,
//! which coerce a *persisted* `0` back to the default on load. That silently
//! defeats "repeat = 0 → infinite" after a disk round-trip, a genuine source
//! quirk. We reproduce it exactly for a faithful 1:1 migration rather than
//! silently changing behavior; see MIGRATION-NOTES.md if this should be fixed.

use std::path::Path;

use serde::{Deserialize, Deserializer, Serialize};

#[derive(Debug, Clone, Serialize)]
pub struct Chain {
    pub id: String,
    pub name: String,
    pub macro_names: Vec<String>,
    pub delay_between: f64,
    /// Times to run the whole chain. 0 means infinite, but see the note above:
    /// a persisted 0 is coerced to 1 on load, matching the Python source.
    pub repeat: i64,
}

impl Default for Chain {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            macro_names: Vec::new(),
            delay_between: 1.0,
            repeat: 1,
        }
    }
}

/// Raw on-disk shape with plain defaults; coercion is applied when converting
/// into `Chain`.
#[derive(Deserialize)]
struct ChainRaw {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    macro_names: Vec<String>,
    #[serde(default)]
    delay_between: f64,
    #[serde(default)]
    repeat: i64,
}

impl<'de> Deserialize<'de> for Chain {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = ChainRaw::deserialize(deserializer)?;
        Ok(Chain {
            id: raw.id,
            name: raw.name,
            macro_names: raw.macro_names,
            // `x or 1.0` / `x or 1`: absent OR persisted-zero both become the default.
            delay_between: if raw.delay_between == 0.0 {
                1.0
            } else {
                raw.delay_between
            },
            repeat: if raw.repeat == 0 { 1 } else { raw.repeat },
        })
    }
}

pub fn load(path: &Path) -> Vec<Chain> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

pub fn save_to(path: &Path, chains: &[Chain]) -> std::io::Result<()> {
    let json = serde_json::to_string_pretty(chains).expect("chains always serialize");
    crate::util::write_atomic(path, json.as_bytes())
}
