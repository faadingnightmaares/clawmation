//! One-time, recoverable upgrades for user-owned macro files.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::models::macro_def::{InputEventType, Macro, MacroEvent, CURRENT_MACRO_FORMAT_VERSION};

/// The exact old tail is unknowable. Ten seconds restores the reported 2:37
/// recording whose final action was at 2:27 and errs toward safety for loops.
const LEGACY_LOOP_TAIL_SECONDS: f64 = 10.0;

#[derive(Debug, Default)]
pub struct MigrationReport {
    pub upgraded: usize,
    pub loop_tails_repaired: usize,
    pub errors: Vec<String>,
}

impl MigrationReport {
    pub fn summary(&self) -> Option<String> {
        if self.upgraded == 0 && self.errors.is_empty() {
            return None;
        }
        let mut text = format!(
            "Macro upgrade: {} file(s) upgraded, {} legacy loop tail(s) repaired",
            self.upgraded, self.loop_tails_repaired
        );
        if !self.errors.is_empty() {
            text.push_str(&format!(
                ", {} file(s) left unchanged due to errors",
                self.errors.len()
            ));
        }
        Some(text)
    }
}

fn backup_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("macro.json");
    path.with_file_name(format!("{name}.pre-v2.bak"))
}

fn checkpoint_fail_closed(event: &mut MacroEvent) {
    if event.event_type != InputEventType::Checkpoint {
        return;
    }
    let Some(Value::Object(config)) = event.checkpoint.as_mut() else {
        return;
    };
    config
        .entry("on_timeout".to_string())
        .or_insert_with(|| Value::String("stop".to_string()));
}

fn trailing_wait(timestamp: f64) -> MacroEvent {
    MacroEvent {
        event_type: InputEventType::Wait,
        timestamp,
        x: 0,
        y: 0,
        button: "left".to_string(),
        key: String::new(),
        delta: 0,
        duration: 0.0,
        checkpoint: None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MigrationOutcome {
    Current,
    Upgraded { tail_repaired: bool },
}

fn migrate_one(path: &Path) -> Result<MigrationOutcome, String> {
    let mut macro_def = Macro::load(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if macro_def.format_version >= CURRENT_MACRO_FORMAT_VERSION {
        return Ok(MigrationOutcome::Current);
    }

    let backup = backup_path(path);
    if !backup.exists() {
        std::fs::copy(path, &backup).map_err(|e| {
            format!(
                "{}: could not create migration backup {}: {e}",
                path.display(),
                backup.display()
            )
        })?;
    }

    let repeats = macro_def.loop_enabled || macro_def.loop_count > 1;
    let needs_tail = repeats
        && macro_def
            .events
            .last()
            .map(|e| {
                !matches!(
                    e.event_type,
                    InputEventType::Wait | InputEventType::Checkpoint
                )
            })
            .unwrap_or(false);
    if needs_tail {
        let timestamp = macro_def.events.last().unwrap().timestamp + LEGACY_LOOP_TAIL_SECONDS;
        macro_def.events.push(trailing_wait(timestamp));
    }
    for event in &mut macro_def.events {
        checkpoint_fail_closed(event);
    }
    macro_def.recording_duration = Some(
        macro_def
            .events
            .last()
            .map(|event| event.timestamp)
            .unwrap_or(0.0),
    );
    macro_def.format_version = CURRENT_MACRO_FORMAT_VERSION;
    macro_def
        .save_to(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(MigrationOutcome::Upgraded {
        tail_repaired: needs_tail,
    })
}

/// Bring one macro current immediately before use. This also covers a legacy
/// file imported after startup, not only files present during an app update.
pub fn ensure_macro_current(path: &Path) -> Result<bool, String> {
    match migrate_one(path)? {
        MigrationOutcome::Current => Ok(false),
        MigrationOutcome::Upgraded { tail_repaired } => Ok(tail_repaired),
    }
}

pub fn migrate_legacy_macros(macros_dir: &Path) -> MigrationReport {
    let mut report = MigrationReport::default();
    let entries = match std::fs::read_dir(macros_dir) {
        Ok(entries) => entries,
        Err(e) => {
            report.errors.push(format!("{}: {e}", macros_dir.display()));
            return report;
        }
    };

    let mut paths: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|s| s.to_str()) == Some("json"))
        .collect();
    paths.sort();

    for path in paths {
        match migrate_one(&path) {
            Ok(MigrationOutcome::Upgraded {
                tail_repaired: true,
            }) => {
                report.upgraded += 1;
                report.loop_tails_repaired += 1;
            }
            Ok(MigrationOutcome::Upgraded {
                tail_repaired: false,
            }) => report.upgraded += 1,
            Ok(MigrationOutcome::Current) => {}
            Err(e) => report.errors.push(e),
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_dir;
    use serde_json::Map;

    fn legacy_json(name: &str, looping: bool, final_type: &str) -> String {
        format!(
            r#"{{
  "name": "{name}",
  "record_resolution": [1920, 1080],
  "created_at": 1.0,
  "loop": {looping},
  "loop_count": 0,
  "events": [
    {{
      "type": "{final_type}",
      "timestamp": 147.0,
      "x": 1,
      "y": 2,
      "button": "left",
      "key": "",
      "delta": 0,
      "duration": 0.0,
      "checkpoint": null
    }}
  ]
}}"#
        )
    }

    #[test]
    fn legacy_loop_gets_a_backup_and_exactly_one_safe_tail() {
        let dir = temp_dir("migration_loop_tail");
        let path = dir.join("old.json");
        std::fs::write(&path, legacy_json("old", true, "MOUSE_UP")).unwrap();

        let first = migrate_legacy_macros(&dir);
        assert_eq!(first.upgraded, 1);
        assert_eq!(first.loop_tails_repaired, 1);
        assert!(backup_path(&path).exists());
        let migrated = Macro::load(&path).unwrap();
        assert_eq!(migrated.format_version, CURRENT_MACRO_FORMAT_VERSION);
        assert_eq!(
            migrated.events.last().unwrap().event_type,
            InputEventType::Wait
        );
        assert_eq!(migrated.events.last().unwrap().timestamp, 157.0);
        assert_eq!(migrated.recording_duration, Some(157.0));

        let second = migrate_legacy_macros(&dir);
        assert_eq!(second.upgraded, 0, "migration is idempotent");
        assert_eq!(Macro::load(&path).unwrap().events.len(), 2);
    }

    #[test]
    fn legacy_one_shot_is_versioned_without_inventing_a_tail() {
        let dir = temp_dir("migration_one_shot");
        let path = dir.join("old.json");
        std::fs::write(&path, legacy_json("old", false, "MOUSE_UP")).unwrap();
        let report = migrate_legacy_macros(&dir);
        assert_eq!(report.upgraded, 1);
        assert_eq!(report.loop_tails_repaired, 0);
        let migrated = Macro::load(&path).unwrap();
        assert_eq!(migrated.events.len(), 1);
        assert_eq!(migrated.recording_duration, Some(147.0));
    }

    #[test]
    fn legacy_file_imported_after_startup_is_upgraded_before_playback() {
        let dir = temp_dir("migration_late_import");
        assert_eq!(migrate_legacy_macros(&dir).upgraded, 0);

        let path = dir.join("imported-later.json");
        std::fs::write(&path, legacy_json("imported-later", true, "MOUSE_UP")).unwrap();

        assert!(ensure_macro_current(&path).unwrap());
        assert!(backup_path(&path).exists());
        let migrated = Macro::load(&path).unwrap();
        assert_eq!(migrated.format_version, CURRENT_MACRO_FORMAT_VERSION);
        assert_eq!(
            migrated.events.last().unwrap().event_type,
            InputEventType::Wait
        );
    }

    #[test]
    fn malformed_files_are_preserved_and_reported() {
        let dir = temp_dir("migration_bad");
        let path = dir.join("broken.json");
        std::fs::write(&path, b"{ definitely not json").unwrap();
        let original = std::fs::read(&path).unwrap();
        let report = migrate_legacy_macros(&dir);
        assert_eq!(report.upgraded, 0);
        assert_eq!(report.errors.len(), 1);
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert!(!backup_path(&path).exists());
    }

    #[test]
    fn legacy_checkpoint_timeout_becomes_fail_closed() {
        let dir = temp_dir("migration_checkpoint");
        let path = dir.join("old.json");
        let mut root: Value =
            serde_json::from_str(&legacy_json("old", true, "CHECKPOINT")).unwrap();
        root["events"][0]["checkpoint"] = Value::Object(Map::from_iter([(
            "mode".to_string(),
            Value::String("wait_for".to_string()),
        )]));
        std::fs::write(&path, serde_json::to_vec_pretty(&root).unwrap()).unwrap();
        migrate_legacy_macros(&dir);
        let migrated = Macro::load(&path).unwrap();
        assert_eq!(
            migrated.events[0].checkpoint.as_ref().unwrap()["on_timeout"],
            Value::String("stop".to_string())
        );
    }
}
