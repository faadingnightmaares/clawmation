//! Playback commands — play, stop, and the global emergency stop.
//!
//! `play_macro` and `stop_playback` are thin wrappers over
//! [`Core`](crate::core::Core). `emergency_stop` also reaches the vision agent
//! that lives on [`AppState`], so it is orchestrated here rather than on `Core`
//! — a faithful port of `Api.emergency_stop`.

use serde_json::{json, Value};
use tauri::State;

use crate::state::AppState;

#[tauri::command]
pub fn play_macro(
    state: State<AppState>,
    name: String,
    repeat: Option<Value>,
    speed: Option<f64>,
) -> Value {
    // `speed` defaults to 1.0, matching the Python keyword default.
    state.core.play_macro(&name, repeat, speed.unwrap_or(1.0))
}

#[tauri::command]
pub fn stop_playback(state: State<AppState>) -> Value {
    state.core.stop_playback()
}

#[tauri::command]
pub fn emergency_stop(state: State<AppState>) -> Value {
    emergency_stop_impl(state.inner())
}

/// The body of `Api.emergency_stop`, split out so the global stop hotkey can call
/// it directly (the hotkey handler holds an `&AppState`, not a command `State`).
pub fn emergency_stop_impl(state: &AppState) -> Value {
    let core = &state.core;
    let mut stopped: Vec<&str> = Vec::new();

    if let Some(recorder) = core.recorder.lock().unwrap().as_mut() {
        if recorder.is_recording() {
            recorder.stop();
            stopped.push("recording");
        }
    }
    if core.player.is_playing() {
        core.player.stop();
        stopped.push("playback");
    }
    {
        // Gate on is_running (Python's check), then take the agent out from under
        // the lock before stop() joins its thread: a join blocked on an infinite
        // in-flight macro must not wedge concurrent vision_status polls.
        let running = state
            .vision_agent
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|a| a.is_running());
        if running {
            if let Some(agent) = state.vision_agent.lock().unwrap().take() {
                agent.stop();
            }
            stopped.push("vision");
        }
    }

    core.set_mode("idle");
    let summary = if stopped.is_empty() {
        "nothing active".to_string()
    } else {
        stopped.join(", ")
    };
    core.emit("err", format!("STOP ({summary})"));
    json!({ "ok": true, "stopped": stopped })
}
