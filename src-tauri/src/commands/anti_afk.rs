//! Anti-AFK window discovery and session controls.

use serde_json::{json, Value};
use tauri::State;

use crate::engine::anti_afk::{AntiAfkAction, AntiAfkSnapshot};
use crate::hardware::window::{list_selectable_windows, SelectableWindow};
use crate::state::AppState;

#[tauri::command(async)]
pub fn anti_afk_list_windows() -> Vec<SelectableWindow> {
    list_selectable_windows()
}

#[tauri::command(async)]
pub fn anti_afk_get(state: State<AppState>) -> AntiAfkSnapshot {
    state.anti_afk.get()
}

#[tauri::command(async)]
pub fn anti_afk_update(
    state: State<AppState>,
    target_id: Option<String>,
    interval_min: Option<u32>,
    action: Option<AntiAfkAction>,
    enabled: Option<bool>,
) -> Value {
    let previous = state.anti_afk.get();
    let snapshot = match state
        .anti_afk
        .update(target_id, interval_min, action, enabled)
    {
        Ok(snapshot) => snapshot,
        Err(error) => return json!({ "ok": false, "error": error }),
    };

    if snapshot.interval_min != previous.interval_min || snapshot.action != previous.action {
        let mut config = state.core.config.lock().unwrap();
        config.anti_afk_interval_min = snapshot.interval_min;
        config.anti_afk_action = snapshot.action.as_config().to_string();
        if let Err(error) = config.save() {
            config.anti_afk_interval_min = previous.interval_min;
            config.anti_afk_action = previous.action.as_config().to_string();
            let _ = state.anti_afk.update(
                None,
                Some(previous.interval_min),
                Some(previous.action),
                None,
            );
            return json!({ "ok": false, "error": error.to_string() });
        }
    }

    json!({ "ok": true, "state": snapshot })
}
