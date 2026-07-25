//! Window-chrome commands, and the "get out of the way" helper every screen
//! grab the editor starts runs behind.

use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tauri::Window;

/// Minimize the main window — `Api.window_minimize_to_tray`. Python's docstring
/// says "hide from taskbar", but the code just does `ShowWindow(SW_MINIMIZE)`, a
/// plain minimize; the always-running tray icon is what keeps the window one
/// click away. Faithful to the code, not the docstring.
#[tauri::command(async)]
pub fn window_minimize_to_tray(window: tauri::Window) -> Value {
    match window.minimize() {
        Ok(()) => json!({ "ok": true }),
        Err(e) => json!({ "ok": false, "error": e.to_string() }),
    }
}

/// After `hide()` is accepted, how long the desktop is given to redraw without
/// us in it. `hide()` only queues the request, and the capture backends hand
/// back the last presented frame, so a grab that races this comes back with
/// Clawmation still covering the thing the user was pointing at.
const SETTLE: Duration = Duration::from_millis(120);

/// Longest we wait for the hide itself to take effect before giving up on it
/// and grabbing anyway — a frame with the app in it beats a hung editor.
const HIDE_TIMEOUT: Duration = Duration::from_millis(500);

/// Run `f` with the app off screen, then put it back.
///
/// Everything the editor captures — the colour/region/template/surgical pickers
/// and the Test button — grabs the desktop as it is, and the desktop as it is
/// has Clawmation in front of the game. Without this the pickers ask the user to
/// point at a screenshot of Clawmation, and a text trigger tests against our own
/// UI instead of the words it was written for: the reason text guards appeared
/// never to match anything.
///
/// The window is restored by a drop guard, so a panic inside `f` cannot leave
/// the app invisible with only the tray icon to get it back.
pub fn with_window_hidden<T>(window: &Window, f: impl FnOnce() -> T) -> T {
    // Already out of the way (minimized to tray, say) — leave it exactly as the
    // user left it rather than "restoring" a window they deliberately put away.
    if !window.is_visible().unwrap_or(true) || window.hide().is_err() {
        return f();
    }
    let _restore = Restore(window);

    let deadline = Instant::now() + HIDE_TIMEOUT;
    while Instant::now() < deadline && window.is_visible().unwrap_or(false) {
        std::thread::sleep(Duration::from_millis(10));
    }
    std::thread::sleep(SETTLE);
    f()
}

struct Restore<'a>(&'a Window);

impl Drop for Restore<'_> {
    fn drop(&mut self) {
        let _ = self.0.show();
        let _ = self.0.set_focus();
    }
}
