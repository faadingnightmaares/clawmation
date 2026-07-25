//! Schedule data model: `config/schedules.json` as a bare array.
//!
//! Mirrors `anime_macro/scheduler.py::Schedule`. Like `Chain`, Python coerces a
//! persisted `0` for `interval_min` (-> 30.0) and `repeat` (-> 1) back to the
//! default on load; reproduced here for fidelity. Other fields keep their
//! present values. Field order matches Python's `asdict`.

use std::path::Path;

use serde::{Deserialize, Deserializer, Serialize};

#[derive(Debug, Clone, Serialize)]
pub struct Schedule {
    pub id: String,
    pub macro_name: String,
    /// `"interval"`, `"once"`, or `"daily"` (no cron).
    pub kind: String,
    pub interval_min: f64,
    pub at_time: String,
    pub enabled: bool,
    pub last_run: f64,
    pub repeat: i64,
    /// `"macro"` or `"chain"`.
    pub target_type: String,
    pub target_id: String,
}

impl Default for Schedule {
    fn default() -> Self {
        Self {
            id: String::new(),
            macro_name: String::new(),
            kind: "interval".to_string(),
            interval_min: 30.0,
            at_time: String::new(),
            enabled: true,
            last_run: 0.0,
            repeat: 1,
            target_type: "macro".to_string(),
            target_id: String::new(),
        }
    }
}

#[derive(Deserialize)]
struct ScheduleRaw {
    #[serde(default)]
    id: String,
    #[serde(default)]
    macro_name: String,
    #[serde(default = "default_kind")]
    kind: String,
    #[serde(default)]
    interval_min: f64,
    #[serde(default)]
    at_time: String,
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default)]
    last_run: f64,
    #[serde(default)]
    repeat: i64,
    #[serde(default = "default_target_type")]
    target_type: String,
    #[serde(default)]
    target_id: String,
}

fn default_kind() -> String {
    "interval".to_string()
}

fn default_target_type() -> String {
    "macro".to_string()
}

fn default_true() -> bool {
    true
}

impl<'de> Deserialize<'de> for Schedule {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = ScheduleRaw::deserialize(deserializer)?;
        Ok(Schedule {
            id: raw.id,
            macro_name: raw.macro_name,
            kind: raw.kind,
            // `x or 30.0` / `x or 1`: absent OR persisted-zero both become the default.
            interval_min: if raw.interval_min == 0.0 {
                30.0
            } else {
                raw.interval_min
            },
            at_time: raw.at_time,
            enabled: raw.enabled,
            last_run: raw.last_run,
            repeat: if raw.repeat == 0 { 1 } else { raw.repeat },
            target_type: raw.target_type,
            target_id: raw.target_id,
        })
    }
}

pub fn load(path: &Path) -> Vec<Schedule> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

pub fn save_to(path: &Path, schedules: &[Schedule]) -> std::io::Result<()> {
    let json = serde_json::to_string_pretty(schedules).expect("schedules always serialize");
    crate::util::write_atomic(path, json.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_fields_roundtrip_and_legacy_defaults_to_macro() {
        let s = Schedule {
            id: "s1".to_string(),
            target_type: "chain".to_string(),
            target_id: "chain_abc".to_string(),
            kind: "once".to_string(),
            at_time: "09:30".to_string(),
            ..Default::default()
        };
        let text = serde_json::to_string(&s).unwrap();
        let back: Schedule = serde_json::from_str(&text).unwrap();
        assert_eq!(back.target_type, "chain");
        assert_eq!(back.target_id, "chain_abc");
        assert_eq!(back.at_time, "09:30");
        assert_eq!(back.kind, "once");

        // A legacy file with no target_type/target_id defaults to a macro target.
        let legacy: Schedule = serde_json::from_str(r#"{"id":"old","macro_name":"m1"}"#).unwrap();
        assert_eq!(legacy.target_type, "macro");
        assert_eq!(legacy.target_id, "");
        assert_eq!(legacy.macro_name, "m1");
    }
}
