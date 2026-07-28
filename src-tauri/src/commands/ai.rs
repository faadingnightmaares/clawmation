//! AI step-macro commands: the per-macro step editor's backend surface
//! (`ui_app.py::{macro_to_steps, steps_save, steps_run, steps_test}`).
//!
//! `macro_to_steps` and `steps_save` are file work, so they follow the same
//! wrapper/`*_in` split as `macros.rs`: a thin `#[tauri::command]` resolves the
//! data directory (and, for save, emits the one log line the source emits) over a
//! pure `*_in(dir, …)` unit-tested against a temp dir. `steps_run` and
//! `steps_test` are runtime/vision work, so they delegate straight to [`Core`].

use std::collections::HashSet;
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
pub fn node_graph_list() -> Value {
    node_graph_list_in(&nodes_dir())
}

#[tauri::command(async)]
pub fn node_graph_create(state: State<AppState>, name: String) -> Value {
    let result = node_graph_create_in(&nodes_dir(), &name);
    if result["ok"] == json!(true) {
        state.emit(
            "ok",
            format!(
                "Created Loop '{}'",
                result["name"].as_str().unwrap_or("Loop")
            ),
        );
    }
    result
}

#[tauri::command(async)]
pub fn node_graph_rename(state: State<AppState>, old_name: String, new_name: String) -> Value {
    let result = node_graph_rename_in(&nodes_dir(), &old_name, &new_name);
    if result["ok"] == json!(true) {
        state.emit(
            "ok",
            format!(
                "Renamed Loop '{}' to '{}'",
                old_name,
                result["name"].as_str().unwrap_or(&new_name)
            ),
        );
    }
    result
}

#[tauri::command(async)]
pub fn node_graph_delete(state: State<AppState>, name: String) -> Value {
    let result = node_graph_delete_in(&nodes_dir(), &name);
    if result["ok"] == json!(true) {
        state.emit("ok", format!("Deleted Loop '{name}'"));
    }
    result
}

fn available_macro_names() -> HashSet<String> {
    std::fs::read_dir(paths::macros_dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            (path.is_file() && path.extension().and_then(|ext| ext.to_str()) == Some("json"))
                .then(|| path.file_stem()?.to_str().map(str::to_string))
                .flatten()
        })
        .collect()
}

fn available_chain_ids(state: &AppState) -> HashSet<String> {
    state
        .chains
        .list()
        .into_iter()
        .map(|chain| chain.id)
        .collect()
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
pub fn node_graph_load(loop_name: String) -> Value {
    node_graph_load_in(&paths::macros_dir(), &loop_name)
}

#[tauri::command(async)]
pub fn node_graph_validate(state: State<AppState>, graph: Value) -> Value {
    match serde_json::from_value::<NodeGraph>(graph) {
        Ok(graph) => {
            json!(graph.validate_with_resources(
                &available_macro_names(),
                &available_chain_ids(state.inner()),
            ))
        }
        Err(error) => json!({
            "ok": false,
            "errors": [format!("Bad node graph: {error}")],
            "warnings": [],
        }),
    }
}

#[tauri::command(async)]
pub fn node_graph_save(state: State<AppState>, loop_name: String, graph: Value) -> Value {
    let mut parsed = match serde_json::from_value::<NodeGraph>(graph.clone()) {
        Ok(graph) => graph,
        Err(error) => return json!({ "ok": false, "error": error.to_string() }),
    };
    parsed.name = loop_name.clone();
    let report = parsed.validate_with_resources(
        &available_macro_names(),
        &available_chain_ids(state.inner()),
    );
    if !report.ok {
        return json!({ "ok": false, "error": report.errors.join("; "), "validation": report });
    }
    let result = node_graph_save_in(&nodes_dir(), &loop_name, graph);
    if result["ok"] == json!(true) {
        state.emit("ok", format!("Saved Loop '{loop_name}'"));
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
    let parsed = match serde_json::from_value::<NodeGraph>(graph.clone()) {
        Ok(graph) => graph,
        Err(error) => return json!({ "ok": false, "error": format!("Bad node graph: {error}") }),
    };
    let report = parsed.validate_with_resources(
        &available_macro_names(),
        &available_chain_ids(state.inner()),
    );
    if !report.ok {
        return json!({ "ok": false, "error": report.errors.join("; ") });
    }
    state.core.node_graph_run(graph, state.chains.list())
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

fn node_graph_load_in(macros_dir: &Path, loop_name: &str) -> Value {
    let graph_path = macros_dir.join("nodes").join(format!("{loop_name}.json"));
    if !graph_path.exists() {
        return json!({ "ok": false, "error": "Loop not found" });
    }
    match NodeGraph::load(&graph_path) {
        Ok(mut graph) => {
            graph.name = loop_name.to_string();
            json!({ "ok": true, "graph": graph, "source": "saved" })
        }
        Err(error) => json!({ "ok": false, "error": error }),
    }
}

fn safe_loop_name(name: &str) -> Result<String, &'static str> {
    let safe = name
        .trim()
        .chars()
        .filter(|character| character.is_alphanumeric() || matches!(character, ' ' | '_' | '-'))
        .take(80)
        .collect::<String>()
        .trim()
        .to_string();
    if safe.is_empty() {
        return Err("Invalid Loop name");
    }
    let stem = safe.split('.').next().unwrap_or(&safe).to_ascii_uppercase();
    if matches!(
        stem.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    ) {
        return Err("That Loop name is reserved by Windows");
    }
    Ok(safe)
}

fn node_graph_list_in(nodes_dir: &Path) -> Value {
    let mut loops = Vec::new();
    if let Ok(entries) = std::fs::read_dir(nodes_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() || path.extension().and_then(|value| value.to_str()) != Some("json")
            {
                continue;
            }
            let Some(name) = path
                .file_stem()
                .and_then(|value| value.to_str())
                .map(str::to_string)
            else {
                continue;
            };
            let updated_at = entry
                .metadata()
                .ok()
                .and_then(|metadata| metadata.modified().ok())
                .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|duration| duration.as_millis() as u64)
                .unwrap_or_default();
            match NodeGraph::load(&path) {
                Ok(graph) => loops.push(json!({
                    "name": name,
                    "nodes": graph.nodes.len(),
                    "valid_file": true,
                    "updated_at": updated_at,
                })),
                Err(_) => loops.push(json!({
                    "name": name,
                    "nodes": 0,
                    "valid_file": false,
                    "updated_at": updated_at,
                })),
            }
        }
    }
    loops.sort_by(|left, right| {
        left["name"]
            .as_str()
            .unwrap_or("")
            .to_ascii_lowercase()
            .cmp(&right["name"].as_str().unwrap_or("").to_ascii_lowercase())
    });
    json!(loops)
}

fn node_graph_create_in(nodes_dir: &Path, requested_name: &str) -> Value {
    let base = match safe_loop_name(requested_name) {
        Ok(name) => name,
        Err(error) => return json!({ "ok": false, "error": error }),
    };
    let mut name = base.clone();
    let mut suffix = 2u32;
    while nodes_dir.join(format!("{name}.json")).exists() {
        name = format!("{base} {suffix}");
        suffix += 1;
    }
    let graph = NodeGraph::from_steps(&name, Vec::new());
    match graph.save_to(&nodes_dir.join(format!("{name}.json"))) {
        Ok(()) => json!({ "ok": true, "name": name, "graph": graph }),
        Err(error) => json!({ "ok": false, "error": error }),
    }
}

fn node_graph_rename_in(nodes_dir: &Path, old_name: &str, new_name: &str) -> Value {
    let old_name = old_name.trim();
    let safe = match safe_loop_name(new_name) {
        Ok(name) => name,
        Err(error) => return json!({ "ok": false, "error": error }),
    };
    let source = nodes_dir.join(format!("{old_name}.json"));
    if !source.exists() {
        return json!({ "ok": false, "error": "Loop not found" });
    }
    let destination = nodes_dir.join(format!("{safe}.json"));
    let same_file = old_name.eq_ignore_ascii_case(&safe);
    if destination.exists() && !same_file {
        return json!({ "ok": false, "error": "A Loop with that name already exists" });
    }
    let mut graph = match NodeGraph::load(&source) {
        Ok(graph) => graph,
        Err(error) => return json!({ "ok": false, "error": error }),
    };
    graph.name = safe.clone();
    if let Err(error) = graph.save_to(&destination) {
        return json!({ "ok": false, "error": error });
    }
    if !same_file {
        if let Err(error) = std::fs::remove_file(&source) {
            let _ = std::fs::remove_file(&destination);
            return json!({ "ok": false, "error": error.to_string() });
        }
    }
    json!({ "ok": true, "name": safe })
}

fn node_graph_delete_in(nodes_dir: &Path, name: &str) -> Value {
    let path = nodes_dir.join(format!("{}.json", name.trim()));
    if !path.exists() {
        return json!({ "ok": false, "error": "Loop not found" });
    }
    match std::fs::remove_file(path) {
        Ok(()) => json!({ "ok": true }),
        Err(error) => json!({ "ok": false, "error": error.to_string() }),
    }
}

fn node_graph_save_in(nodes_dir: &Path, loop_name: &str, graph: Value) -> Value {
    let mut graph: NodeGraph = match serde_json::from_value(graph) {
        Ok(graph) => graph,
        Err(error) => return json!({ "ok": false, "error": error.to_string() }),
    };
    graph.name = loop_name.to_string();
    let report = graph.validate();
    if !report.ok {
        return json!({
            "ok": false,
            "error": report.errors.join("; "),
            "errors": report.errors,
            "warnings": report.warnings,
        });
    }
    let path = nodes_dir.join(format!("{loop_name}.json"));
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
            mouse_motion: None,
            dx: 0,
            dy: 0,
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
    fn node_graph_workspaces_create_list_rename_load_and_delete() {
        let macros = temp_dir("node_workspaces");
        let nodes = macros.join("nodes");

        let created = node_graph_create_in(&nodes, "Farm Loop");
        assert_eq!(created["ok"], true);
        assert_eq!(created["name"], "Farm Loop");
        assert_eq!(created["graph"]["name"], "Farm Loop");
        assert_eq!(created["graph"]["nodes"].as_array().unwrap().len(), 2);

        let duplicate = node_graph_create_in(&nodes, "Farm Loop");
        assert_eq!(duplicate["name"], "Farm Loop 2");
        let listed = node_graph_list_in(&nodes);
        assert_eq!(listed.as_array().unwrap().len(), 2);
        assert!(listed[0]["updated_at"].as_u64().is_some());

        let renamed = node_graph_rename_in(&nodes, "Farm Loop", "Daily Farm");
        assert_eq!(renamed["ok"], true);
        assert_eq!(renamed["name"], "Daily Farm");
        assert!(!nodes.join("Farm Loop.json").exists());
        assert!(nodes.join("Daily Farm.json").exists());

        let reopened = node_graph_load_in(&macros, "Daily Farm");
        assert_eq!(reopened["ok"], true);
        assert_eq!(reopened["source"], "saved");
        assert_eq!(reopened["graph"]["name"], "Daily Farm");

        let deleted = node_graph_delete_in(&nodes, "Daily Farm");
        assert_eq!(deleted["ok"], true);
        assert!(!nodes.join("Daily Farm.json").exists());
    }

    #[test]
    fn node_graph_workspaces_do_not_touch_recorded_macros() {
        let macros = temp_dir("node_workspace_isolation");
        let nodes = macros.join("nodes");
        let recorded = Macro {
            name: "Same Name".into(),
            events: vec![ev(InputEventType::KeyPress, 0.1, 0, 0)],
            ..Default::default()
        };
        let macro_path = macros.join("Same Name.json");
        recorded.save_to(&macro_path).unwrap();

        assert_eq!(node_graph_create_in(&nodes, "Same Name")["ok"], true);
        assert_eq!(
            node_graph_rename_in(&nodes, "Same Name", "Renamed Loop")["ok"],
            true
        );
        assert_eq!(node_graph_delete_in(&nodes, "Renamed Loop")["ok"], true);
        assert!(
            macro_path.exists(),
            "Loop CRUD must never alter a recorded macro"
        );
    }

    #[test]
    fn node_graph_workspace_names_are_safe_and_collisions_are_rejected() {
        let nodes = temp_dir("node_workspace_names");
        assert_eq!(node_graph_create_in(&nodes, "CON")["ok"], false);
        assert_eq!(node_graph_create_in(&nodes, "!!!")["ok"], false);
        assert_eq!(
            node_graph_create_in(&nodes, "  My: Loop?  ")["name"],
            "My Loop"
        );
        assert_eq!(node_graph_create_in(&nodes, "Other")["ok"], true);
        assert_eq!(
            node_graph_rename_in(&nodes, "Other", "My Loop")["ok"],
            false
        );
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
