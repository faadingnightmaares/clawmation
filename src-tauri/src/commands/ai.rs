//! AI step-macro commands: the per-macro step editor's backend surface
//! (`ui_app.py::{macro_to_steps, steps_save, steps_run, steps_test}`).
//!
//! `macro_to_steps` and `steps_save` are file work, so they follow the same
//! wrapper/`*_in` split as `macros.rs`: a thin `#[tauri::command]` resolves the
//! data directory (and, for save, emits the one log line the source emits) over a
//! pure `*_in(dir, …)` unit-tested against a temp dir. `steps_run` and
//! `steps_test` are runtime/vision work, so they delegate straight to [`Core`].

use std::path::Path;

use serde_json::{json, Value};
use tauri::{State, Window};

use crate::commands::window::with_window_out_of_frame;
use crate::models::macro_def::Macro;
use crate::models::step::{macro_to_steps as convert, AIMacro, Step};
use crate::paths;
use crate::state::AppState;

/// AI macros live under `macros/ai/`, mirroring Python's `AI_DIR = MACROS_DIR/"ai"`.
fn ai_dir() -> std::path::PathBuf {
    paths::macros_dir().join("ai")
}

#[tauri::command(async)]
pub fn macro_to_steps(macro_name: String) -> Value {
    macro_to_steps_in(&paths::macros_dir(), &macro_name)
}

#[tauri::command(async)]
pub fn steps_save(state: State<AppState>, macro_name: String, steps: Vec<Value>) -> Value {
    let result = steps_save_in(&ai_dir(), &macro_name, steps);
    if result["ok"] == json!(true) {
        let count = result["count"].as_u64().unwrap_or(0);
        state.emit("ok", format!("Saved step macro '{macro_name}' ({count} steps)"));
        json!({ "ok": true })
    } else {
        let err = result["error"].as_str().unwrap_or("").to_string();
        state.emit("err", format!("Save failed: {err}"));
        json!({ "ok": false, "error": err })
    }
}

#[tauri::command(async)]
pub fn steps_run(state: State<AppState>, steps: Vec<Value>) -> Value {
    state.core.steps_run(steps)
}

/// Same reason as `guard_test`: the step editor is in front of whatever the step
/// is written to find, so it goes away for the length of the grab.
#[tauri::command(async)]
pub fn steps_test(state: State<AppState>, window: Window, step: Value) -> Value {
    with_window_out_of_frame(&window, || state.core.steps_test(step))
}

// ── Pure implementations (unit-tested against a temp dir) ────────────────────

/// `macro_to_steps`: load a recorded macro and convert it to an editable step
/// list. A missing macro reports `Macro '<name>' not found`; a load error surfaces
/// verbatim, matching the source's `except → str(e)`.
fn macro_to_steps_in(macros_dir: &Path, macro_name: &str) -> Value {
    let path = macros_dir.join(format!("{macro_name}.json"));
    if !path.exists() {
        return json!({ "ok": false, "error": format!("Macro '{macro_name}' not found") });
    }
    let macro_def = match Macro::load(&path) {
        Ok(m) => m,
        Err(e) => return json!({ "ok": false, "error": e.to_string() }),
    };
    let steps = convert(&macro_def);
    let count = steps.len();
    json!({ "ok": true, "steps": steps, "count": count })
}

/// `steps_save`: parse the edited step dicts and write them as an AI macro
/// (replacing any existing macro of that name). The frontend never sends loop
/// settings, so the macro always saves with `loop=false, loop_count=1`, the
/// defaults `AIMacro` carries. Returns `{ok, count}` on success (the wrapper turns
/// `count` into the log line and drops it from the reply, as the source does) or
/// `{ok:false, error}`. Parse and write failures share the one `except` in
/// Python, so both land here.
fn steps_save_in(ai_dir: &Path, macro_name: &str, steps: Vec<Value>) -> Value {
    let parsed: Result<Vec<Step>, _> = steps.into_iter().map(serde_json::from_value).collect();
    let step_objs = match parsed {
        Ok(objs) => objs,
        Err(e) => return json!({ "ok": false, "error": e.to_string() }),
    };
    let count = step_objs.len();
    let m = AIMacro { name: macro_name.to_string(), steps: step_objs, ..Default::default() };
    let path = ai_dir.join(format!("{macro_name}.json"));
    match m.save_to(&path) {
        Ok(()) => json!({ "ok": true, "count": count }),
        Err(e) => json!({ "ok": false, "error": e.to_string() }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::macro_def::{InputEventType, MacroEvent};
    use crate::test_support::temp_dir;

    fn ev(event_type: InputEventType, timestamp: f64, x: i64, y: i64) -> MacroEvent {
        MacroEvent {
            event_type,
            timestamp,
            x,
            y,
            button: "left".to_string(),
            key: String::new(),
            delta: 0,
            duration: 0.0,
            checkpoint: None,
        }
    }

    #[test]
    fn macro_to_steps_missing_reports_by_name() {
        let macros = temp_dir("ai_m2s_missing");
        let r = macro_to_steps_in(&macros, "nope");
        assert_eq!(r["ok"], json!(false));
        assert_eq!(r["error"], "Macro 'nope' not found");
    }

    #[test]
    fn macro_to_steps_converts_a_recorded_click() {
        let macros = temp_dir("ai_m2s");
        let name = "__test___m2s";
        let m = Macro {
            name: name.to_string(),
            events: vec![
                ev(InputEventType::MouseDown, 0.1, 100, 200),
                ev(InputEventType::MouseUp, 0.12, 100, 200),
            ],
            ..Default::default()
        };
        m.save_to(&macros.join(format!("{name}.json"))).unwrap();

        let r = macro_to_steps_in(&macros, name);
        assert_eq!(r["ok"], json!(true));
        assert_eq!(r["count"], json!(1));
        assert_eq!(r["steps"][0]["type"], "click");
        assert_eq!(r["steps"][0]["label"], "Click (100, 200)");
    }

    #[test]
    fn steps_save_writes_an_ai_macro_and_counts() {
        let dir = temp_dir("ai_save");
        let steps = vec![
            json!({ "type": "click", "x": 5, "y": 6, "label": "Click (5, 6)" }),
            json!({ "type": "delay", "delay": 1.5, "label": "Wait 1.5s" }),
        ];
        let r = steps_save_in(&dir, "combo", steps);
        assert_eq!(r["ok"], json!(true));
        assert_eq!(r["count"], json!(2));

        // Round-trips: the file loads back as a two-step, non-looping macro.
        let saved = AIMacro::load(&dir.join("combo.json")).unwrap();
        assert_eq!(saved.name, "combo");
        assert!(!saved.loop_enabled);
        assert_eq!(saved.loop_count, 1);
        assert_eq!(saved.steps.len(), 2);
        assert_eq!(saved.steps[1].step_type, "delay");
        assert_eq!(saved.steps[1].delay, 1.5);
    }

    #[test]
    fn steps_save_rejects_a_malformed_step() {
        let dir = temp_dir("ai_save_bad");
        // `x` must be an integer; a string is a from_dict/deserialize error.
        let steps = vec![json!({ "type": "click", "x": "nope" })];
        let r = steps_save_in(&dir, "bad", steps);
        assert_eq!(r["ok"], json!(false));
        assert!(r["error"].as_str().unwrap().len() > 0);
    }
}
