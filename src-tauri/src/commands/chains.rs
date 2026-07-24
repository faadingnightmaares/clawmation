//! Chain commands — CRUD, run/stop, duplicate, validate, and duration.
//!
//! Thin wrappers over [`ChainManager`](crate::engine::chains::ChainManager) on
//! [`AppState`]. `run_chain` reuses [`crate::state::run_chain`] (shared with the
//! scheduler's fire-chain callback). `validate_chain` and `get_chain_duration`
//! read a `list()` snapshot: Python copies `macro_names` under the lock and
//! releases it before touching the filesystem, so a snapshot is identical without
//! reaching into the manager's internals. Ports of `Api.list_chains` /
//! `add_chain` / `remove_chain` / `duplicate_chain` / `update_chain` /
//! `run_chain` / `validate_chain` / `get_chain_duration` / `stop_chain` /
//! `get_running_chain`.

use serde_json::{json, Value};
use tauri::State;

use crate::models::macro_def::Macro;
use crate::paths;
use crate::state::AppState;
use crate::util::round1;

#[tauri::command]
pub fn list_chains(state: State<AppState>) -> Value {
    json!(state.chains.list())
}

#[tauri::command]
pub fn add_chain(
    state: State<AppState>,
    name: String,
    macro_names: Vec<String>,
    delay_between: Option<f64>,
    repeat: Option<i64>,
) -> Value {
    let count = macro_names.len();
    let chain = state.chains.add(
        &name,
        macro_names,
        delay_between.unwrap_or(1.0),
        repeat.unwrap_or(1),
    );
    state.emit("ok", format!("Chain '{name}' created ({count} macros)"));
    json!({ "ok": true, "chain": chain })
}

#[tauri::command]
pub fn remove_chain(state: State<AppState>, chain_id: String) -> Value {
    json!({ "ok": state.chains.remove(&chain_id) })
}

#[tauri::command]
pub fn duplicate_chain(state: State<AppState>, chain_id: String) -> Value {
    match state.chains.duplicate(&chain_id) {
        Some(dup) => {
            state.emit(
                "ok",
                format!("Duplicated chain '{}' → '{}'", dup.source_name, dup.name),
            );
            json!({ "ok": true, "id": dup.id, "name": dup.name })
        }
        None => json!({ "ok": false, "error": "Chain not found" }),
    }
}

#[tauri::command]
pub fn update_chain(
    state: State<AppState>,
    chain_id: String,
    name: Option<String>,
    macro_names: Option<Vec<String>>,
    delay_between: Option<f64>,
    repeat: Option<i64>,
) -> Value {
    let ok = state
        .chains
        .update(&chain_id, name, macro_names, delay_between, repeat);
    json!({ "ok": ok })
}

#[tauri::command]
pub fn run_chain(state: State<AppState>, chain_id: String) -> Value {
    crate::state::run_chain(&state.core, &state.chains, &chain_id)
}

#[tauri::command]
pub fn validate_chain(state: State<AppState>, chain_id: String) -> Value {
    let Some(chain) = state.chains.list().into_iter().find(|c| c.id == chain_id) else {
        return json!({ "ok": false, "error": "Chain not found" });
    };
    let mut missing: Vec<String> = Vec::new();
    for name in &chain.macro_names {
        let stem = name.strip_suffix(".json").unwrap_or(name);
        if !paths::macros_dir().join(format!("{stem}.json")).exists() {
            missing.push(name.clone());
        }
    }
    json!({ "ok": true, "missing": missing, "total": chain.macro_names.len() })
}

#[tauri::command]
pub fn get_chain_duration(state: State<AppState>, chain_id: String) -> Value {
    let Some(chain) = state.chains.list().into_iter().find(|c| c.id == chain_id) else {
        return json!({ "ok": false, "error": "Chain not found" });
    };
    let mut total_duration = 0.0f64;
    for name in &chain.macro_names {
        let stem = name.strip_suffix(".json").unwrap_or(name);
        let path = paths::macros_dir().join(format!("{stem}.json"));
        if !path.exists() {
            return json!({ "ok": false, "error": format!("Macro '{name}' not found") });
        }
        match Macro::load(&path) {
            Ok(m) => total_duration += m.duration(),
            Err(_) => {
                return json!({ "ok": false, "error": format!("Failed to load macro '{name}'") })
            }
        }
    }
    json!({ "ok": true, "duration": round1(total_duration) })
}

#[tauri::command]
pub fn stop_chain(state: State<AppState>) -> Value {
    state.chains.stop();
    state.emit("info", "Chain stop requested");
    json!({ "ok": true })
}

#[tauri::command]
pub fn get_running_chain(state: State<AppState>) -> Value {
    let progress = state.chains.get_progress();
    if !progress
        .get("running")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return json!({ "running": false });
    }
    json!({
        "running": true,
        "id": progress.get("chain_id"),
        "name": progress.get("chain_name"),
        "current_index": progress.get("current_index"),
        "current_macro": progress.get("current_macro"),
        "total_macros": progress.get("total_macros"),
        "iteration": progress.get("iteration"),
        "total_iterations": progress.get("total_iterations"),
    })
}
