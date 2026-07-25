//! Scheduler commands: create, list, toggle, and remove schedules.
//!
//! Thin wrappers over [`MacroScheduler`](crate::engine::scheduler::MacroScheduler)
//! on [`AppState`]. A schedule targets a macro (the default) or a chain
//! (`schedule_chain`), distinguished by the `target_type` the engine stores. The
//! engine's `add` never fails for valid input, so Python's defensive `try/except`
//! (which could only ever return `ok: true`) collapses to a direct return here.
//! Ports of `Api.list_schedules` / `add_schedule` / `remove_schedule` /
//! `set_schedule_enabled` / `schedule_chain`.

use serde_json::{json, Value};
use tauri::State;

use crate::state::AppState;

#[tauri::command(async)]
pub fn list_schedules(state: State<AppState>) -> Value {
    json!(state.scheduler.list())
}

#[tauri::command(async)]
pub fn add_schedule(
    state: State<AppState>,
    macro_name: String,
    kind: Option<String>,
    interval_min: Option<f64>,
    at_time: Option<String>,
    repeat: Option<i64>,
    enabled: Option<bool>,
) -> Value {
    let kind = kind.unwrap_or_else(|| "interval".to_string());
    let sched = state.scheduler.add(
        &macro_name,
        &kind,
        interval_min.unwrap_or(30.0),
        &at_time.unwrap_or_default(),
        repeat.unwrap_or(1),
        enabled.unwrap_or(true),
        "macro",
        "",
    );
    state.emit("ok", format!("Scheduled '{macro_name}' ({kind})"));
    json!({ "ok": true, "schedule": sched })
}

#[tauri::command(async)]
pub fn remove_schedule(state: State<AppState>, schedule_id: String) -> Value {
    json!({ "ok": state.scheduler.remove(&schedule_id) })
}

#[tauri::command(async)]
pub fn set_schedule_enabled(state: State<AppState>, schedule_id: String, enabled: bool) -> Value {
    json!({ "ok": state.scheduler.set_enabled(&schedule_id, enabled) })
}

#[tauri::command(async)]
pub fn schedule_chain(
    state: State<AppState>,
    chain_id: String,
    kind: Option<String>,
    interval_min: Option<f64>,
    at_time: Option<String>,
) -> Value {
    let kind = kind.unwrap_or_else(|| "interval".to_string());
    let sched = state.scheduler.add(
        "",
        &kind,
        interval_min.unwrap_or(30.0),
        &at_time.unwrap_or_default(),
        1,
        true,
        "chain",
        &chain_id,
    );
    state.emit("ok", format!("Chain scheduled ({kind})"));
    json!({ "ok": true, "schedule": sched })
}
