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
use crate::models::node_graph::NodeGraph;
use crate::models::step::{macro_to_steps as convert, AIMacro, Step};
use crate::paths;
use crate::state::AppState;

/// AI macros live under `macros/ai/`, mirroring Python's `AI_DIR = MACROS_DIR/"ai"`.
fn ai_dir() -> std::path::PathBuf {
    paths::macros_dir().join("ai")
}

fn nodes_dir() -> std::path::PathBuf {
    paths::macros_dir().join("nodes")
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
        state.emit(
            "ok",
            format!("Saved step macro '{macro_name}' ({count} steps)"),
        );
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

#[tauri::command(async)]
pub fn node_graph_load(macro_name: String) -> Value {
    node_graph_load_in(&paths::macros_dir(), &macro_name)
}

#[tauri::command(async)]
pub fn node_graph_validate(graph: Value) -> Value {
    match serde_json::from_value::<NodeGraph>(graph) {
        Ok(graph) => json!(graph.validate()),
        Err(error) => json!({
            "ok": false,
            "errors": [format!("Bad node graph: {error}")],
            "warnings": [],
        }),
    }
}

#[tauri::command(async)]
pub fn node_graph_save(state: State<AppState>, macro_name: String, graph: Value) -> Value {
    let result = node_graph_save_in(&nodes_dir(), &macro_name, graph);
    if result["ok"] == json!(true) {
        state.emit("ok", format!("Saved node graph '{macro_name}'"));
    } else {
        state.emit(
            "err",
            format!(
                "Node graph save failed: {}",
                result["error"].as_str().unwrap_or("invalid graph")
            ),
        );
    }
    result
}

#[tauri::command(async)]
pub fn node_graph_run(state: State<AppState>, graph: Value) -> Value {
    state.core.node_graph_run(graph)
}

// ── Pure implementations (unit-tested against a temp dir) ────────────────────

/// `macro_to_steps`: the editable step list for a macro. A saved fine-tune
/// (`macros/ai/<name>.json`) wins and is returned verbatim — re-deriving steps
/// from the recorded events, the stale source they were built from, would
/// silently drop any action the user inserted by hand (issue #3). Only when no
/// fine-tune has been saved yet does it convert the recorded macro. A missing
/// macro reports `Macro '<name>' not found`; a load error surfaces verbatim,
/// matching the source's `except → str(e)`.
fn macro_to_steps_in(macros_dir: &Path, macro_name: &str) -> Value {
    let ai_path = macros_dir.join("ai").join(format!("{macro_name}.json"));
    if ai_path.exists() {
        return match AIMacro::load(&ai_path) {
            Ok(m) => {
                let count = m.steps.len();
                json!({ "ok": true, "steps": m.steps, "count": count })
            }
            Err(e) => json!({ "ok": false, "error": e.to_string() }),
        };
    }
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
    let m = AIMacro {
        name: macro_name.to_string(),
        steps: step_objs,
        ..Default::default()
    };
    let path = ai_dir.join(format!("{macro_name}.json"));
    match m.save_to(&path) {
        Ok(()) => json!({ "ok": true, "count": count }),
        Err(e) => json!({ "ok": false, "error": e.to_string() }),
    }
}

fn node_graph_load_in(macros_dir: &Path, macro_name: &str) -> Value {
    let graph_path = macros_dir.join("nodes").join(format!("{macro_name}.json"));
    if graph_path.exists() {
        return match NodeGraph::load(&graph_path) {
            Ok(graph) => json!({ "ok": true, "graph": graph, "source": "saved" }),
            Err(error) => json!({ "ok": false, "error": error }),
        };
    }

    let steps_result = macro_to_steps_in(macros_dir, macro_name);
    if steps_result["ok"] != json!(true) {
        return steps_result;
    }
    let steps: Result<Vec<Step>, _> = serde_json::from_value(steps_result["steps"].clone());
    match steps {
        Ok(steps) => json!({
            "ok": true,
            "graph": NodeGraph::from_steps(macro_name, steps),
            "source": "imported",
        }),
        Err(error) => json!({ "ok": false, "error": error.to_string() }),
    }
}

fn node_graph_save_in(nodes_dir: &Path, macro_name: &str, graph: Value) -> Value {
    let mut graph: NodeGraph = match serde_json::from_value(graph) {
        Ok(graph) => graph,
        Err(error) => return json!({ "ok": false, "error": error.to_string() }),
    };
    graph.name = macro_name.to_string();
    let report = graph.validate();
    if !report.ok {
        return json!({
            "ok": false,
            "error": report.errors.join("; "),
            "errors": report.errors,
            "warnings": report.warnings,
        });
    }
    let path = nodes_dir.join(format!("{macro_name}.json"));
    match graph.save_to(&path) {
        Ok(()) => json!({ "ok": true, "warnings": report.warnings }),
        Err(error) => json!({ "ok": false, "error": error }),
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
    fn macro_to_steps_prefers_a_saved_fine_tune_over_the_recording() {
        let macros = temp_dir("ai_m2s_pref");
        let name = "__test___pref";
        // The recorded macro: one click, which would convert to one step.
        let m = Macro {
            name: name.to_string(),
            events: vec![
                ev(InputEventType::MouseDown, 0.1, 100, 200),
                ev(InputEventType::MouseUp, 0.12, 100, 200),
            ],
            ..Default::default()
        };
        m.save_to(&macros.join(format!("{name}.json"))).unwrap();
        // A saved fine-tune: the click PLUS a hand-inserted wait-for step.
        let ai_dir = macros.join("ai");
        std::fs::create_dir_all(&ai_dir).unwrap();
        let fine = AIMacro {
            name: name.to_string(),
            steps: vec![
                Step {
                    step_type: "click".to_string(),
                    label: "Click (100, 200)".to_string(),
                    ..Default::default()
                },
                Step {
                    step_type: "wait_for".to_string(),
                    label: "Wait for a colour".to_string(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        fine.save_to(&ai_dir.join(format!("{name}.json"))).unwrap();

        // The saved steps come back verbatim — the inserted action survives.
        let r = macro_to_steps_in(&macros, name);
        assert_eq!(r["ok"], json!(true));
        assert_eq!(r["count"], json!(2));
        assert_eq!(r["steps"][1]["type"], "wait_for");
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

    #[test]
    fn node_graph_load_imports_legacy_steps_then_prefers_saved_graph() {
        let macros = temp_dir("node_load");
        let name = "__test___node_load";
        let recorded = Macro {
            name: name.into(),
            events: vec![
                ev(InputEventType::MouseDown, 0.1, 5, 6),
                ev(InputEventType::MouseUp, 0.2, 5, 6),
            ],
            ..Default::default()
        };
        recorded
            .save_to(&macros.join(format!("{name}.json")))
            .unwrap();

        let imported = node_graph_load_in(&macros, name);
        assert_eq!(imported["ok"], true);
        assert_eq!(imported["source"], "imported");
        assert_eq!(imported["graph"]["nodes"].as_array().unwrap().len(), 3);

        let saved: NodeGraph = serde_json::from_value(imported["graph"].clone()).unwrap();
        saved
            .save_to(&macros.join("nodes").join(format!("{name}.json")))
            .unwrap();
        let reopened = node_graph_load_in(&macros, name);
        assert_eq!(reopened["source"], "saved");
    }

    #[test]
    fn node_graph_save_rejects_invalid_edges() {
        let dir = temp_dir("node_save_bad");
        let result = node_graph_save_in(
            &dir,
            "bad",
            json!({
                "version": 1,
                "entry": "start",
                "nodes": [{"id":"start","type":"start"}],
                "edges": [{"id":"e","from":"start","output":"next","to":"missing"}]
            }),
        );
        assert_eq!(result["ok"], false);
        assert!(result["error"].as_str().unwrap().contains("missing target"));
    }
}
