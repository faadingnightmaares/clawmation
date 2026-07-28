//! Vision checkpoint CRUD: insert, remove, and list `CHECKPOINT` events inside a
//! recorded macro (`ui_app.py::{add_checkpoint, remove_checkpoint, list_checkpoints}`).
//!
//! Same wrapper/`*_in` split as `macros.rs`: the thin `#[tauri::command]` resolves
//! the data directory and emits the one log line the source emits, while the pure
//! `*_in(macros_dir, …)` does the file work and is unit-tested against a temp dir.

use std::path::Path;

use serde_json::{json, Value};
use tauri::State;

use crate::hardware::recorder::round4;
use crate::models::macro_def::{InputEventType, Macro, MacroEvent};
use crate::paths;
use crate::state::AppState;

/// Strip a trailing `.json`, matching Python's `name[:-5] if endswith(".json")`.
fn strip_json(name: &str) -> &str {
    name.strip_suffix(".json").unwrap_or(name)
}

#[tauri::command(async)]
pub fn add_checkpoint(
    state: State<AppState>,
    name: String,
    index: i64,
    checkpoint: Value,
) -> Value {
    let result = add_checkpoint_in(&paths::macros_dir(), &name, index, checkpoint);
    if result["ok"] == json!(true) {
        let stem = strip_json(&name);
        // The returned index is the source's `index` after the `< 0 → append`
        // rewrite, so the log line reports whatever step number it settled on,
        // including one past the end when the caller asked to append.
        let idx = result["index"].as_i64().unwrap_or(index);
        state.emit("ok", format!("Checkpoint added to '{stem}' at step {idx}"));
    }
    result
}

#[tauri::command(async)]
pub fn remove_checkpoint(name: String, index: i64) -> Value {
    remove_checkpoint_in(&paths::macros_dir(), &name, index)
}

#[tauri::command(async)]
pub fn list_checkpoints(name: String) -> Value {
    list_checkpoints_in(&paths::macros_dir(), &name)
}

// ── Pure implementations (unit-tested against a temp dir) ────────────────────

/// `add_checkpoint`: insert a `CHECKPOINT` event whose timestamp is interpolated
/// from its neighbours. A negative `index` appends; `min(index, n)` clamps the
/// insertion point, but the *returned* index is the un-clamped value, so an
/// out-of-range request reports (and inserts at the end under) that number, a
/// faithful quirk of the source.
fn add_checkpoint_in(macros_dir: &Path, name: &str, index: i64, mut checkpoint: Value) -> Value {
    let stem = strip_json(name);
    let path = macros_dir.join(format!("{stem}.json"));
    if !path.exists() {
        return json!({ "ok": false, "error": format!("Not found: {stem}") });
    }
    let mut macro_def = match Macro::load(&path) {
        Ok(m) => m,
        Err(e) => return json!({ "ok": false, "error": e.to_string() }),
    };
    if let Value::Object(config) = &mut checkpoint {
        config
            .entry("on_timeout".to_string())
            .or_insert_with(|| Value::String("stop".to_string()));
    }

    let n = macro_def.events.len() as i64;
    let mut index = index;
    if index < 0 {
        index = n;
    }
    // Interpolate the timestamp between the surrounding events. `round4` matches
    // Python's `to_dict` rounding on save; the model serializes the timestamp
    // verbatim, so a fresh event must be pre-rounded to stay byte-identical.
    let ts = if macro_def.events.is_empty() || index <= 0 {
        0.0
    } else if index >= n {
        macro_def.events.last().unwrap().timestamp + 0.01
    } else {
        macro_def.events[(index - 1) as usize].timestamp + 0.001
    };

    let ev = MacroEvent {
        event_type: InputEventType::Checkpoint,
        timestamp: round4(ts),
        x: 0,
        y: 0,
        mouse_motion: None,
        dx: 0,
        dy: 0,
        button: "left".to_string(),
        key: String::new(),
        delta: 0,
        duration: 0.0,
        checkpoint: Some(checkpoint),
    };
    macro_def.events.insert(index.min(n) as usize, ev);
    match macro_def.save_to(&path) {
        Ok(()) => json!({ "ok": true, "index": index }),
        Err(e) => json!({ "ok": false, "error": e.to_string() }),
    }
}

/// `remove_checkpoint`: pop the event at `index` only if it is a `CHECKPOINT`;
/// any other index (out of range, or a non-checkpoint event) is a no-op error.
fn remove_checkpoint_in(macros_dir: &Path, name: &str, index: i64) -> Value {
    let stem = strip_json(name);
    let path = macros_dir.join(format!("{stem}.json"));
    if !path.exists() {
        return json!({ "ok": false, "error": format!("Not found: {stem}") });
    }
    let mut macro_def = match Macro::load(&path) {
        Ok(m) => m,
        Err(e) => return json!({ "ok": false, "error": e.to_string() }),
    };
    let n = macro_def.events.len() as i64;
    if index >= 0
        && index < n
        && macro_def.events[index as usize].event_type == InputEventType::Checkpoint
    {
        macro_def.events.remove(index as usize);
        return match macro_def.save_to(&path) {
            Ok(()) => json!({ "ok": true }),
            Err(e) => json!({ "ok": false, "error": e.to_string() }),
        };
    }
    json!({ "ok": false, "error": "No checkpoint at that index" })
}

/// `list_checkpoints`: every `CHECKPOINT` event carrying a non-empty config, with
/// its event index. The truthiness gate (`and ev.checkpoint`) drops a checkpoint
/// with an empty `{}` config, matching the play loop's own gate.
fn list_checkpoints_in(macros_dir: &Path, name: &str) -> Value {
    let stem = strip_json(name);
    let path = macros_dir.join(format!("{stem}.json"));
    if !path.exists() {
        return json!({ "ok": false, "error": format!("Not found: {stem}") });
    }
    let macro_def = match Macro::load(&path) {
        Ok(m) => m,
        Err(e) => return json!({ "ok": false, "error": e.to_string() }),
    };
    let checkpoints: Vec<Value> = macro_def
        .events
        .iter()
        .enumerate()
        .filter(|(_, ev)| {
            ev.event_type == InputEventType::Checkpoint
                && matches!(&ev.checkpoint, Some(Value::Object(m)) if !m.is_empty())
        })
        .map(|(i, ev)| json!({ "index": i, "checkpoint": ev.checkpoint }))
        .collect();
    json!({ "ok": true, "checkpoints": checkpoints })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_dir;

    /// A macro with `n` mouse-move events at t = 0.0, 0.1, 0.2, … saved to disk.
    fn make_macro(dir: &Path, name: &str, n: usize) -> std::path::PathBuf {
        let events = (0..n)
            .map(|i| MacroEvent {
                event_type: InputEventType::MouseMove,
                timestamp: i as f64 * 0.1,
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
            })
            .collect();
        let m = Macro {
            name: name.to_string(),
            events,
            ..Default::default()
        };
        let path = dir.join(format!("{name}.json"));
        m.save_to(&path).unwrap();
        path
    }

    fn cfg() -> Value {
        json!({ "mode": "wait_for", "method": "color", "action": "click" })
    }

    #[test]
    fn add_interpolates_timestamp_by_position() {
        let macros = temp_dir("cp_add");
        let name = "__test___cp_add";
        let path = make_macro(&macros, name, 3); // t = 0.0, 0.1, 0.2

        // Middle insert: previous event's timestamp + 0.001.
        let r = add_checkpoint_in(&macros, name, 2, cfg());
        assert_eq!(r["ok"], json!(true));
        assert_eq!(r["index"], json!(2));
        let m = Macro::load(&path).unwrap();
        assert_eq!(m.events[2].event_type, InputEventType::Checkpoint);
        assert_eq!(m.events[2].timestamp, 0.101, "prev (0.1) + 0.001");
        assert_eq!(
            m.events[2].checkpoint.as_ref().unwrap()["on_timeout"],
            json!("stop"),
            "new checkpoints fail closed unless explicitly overridden"
        );
        assert_eq!(m.events.len(), 4);
    }

    #[test]
    fn add_at_zero_and_end_and_negative() {
        let macros = temp_dir("cp_add_edges");
        let name = "__test___cp_edges";
        let path = make_macro(&macros, name, 2); // t = 0.0, 0.1

        // index <= 0 → timestamp 0.0, inserted at the front.
        assert_eq!(
            add_checkpoint_in(&macros, name, 0, cfg())["index"],
            json!(0)
        );
        // index >= n → last timestamp + 0.01, appended.
        let end = add_checkpoint_in(&macros, name, 99, cfg());
        assert_eq!(
            end["index"],
            json!(99),
            "out-of-range index returned verbatim"
        );
        // Negative → append (index becomes n).
        let neg = add_checkpoint_in(&macros, name, -1, cfg());
        let m = Macro::load(&path).unwrap();
        assert_eq!(neg["index"].as_i64().unwrap(), (m.events.len() - 1) as i64);
        assert_eq!(m.events[0].timestamp, 0.0, "front insert is t=0.0");
        // The far-out-of-range insert landed at the end (clamped), not at step 99.
        assert_eq!(m.events[0].event_type, InputEventType::Checkpoint);
    }

    #[test]
    fn add_to_empty_macro_is_timestamp_zero() {
        let macros = temp_dir("cp_add_empty");
        let name = "__test___cp_empty";
        let path = make_macro(&macros, name, 0);
        let r = add_checkpoint_in(&macros, name, 5, cfg());
        assert_eq!(r["ok"], json!(true));
        let m = Macro::load(&path).unwrap();
        assert_eq!(m.events.len(), 1);
        assert_eq!(
            m.events[0].timestamp, 0.0,
            "empty macro → t=0.0 regardless of index"
        );
    }

    #[test]
    fn add_missing_macro_is_not_found() {
        let macros = temp_dir("cp_add_missing");
        let r = add_checkpoint_in(&macros, "__test___nope", 0, cfg());
        assert_eq!(r["ok"], json!(false));
        assert_eq!(r["error"], json!("Not found: __test___nope"));
    }

    #[test]
    fn remove_only_pops_checkpoint_events() {
        let macros = temp_dir("cp_remove");
        let name = "__test___cp_rm";
        let path = make_macro(&macros, name, 2);
        add_checkpoint_in(&macros, name, 1, cfg()); // checkpoint now at index 1

        // A non-checkpoint index (a mouse-move) is rejected.
        let r = remove_checkpoint_in(&macros, name, 0);
        assert_eq!(r["ok"], json!(false));
        assert_eq!(r["error"], json!("No checkpoint at that index"));

        // The checkpoint index removes cleanly.
        let r = remove_checkpoint_in(&macros, name, 1);
        assert_eq!(r["ok"], json!(true));
        let m = Macro::load(&path).unwrap();
        assert_eq!(m.events.len(), 2, "back to the two moves");
        assert!(m
            .events
            .iter()
            .all(|e| e.event_type != InputEventType::Checkpoint));

        // Out-of-range index is the same no-op error.
        let r = remove_checkpoint_in(&macros, name, 99);
        assert_eq!(r["error"], json!("No checkpoint at that index"));
    }

    #[test]
    fn list_returns_checkpoints_with_indices_and_skips_empty_config() {
        let macros = temp_dir("cp_list");
        let name = "__test___cp_list";
        let path = make_macro(&macros, name, 2);
        add_checkpoint_in(&macros, name, 1, json!({ "mode": "wait_for" }));

        // Hand-insert an empty-config checkpoint at the end; list must skip it.
        let mut m = Macro::load(&path).unwrap();
        m.events.push(MacroEvent {
            event_type: InputEventType::Checkpoint,
            timestamp: 9.0,
            x: 0,
            y: 0,
            mouse_motion: None,
            dx: 0,
            dy: 0,
            button: "left".to_string(),
            key: String::new(),
            delta: 0,
            duration: 0.0,
            checkpoint: Some(json!({})),
        });
        m.save_to(&path).unwrap();

        let r = list_checkpoints_in(&macros, name);
        assert_eq!(r["ok"], json!(true));
        let cps = r["checkpoints"].as_array().unwrap();
        assert_eq!(cps.len(), 1, "empty-config checkpoint is filtered out");
        assert_eq!(cps[0]["index"], json!(1));
        assert_eq!(
            cps[0]["checkpoint"],
            json!({ "mode": "wait_for", "on_timeout": "stop" })
        );
    }

    #[test]
    fn list_missing_macro_is_not_found() {
        let macros = temp_dir("cp_list_missing");
        let r = list_checkpoints_in(&macros, "__test___nope.json");
        assert_eq!(r["ok"], json!(false));
        assert_eq!(r["error"], json!("Not found: __test___nope"));
    }
}
