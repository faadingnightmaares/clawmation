//! The macro launcher: a Raycast/PowerToys-style command palette the play hotkey
//! toggles. Where Python's play hotkey fired the last macro outright, this opens
//! a small always-on-top panel listing every macro and chain with its play count
//! and time played; the user picks one with the mouse or the arrow keys and Enter
//! runs it. The panel hides the moment it loses focus, so clicking back into a
//! game dismisses it exactly like Raycast.
//!
//! It is a real, focusable window (not a transparent click-through overlay like
//! the indicator), built once hidden at startup and shown/centered on demand. On
//! a multi-monitor setup it opens centered on the screen under the cursor — the
//! one the user is actually looking at — not always the primary.

use tauri::Manager;
use tauri::{AppHandle, Monitor, PhysicalPosition, WebviewUrl, WebviewWindowBuilder};

/// Window label; the `launcher` capability and this module key off it.
pub const LABEL: &str = "launcher";

/// Panel size, logical px: a command-palette column, wide enough for a macro
/// name plus its stats, tall enough for ~8 rows before scrolling.
const WIDTH: f64 = 640.0;
const HEIGHT: f64 = 460.0;

/// Build the panel once, hidden and centered. Frameless and skip-taskbar so it
/// reads as a popup, not an app window; focusable and always-on-top so the hotkey
/// can pull it up over a fullscreen game. Not transparent and not click-through —
/// unlike the indicator, this one takes keyboard and mouse. A failure is logged
/// by the caller in `setup` and startup continues; the launcher is optional.
pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let win = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("launcher.html".into()))
        .title("Clawmation Launcher")
        .inner_size(WIDTH, HEIGHT)
        .decorations(false)
        .resizable(false)
        .skip_taskbar(true)
        .always_on_top(true)
        .visible(false)
        .build()?;
    center(app, &win);
    Ok(())
}

/// Open the launcher if it is hidden, dismiss it if it is showing — the play
/// hotkey's action. Showing re-centers on the monitor under the cursor and takes
/// focus; the page reloads its list on focus, so what you see is always current.
pub fn toggle(app: &AppHandle) {
    let Some(win) = app.get_webview_window(LABEL) else {
        return;
    };
    if win.is_visible().unwrap_or(false) {
        let _ = win.hide();
    } else {
        center(app, &win);
        let _ = win.show();
        let _ = win.set_focus();
    }
}

/// Hide the panel (called on blur and after a macro is launched).
pub fn hide(app: &AppHandle) {
    if let Some(win) = app.get_webview_window(LABEL) {
        let _ = win.hide();
    }
}

/// Center the panel on the monitor the user is on. Cursor monitor first (so a
/// dual-screen setup lands the palette on whichever display the hotkey was
/// pressed from), then the primary monitor as a fallback. All math is done in
/// physical pixels against the virtual-screen origin so per-monitor DPI scaling
/// and non-primary origins (a monitor to the left of the primary has negative x)
/// place it correctly without per-scale conversions.
fn center(app: &AppHandle, win: &tauri::WebviewWindow) {
    let Some(monitor) = cursor_monitor(app).or_else(|| app.primary_monitor().ok().flatten()) else {
        return;
    };
    let scale = monitor.scale_factor();
    let pos = monitor.position();
    let size = monitor.size();
    let x = pos.x as f64 + (size.width as f64 - WIDTH * scale) / 2.0;
    let y = pos.y as f64 + (size.height as f64 - HEIGHT * scale) / 2.0;
    let _ = win.set_position(PhysicalPosition::new(x, y));
}

/// The monitor currently containing the mouse cursor, if any. Cursor and monitor
/// positions are both physical pixels relative to the same virtual-screen origin,
/// so a simple bounds test picks the right display.
fn cursor_monitor(app: &AppHandle) -> Option<Monitor> {
    let cursor = app.cursor_position().ok()?;
    app.available_monitors().ok()?.into_iter().find(|m| {
        let p = m.position();
        let s = m.size();
        cursor.x >= p.x as f64
            && cursor.x < p.x as f64 + s.width as f64
            && cursor.y >= p.y as f64
            && cursor.y < p.y as f64 + s.height as f64
    })
}
