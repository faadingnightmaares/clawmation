//! Config and data-folder commands.

use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, State};

use crate::models::config::MacroConfig;
use crate::paths;
use crate::state::AppState;

/// Subset of config surfaced to the settings UI (mirrors `Api.get_config`).
#[derive(Debug, Serialize)]
pub struct ConfigDto {
    pub capture_backend: String,
    pub hotkey_record: String,
    pub hotkey_play: String,
    pub hotkey_stop: String,
    pub indicator_on_top: bool,
    pub humanize_clicks: bool,
    pub notify_on_schedule: bool,
    pub notify_on_complete: bool,
}

#[tauri::command(async)]
pub fn get_config(state: State<AppState>) -> ConfigDto {
    let c = state.core.config.lock().unwrap();
    ConfigDto {
        capture_backend: c.capture_backend.clone(),
        hotkey_record: c.hotkey_record.clone(),
        hotkey_play: c.hotkey_play.clone(),
        hotkey_stop: c.hotkey_stop.clone(),
        indicator_on_top: c.indicator_on_top,
        humanize_clicks: c.humanize_clicks,
        notify_on_schedule: c.notify_on_schedule,
        notify_on_complete: c.notify_on_complete,
    }
}

#[tauri::command(async)]
pub fn update_config(app: AppHandle, state: State<AppState>, patch: Value) -> Value {
    // `Api.update_config`: note which hotkeys the patch touches before applying,
    // so the global shortcuts can be re-registered if any changed.
    let hotkeys_changed = patch
        .as_object()
        .map(|o| {
            o.contains_key("hotkey_record")
                || o.contains_key("hotkey_play")
                || o.contains_key("hotkey_stop")
        })
        .unwrap_or(false);

    let mut guard = state.core.config.lock().unwrap();

    // Merge the patch over the current config; unknown keys are ignored on the
    // way back into `MacroConfig`, matching Python's `hasattr` guard.
    let mut merged = serde_json::to_value(&*guard).unwrap_or_default();
    if let (Some(obj), Some(patch_obj)) = (merged.as_object_mut(), patch.as_object()) {
        for (key, value) in patch_obj {
            obj.insert(key.clone(), value.clone());
        }
    }

    let new_config: MacroConfig = match serde_json::from_value(merged) {
        Ok(c) => c,
        Err(e) => return json!({ "ok": false, "error": e.to_string() }),
    };
    *guard = new_config;

    if let Err(e) = guard.save() {
        return json!({ "ok": false, "error": e.to_string() });
    }
    drop(guard);

    // A shortcut can parse and still be refused — Windows hands a global hotkey
    // to whoever asked first — so the keys that did not take ride back with the
    // result instead of failing silently.
    let unbound = if hotkeys_changed {
        crate::shell::hotkeys::register_from_config(&app)
    } else {
        Vec::new()
    };
    state.emit("ok", "Settings saved");
    json!({ "ok": true, "unbound": unbound })
}

/// Release the global shortcuts while the settings UI listens for a keystroke.
/// Without this, capturing a key that is already bound would fire that binding
/// instead — global shortcuts are exclusive and never reach the webview.
#[tauri::command(async)]
pub fn hotkeys_suspend(app: AppHandle) -> Value {
    crate::shell::hotkeys::suspend(&app);
    json!({ "ok": true })
}

/// Re-arm the global shortcuts after a capture ends, reporting any that the OS
/// refused.
#[tauri::command(async)]
pub fn hotkeys_resume(app: AppHandle) -> Value {
    let unbound = crate::shell::hotkeys::register_from_config(&app);
    json!({ "ok": true, "unbound": unbound })
}

#[derive(Debug, Serialize)]
pub struct DataPaths {
    pub ok: bool,
    pub root: String,
    pub macros_dir: String,
    pub templates_dir: String,
    pub snapshots_dir: String,
    pub config_dir: String,
    pub macro_count: i64,
    pub template_count: i64,
    pub snapshot_count: i64,
}

#[tauri::command(async)]
pub fn get_data_paths() -> DataPaths {
    DataPaths {
        ok: true,
        root: paths::root().display().to_string(),
        macros_dir: paths::macros_dir().display().to_string(),
        templates_dir: paths::templates_dir().display().to_string(),
        snapshots_dir: paths::snapshots_dir().display().to_string(),
        config_dir: paths::config_dir().display().to_string(),
        macro_count: paths::count_ext(&paths::macros_dir(), "json"),
        template_count: paths::count_ext(&paths::templates_dir(), "png"),
        snapshot_count: paths::count_ext(&paths::snapshots_dir(), "png"),
    }
}

#[tauri::command(async)]
pub fn open_data_folder(kind: String) -> Value {
    let dir = match kind.as_str() {
        "root" => paths::root(),
        "macros" => paths::macros_dir(),
        "templates" => paths::templates_dir(),
        "snapshots" => paths::snapshots_dir(),
        "config" => paths::config_dir(),
        other => return json!({ "ok": false, "error": format!("unknown folder: {other}") }),
    };
    let _ = std::fs::create_dir_all(&dir);
    match std::process::Command::new("explorer").arg(&dir).spawn() {
        Ok(_) => json!({ "ok": true }),
        Err(e) => json!({ "ok": false, "error": e.to_string() }),
    }
}
