//! Recording commands — start, stop, and pause capture.
//!
//! Thin `#[tauri::command]` wrappers over [`Core`](crate::core::Core), which
//! holds the faithful port of `Api.start_record` / `stop_record` /
//! `pause_record`.

use serde_json::Value;
use tauri::State;

use crate::state::AppState;

#[tauri::command(async)]
pub fn start_record(state: State<AppState>) -> Value {
    state.core.start_record()
}

#[tauri::command(async)]
pub fn stop_record(state: State<AppState>) -> Value {
    state.core.stop_record()
}

#[tauri::command(async)]
pub fn pause_record(state: State<AppState>) -> Value {
    state.core.pause_record()
}
