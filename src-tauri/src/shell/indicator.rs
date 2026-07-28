//! The transparent pixel-cat recording indicator, the Rust seat of Python's
//! `NativeIndicator` (`overlay.py`) and the `_sync_indicator` glue that shows it.
//!
//! Python drew the cat itself into a Win32 layered window; here the drawing lives
//! in the `indicator.html` webview and this module owns only the *window*. It
//! builds one transparent, click-through, no-activate overlay at startup (hidden,
//! top-right), and [`Indicator::sync`] shows it for recording/playing/paused and
//! hides it at idle, driven from [`Core::set_mode`](crate::core::Core::set_mode),
//! the exact spot where `ui_app._set_mode` calls `_sync_indicator`.

use std::sync::Mutex;

use tauri::{AppHandle, LogicalPosition, Manager, WebviewUrl, WebviewWindowBuilder};

/// Window label; the `indicator` capability and the render page key off it.
pub const LABEL: &str = "indicator";

/// Overlay size: the compact 96×88 hanging-cat canvas (`src/indicator/cat.ts`) at
/// 2× so the counting eyes stay legible; see `indicator.html`.
const WIDTH: f64 = 192.0;
const HEIGHT: f64 = 176.0;
/// Gap from the right screen edge (`NativeIndicator.MARGIN`). Horizontal only:
/// see the vertical placement in [`create`].
const MARGIN: f64 = 16.0;

/// Holds the app handle, bound in `setup()` once the overlay exists. A `sync`
/// before that (or in tests, where no window is built) finds `None` and no-ops,
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

    /// Whether the overlay currently exists. Unlike the old latched runtime flag,
    /// this reflects the live window and therefore becomes true after a self-heal.
    pub fn is_alive(&self) -> bool {
        self.app
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|app| app.get_webview_window(LABEL).is_some())
    }

    /// Show the cat for recording/playing/paused, hide it at idle: the port of
    /// `NativeIndicator.set_state`'s show/hide branch. The page reads mode and
    /// elapsed straight off `get_status`, so this only toggles window visibility;
    /// an unattached handle (pre-`setup`, or tests) no-ops.
    pub fn sync(&self, mode: &str, enabled: bool) {
        let app = match self.app.lock().unwrap().clone() {
            Some(app) => app,
            None => return,
        };
        let show = should_show(mode, enabled);
        if let Some(win) = app.get_webview_window(LABEL) {
            let _ = if show { win.show() } else { win.hide() };
            return;
        }

        // A transient WebView/Win32 setup failure used to permanently remove the
        // indicator for the rest of the process. Recreate it on the next active
        // transition instead; idle/disabled states do not allocate a window.
        if show {
            if let Err(error) = create(&app) {
                eprintln!("Clawmation: recording indicator retry failed: {error}");
                return;
            }
            if let Some(win) = app.get_webview_window(LABEL) {
                let _ = win.show();
            }
        }
    }
}

fn should_show(mode: &str, enabled: bool) -> bool {
    enabled && matches!(mode, "recording" | "playing" | "paused")
}

/// Build the overlay once: hidden, top-right of the primary monitor. The builder
/// flags reproduce `overlay.py`'s Win32 ex-styles: `transparent` (WS_EX_LAYERED
/// per-pixel alpha), `focusable(false)` (WS_EX_NOACTIVATE, never steals focus
/// from the game), `skip_taskbar` (WS_EX_TOOLWINDOW, no taskbar/Alt-Tab entry),
/// `always_on_top` (WS_EX_TOPMOST), and `set_ignore_cursor_events` (WS_EX_TRANSPARENT
/// click-through). It starts hidden; `set_mode` reveals it. Errors bubble to the
/// caller in `setup`, which logs and continues; a failed overlay must not abort
/// startup, matching `_run`'s try/except that leaves the indicator absent.
pub fn create(app: &AppHandle) -> tauri::Result<()> {
    if app.get_webview_window(LABEL).is_some() {
        return Ok(());
    }
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
    // These refinements are best-effort. The old implementation returned an
    // error after the WebView had already been built, so setup never attached
    // the live window and every later show/hide became a permanent no-op.
    if let Err(error) = win.set_ignore_cursor_events(true) {
        eprintln!("Clawmation: indicator click-through unavailable: {error}");
    }
    // And out of every screen grab: the cat sits over the top-right corner of the
    // screen the whole time a macro plays, which is the corner a trigger watching
    // that region is trying to read.
    match win.hwnd() {
        Ok(hwnd) => {
            let _ = crate::hardware::shield::set_excluded(hwnd.0, true);
        }
        Err(error) => eprintln!("Clawmation: indicator capture shielding unavailable: {error}"),
    }
    // Park it against the top-right corner, in logical px. Inset from the right by
    // MARGIN, but flush to the top at y = 0: the cat's paws and tail are drawn
    // already cut off by the canvas edge, and only the physical screen edge makes
    // that cut read as a ledge. A vertical gap would leave the paws in mid-air.
    // Done after build (still hidden, so no flash), since the placement depends on
    // the monitor the window actually landed on.
    if let Ok(Some(monitor)) = win.primary_monitor() {
        let logical_w = monitor.size().width as f64 / monitor.scale_factor();
        let x = (logical_w - WIDTH - MARGIN).max(0.0);
        let _ = win.set_position(LogicalPosition::new(x, 0.0));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{should_show, HEIGHT, WIDTH};

    #[test]
    fn preference_hides_the_cat_even_while_active() {
        assert!(should_show("playing", true));
        assert!(should_show("recording", true));
        assert!(!should_show("playing", false));
        assert!(!should_show("idle", true));
    }

    #[test]
    fn every_active_mode_requests_a_visible_indicator() {
        for mode in ["recording", "playing", "paused"] {
            assert!(should_show(mode, true), "{mode} must self-heal and show");
        }
        for mode in ["idle", "stopping", ""] {
            assert!(!should_show(mode, true), "{mode} must remain hidden");
        }
    }

    #[test]
    fn native_window_matches_the_two_x_canvas_size() {
        assert_eq!(WIDTH, 96.0 * 2.0);
        assert_eq!(HEIGHT, 88.0 * 2.0);
    }
}
