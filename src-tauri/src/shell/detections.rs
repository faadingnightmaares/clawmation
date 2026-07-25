//! The live detection overlay: a box drawn round everything the triggers are
//! finding, while they are finding it.
//!
//! Until now the detection loops were invisible: a trigger either fired or it
//! did not, and a trigger that was matching the wrong thing looked exactly like
//! one that was matching nothing. This module owns a fullscreen transparent
//! click-through window whose page (`src/detections/detections.ts`) polls
//! [`get_detections`](crate::commands::vision::get_detections) and draws the last
//! pass, so the user watches the detector work the way an object detector's demo
//! video shows its boxes.
//!
//! The window is only up while a detection loop is running ([`Detections::set`]
//! is called by the guard engine's start/stop and by the watcher's), and it is
//! kept out of the capture path, without which it would be a feedback loop: the
//! boxes would land in the next frame the detector reads.

use std::collections::HashSet;
use std::sync::Mutex;

use tauri::{
    AppHandle, LogicalPosition, LogicalSize, Manager, WebviewUrl, WebviewWindowBuilder,
};

use crate::hardware::shield;

/// Window label; the `detections` capability and the render page key off it.
pub const LABEL: &str = "detections";

/// Holds the app handle, bound in `setup()` once the overlay exists, and the set
/// of detection loops currently running. A `set` before the window is built (or
/// in tests, where none is) finds no handle and only tracks the flag.
#[derive(Default)]
pub struct Detections {
    app: Mutex<Option<AppHandle>>,
    /// Which loops are detecting right now. Tracked as a set rather than a flag
    /// because the guard engine and the watcher run independently: a macro
    /// finishing must not take the boxes away from a watcher that is still going.
    sources: Mutex<HashSet<&'static str>>,
}

impl Detections {
    pub fn new() -> Self {
        Self::default()
    }

    /// Bind the app handle once the Tauri app is built (`setup`).
    pub fn attach(&self, app: AppHandle) {
        *self.app.lock().unwrap() = Some(app);
    }

    /// Arm or disarm one detection loop by name; the overlay is up while any of
    /// them is armed.
    pub fn set(&self, source: &'static str, on: bool) {
        let live = {
            let mut sources = self.sources.lock().unwrap();
            if on {
                sources.insert(source);
            } else {
                sources.remove(source);
            }
            !sources.is_empty()
        };
        self.show(live);
    }

    /// Queue the show or hide onto the UI thread and return.
    ///
    /// Every caller here is a detection loop arming or disarming, on whatever
    /// thread happened to run the command. Showing a window is a round trip to
    /// the UI thread, so doing it inline makes the caller wait on a thread it
    /// is also competing with for the CPU. That is what left `vision_start`
    /// hanging and the Start button spinning: the watcher was already running
    /// flat out by the time the overlay was asked for, and the answer never
    /// came back. The overlay is a view onto the loops, never a step in
    /// starting one, so nothing is lost by letting it catch up on its own.
    fn show(&self, live: bool) {
        let Some(app) = self.app.lock().unwrap().clone() else { return };
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || {
            let Some(win) = handle.get_webview_window(LABEL) else { return };
            if !live {
                let _ = win.hide();
                return;
            }
            // The exclusion is re-asserted at the moment it matters, and the
            // overlay stays down without it: boxes a screen grab can see are
            // boxes the next detection pass reads back as part of the game.
            let Ok(hwnd) = win.hwnd() else { return };
            if !shield::set_excluded(hwnd.0, true) {
                return;
            }
            let _ = win.show();
        });
    }
}

/// Build the overlay once: hidden, covering the primary monitor.
///
/// Same ex-styles as the recording indicator (transparent, no-activate,
/// off-taskbar, topmost, click-through), sized to the whole screen because the
/// boxes land wherever the triggers match. Errors bubble to `setup`, which logs
/// and carries on: the overlay is a view onto the detection loops, never a
/// condition for running them.
pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let win = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("detections.html".into()))
        .title("Clawmation Detections")
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
    win.set_ignore_cursor_events(true)?;
    if let Ok(Some(monitor)) = win.primary_monitor() {
        let scale = monitor.scale_factor();
        let origin = monitor.position().to_logical::<f64>(scale);
        let size = monitor.size().to_logical::<f64>(scale);
        let _ = win.set_position(LogicalPosition::new(origin.x, origin.y));
        let _ = win.set_size(LogicalSize::new(size.width, size.height));
    }
    // Take it out of the capture path up front as well as at every show, so a
    // grab that races the first arm cannot catch it.
    let _ = shield::set_excluded(win.hwnd()?.0, true);
    Ok(())
}
