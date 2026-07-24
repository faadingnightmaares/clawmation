//! The transparent pixel-cat recording indicator — the Rust seat of Python's
//! `NativeIndicator` (`overlay.py`) and the `_sync_indicator` glue that shows it.
//!
//! Python drew the cat itself into a Win32 layered window; here the drawing lives
//! in the `indicator.html` webview and this module owns only the *window*. It
//! builds one transparent, click-through, no-activate overlay at startup (hidden,
//! top-right), and [`Indicator::sync`] shows it for recording/playing/paused and
//! hides it at idle — driven from [`Core::set_mode`](crate::core::Core::set_mode),
//! the exact spot where `ui_app._set_mode` calls `_sync_indicator`.

use std::sync::Mutex;

use tauri::{AppHandle, LogicalPosition, Manager, WebviewUrl, WebviewWindowBuilder};

/// Window label; the `indicator` capability and the render page key off it.
pub const LABEL: &str = "indicator";

/// Overlay size: the 72×66 pixel-cat canvas at 2× so the counting eyes stay
/// legible in the corner. (`NativeIndicator` used the literal 72×66; the art is
/// the same, just shown larger — see the page's `image-rendering: pixelated`.)
const WIDTH: f64 = 144.0;
const HEIGHT: f64 = 132.0;
/// Gap from the top-right screen corner — `NativeIndicator.MARGIN`.
const MARGIN: f64 = 16.0;

/// Holds the app handle, bound in `setup()` once the overlay exists. A `sync`
/// before that (or in tests, where no window is built) finds `None` and no-ops —
/// the analogue of Python's `if self._hwnd:` guard around show/hide.
#[derive(Default)]
pub struct Indicator {
    app: Mutex<Option<AppHandle>>,
}

impl Indicator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Bind the app handle once the Tauri app is built (`setup`).
    pub fn attach(&self, app: AppHandle) {
        *self.app.lock().unwrap() = Some(app);
    }

    /// Show the cat for recording/playing/paused, hide it at idle — the port of
    /// `NativeIndicator.set_state`'s show/hide branch. The page reads mode and
    /// elapsed straight off `get_status`, so this only toggles window visibility;
    /// an unattached handle (pre-`setup`, or tests) no-ops.
    pub fn sync(&self, mode: &str) {
        let app = match self.app.lock().unwrap().clone() {
            Some(app) => app,
            None => return,
        };
        if let Some(win) = app.get_webview_window(LABEL) {
            let active = matches!(mode, "recording" | "playing" | "paused");
            let _ = if active { win.show() } else { win.hide() };
        }
    }
}

/// Build the overlay once — hidden, top-right of the primary monitor. The builder
/// flags reproduce `overlay.py`'s Win32 ex-styles: `transparent` (WS_EX_LAYERED
/// per-pixel alpha), `focusable(false)` (WS_EX_NOACTIVATE — never steals focus
/// from the game), `skip_taskbar` (WS_EX_TOOLWINDOW — no taskbar/Alt-Tab entry),
/// `always_on_top` (WS_EX_TOPMOST), and `set_ignore_cursor_events` (WS_EX_TRANSPARENT
/// click-through). It starts hidden; `set_mode` reveals it. Errors bubble to the
/// caller in `setup`, which logs and continues — a failed overlay must not abort
/// startup, matching `_run`'s try/except that leaves the indicator absent.
pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let win = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("indicator.html".into()))
        .title("Clawmation Indicator")
        .inner_size(WIDTH, HEIGHT)
        .transparent(true)
        .decorations(false)
        .shadow(false)
        .resizable(false)
        .skip_taskbar(true)
        .always_on_top(true)
        .focusable(false)
        .focused(false)
        .visible(false)
        .build()?;
    // Click-through: mouse events fall straight through to the game below.
    win.set_ignore_cursor_events(true)?;
    // Park it top-right with MARGIN, in logical px (`sw - WIDTH - MARGIN`, `MARGIN`
    // from `_create_window`). Done after build — still hidden, so no flash — since
    // the placement depends on the monitor the window actually landed on.
    if let Ok(Some(monitor)) = win.primary_monitor() {
        let logical_w = monitor.size().width as f64 / monitor.scale_factor();
        let x = (logical_w - WIDTH - MARGIN).max(0.0);
        let _ = win.set_position(LogicalPosition::new(x, MARGIN));
    }
    Ok(())
}
