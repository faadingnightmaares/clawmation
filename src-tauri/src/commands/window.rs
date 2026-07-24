//! Window-chrome commands.

use serde_json::{json, Value};

/// Minimize the main window — `Api.window_minimize_to_tray`. Python's docstring
/// says "hide from taskbar", but the code just does `ShowWindow(SW_MINIMIZE)`, a
/// plain minimize; the always-running tray icon is what keeps the window one
/// click away. Faithful to the code, not the docstring.
#[tauri::command]
pub fn window_minimize_to_tray(window: tauri::Window) -> Value {
    match window.minimize() {
        Ok(()) => json!({ "ok": true }),
        Err(e) => json!({ "ok": false, "error": e.to_string() }),
    }
}
