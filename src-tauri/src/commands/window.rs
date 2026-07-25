//! The "get out of the way" helper every screen grab the editor starts runs
//! behind.

use std::ffi::c_void;
use std::time::{Duration, Instant};

use tauri::Window;

use crate::hardware::shield;

/// After we are out of the capture, how long the desktop is given to redraw
/// without us in it. Neither route takes effect on the instruction that asks for
/// it (a hide is a window message, an exclusion is a DWM recomposition), and
/// the capture backends hand back the last presented frame, so a grab that races
/// this comes back with Clawmation still covering the thing the user was
/// pointing at.
const SETTLE: Duration = Duration::from_millis(120);

/// Longest we wait for a hide to take effect before giving up on it and grabbing
/// anyway; a frame with the app in it beats a hung editor.
const HIDE_TIMEOUT: Duration = Duration::from_millis(500);

/// Run `f` with Clawmation absent from anything that captures the screen.
///
/// Everything the editor captures (the colour/region/template/surgical pickers
/// and the Test button) grabs the desktop as it is, and the desktop as it is
/// has Clawmation in front of the game. Without this the pickers ask the user to
/// point at a screenshot of Clawmation, and a text trigger tests against our own
/// UI instead of the words it was written for: the reason text guards appeared
/// never to match anything.
///
/// Two routes to the same absence. [`shield`] asks Windows to leave the window
/// out of the capture path, which changes nothing the user can see; only where
/// the platform has no such thing does the window actually get hidden and shown
/// again, and that flicker (the app apparently quitting and relaunching every
/// time Test is pressed) is why the shield is tried first.
///
/// Either way the window is put back by a drop guard, so a panic inside `f`
/// cannot leave the app invisible with only the tray icon to get it back.
pub fn with_window_out_of_frame<T>(window: &Window, f: impl FnOnce() -> T) -> T {
    // Already out of the way (closed to the tray, say): leave it exactly as the
    // user left it rather than "restoring" a window they deliberately put away.
    if !window.is_visible().unwrap_or(true) {
        return f();
    }
    match Shield::raise(window) {
        Some(_shield) => {
            std::thread::sleep(SETTLE);
            f()
        }
        None => with_window_hidden(window, f),
    }
}

/// The capture exclusion, lifted again when it goes out of scope: a window left
/// permanently unscreenshottable would break the user's own screen recorder, not
/// just ours.
struct Shield(*mut c_void);

impl Shield {
    fn raise(window: &Window) -> Option<Self> {
        let hwnd = window.hwnd().ok()?.0;
        shield::set_excluded(hwnd, true).then_some(Self(hwnd))
    }
}

impl Drop for Shield {
    fn drop(&mut self) {
        shield::set_excluded(self.0, false);
    }
}

/// The fallback route: off screen entirely, then back.
fn with_window_hidden<T>(window: &Window, f: impl FnOnce() -> T) -> T {
    if window.hide().is_err() {
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
