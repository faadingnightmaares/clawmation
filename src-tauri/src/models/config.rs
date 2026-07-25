//! User configuration, persisted to `config/settings.yaml`.
//!
//! Mirrors `anime_macro/config.py::MacroConfig`. Field defaults and the on-disk
//! YAML shape stay compatible so existing installs keep working. Unknown keys in
//! the file are ignored and missing keys fall back to defaults, exactly like the
//! Python loader's `{k: v for k, v in data.items() if k in fields}` filter.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::paths;

/// Capture resolution used when none is stored.
pub const DEFAULT_RESOLUTION: (u32, u32) = (2560, 1440);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MacroConfig {
    pub resolution: (u32, u32),
    /// Screen-capture backend: `"dxcam"` or `"mss"`.
    pub capture_backend: String,
    pub log_level: String,
    pub indicator_on_top: bool,
    /// Autonomous clicks travel a human-like curve instead of teleporting.
    pub humanize_clicks: bool,
    pub hotkey_record: String,
    pub hotkey_play: String,
    pub hotkey_stop: String,
    pub notify_on_schedule: bool,
    pub notify_on_complete: bool,
}

impl Default for MacroConfig {
    fn default() -> Self {
        Self {
            resolution: DEFAULT_RESOLUTION,
            capture_backend: "dxcam".to_string(),
            log_level: "INFO".to_string(),
            indicator_on_top: true,
            humanize_clicks: false,
            hotkey_record: "f9".to_string(),
            hotkey_play: "f10".to_string(),
            hotkey_stop: "f12".to_string(),
            notify_on_schedule: true,
            notify_on_complete: true,
        }
    }
}

impl MacroConfig {
    pub fn load() -> Self {
        Self::load_from(&paths::config_dir().join("settings.yaml"))
    }

    pub fn load_from(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_yaml::from_str(&text).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        self.save_to(&paths::config_dir().join("settings.yaml"))
    }

    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        let yaml = serde_yaml::to_string(self).expect("MacroConfig always serializes");
        crate::util::write_atomic(path, yaml.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_settings_fixture_loads_and_absent_keys_default() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/settings.yaml");
        let text = std::fs::read_to_string(&path).expect("fixture readable");
        // Deserialize the text directly; `MacroConfig::load` is hardcoded to the
        // real config dir and cannot point at the fixture.
        let cfg: MacroConfig = serde_yaml::from_str(&text).expect("settings.yaml parses");

        // Keys present in the file.
        assert_eq!(cfg.capture_backend, "mss");
        assert_eq!(cfg.hotkey_play, "f4");
        assert_eq!(cfg.hotkey_record, "f6");
        assert_eq!(cfg.hotkey_stop, "f3");
        assert!(!cfg.indicator_on_top);
        assert_eq!(cfg.log_level, "INFO");
        assert_eq!(cfg.resolution, (2560, 1440));

        // Keys absent from the file fall back to their defaults.
        assert!(!cfg.humanize_clicks, "absent humanize_clicks -> false");
        assert!(cfg.notify_on_schedule, "absent notify_on_schedule -> true");
        assert!(cfg.notify_on_complete, "absent notify_on_complete -> true");
    }
}
