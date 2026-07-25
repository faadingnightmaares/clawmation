//! Tray toast notifications with a shared 30-second cooldown, the Rust
//! analogue of Python's `SystemTray.notify` anti-fatigue throttle.
//!
//! One [`Notifier`] is held on [`Core`](crate::core::Core) and shared by all
//! three notifications (playback-complete, scheduled macro, scheduled chain), so
//! they throttle against a single timer exactly as the Python `SystemTray` keeps
//! one `_last_notify_time`.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

/// Seconds between toasts (Microsoft anti-fatigue guidance):
/// `SystemTray.NOTIFY_COOLDOWN`.
const NOTIFY_COOLDOWN: Duration = Duration::from_secs(30);

/// Holds the app handle (bound in `setup()`, so a toast fired before the UI is up
/// no-ops like Python's `if not self._icon`) and the last-shown instant that all
/// notifications throttle against.
#[derive(Default)]
pub struct Notifier {
    app: Mutex<Option<AppHandle>>,
    last: Mutex<Option<Instant>>,
}

impl Notifier {
    pub fn new() -> Self {
        Self::default()
    }

    /// Bind the app handle once the Tauri app is built (`setup`).
    pub fn attach(&self, app: AppHandle) {
        *self.app.lock().unwrap() = Some(app);
    }

    /// Show a toast, enforcing the shared cooldown (`SystemTray.notify`). No-ops
    /// if the handle isn't attached yet or the cooldown is active; the timer
    /// advances only on a successful show (Python updates `_last_notify_time`
    /// only after `_icon.notify` succeeds).
    pub fn notify(&self, title: &str, message: &str) {
        let app = match self.app.lock().unwrap().clone() {
            Some(app) => app,
            None => return,
        };
        let mut last = self.last.lock().unwrap();
        if let Some(t) = *last {
            if t.elapsed() < NOTIFY_COOLDOWN {
                return;
            }
        }
        if app
            .notification()
            .builder()
            .title(title)
            .body(message)
            .show()
            .is_ok()
        {
            *last = Some(Instant::now());
        }
    }
}
