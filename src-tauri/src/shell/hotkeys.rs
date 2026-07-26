//! Global record/play/stop hotkeys: the port of `Api._register_hotkeys` and its
//! three `keyboard.on_press_key` hooks.
//!
//! A single handler is wired into the global-shortcut plugin at build time; it
//! dispatches a fired shortcut to the matching action by comparing it against the
//! [`HotkeyBindings`] captured at registration. Re-registering (on startup and
//! after any `hotkey_*` config change) clears the old bindings first, mirroring
//! Python's `keyboard.unhook_all()`.
//!
//! One behavioral departure worth naming for the open-source README: Tauri's
//! global shortcut is *exclusive* (it consumes the key so the focused app never
//! sees it), whereas Python's `keyboard.on_press_key` hook was non-exclusive and
//! passed the key through. A hotkey that collides with the target game's own
//! binding is therefore swallowed here.

use std::str::FromStr;
use std::sync::Mutex;

use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::state::AppState;

/// The three currently-registered shortcuts, so the fired-shortcut handler can
/// tell record/play/stop apart. Managed by Tauri; filled by [`register_from_config`].
#[derive(Default)]
pub struct HotkeyBindings {
    inner: Mutex<Bindings>,
}

#[derive(Default)]
struct Bindings {
    record: Option<Shortcut>,
    play: Option<Shortcut>,
    stop: Option<Shortcut>,
}

/// (Re)register the record/play/stop global shortcuts from the live config:
/// `Api._register_hotkeys`. Clears any existing registration first, then parses
/// each configured key (upper-cased, e.g. `f9` → `F9`) and stores the ones that
/// took. A key that fails to parse or register is left unbound, matching
/// Python's best-effort `except Exception` around the whole block, but it is
/// also *returned*, so the settings UI can say so instead of handing the user a
/// shortcut that will never fire. A blank key is unbound on purpose, not a
/// failure.
pub fn register_from_config(app: &AppHandle) -> Vec<String> {
    let _ = app.global_shortcut().unregister_all();

    let (rec, play, stop) = {
        let state = app.state::<AppState>();
        let c = state.core.config.lock().unwrap();
        (
            c.hotkey_record.to_uppercase(),
            c.hotkey_play.to_uppercase(),
            c.hotkey_stop.to_uppercase(),
        )
    };

    let bindings = app.state::<HotkeyBindings>();
    let mut b = bindings.inner.lock().unwrap();
    b.record = register_one(app, &rec);
    b.play = register_one(app, &play);
    b.stop = register_one(app, &stop);

    [(&rec, &b.record), (&play, &b.play), (&stop, &b.stop)]
        .into_iter()
        .filter(|(key, slot)| !key.trim().is_empty() && slot.is_none())
        .map(|(key, _)| key.to_lowercase())
        .collect()
}

/// Drop every global shortcut until [`register_from_config`] runs again. The
/// settings UI calls this while it is listening for a keystroke: Tauri's global
/// shortcuts are exclusive, so pressing an already-bound key would fire its
/// action instead of reaching the webview to be captured.
pub fn suspend(app: &AppHandle) {
    let _ = app.global_shortcut().unregister_all();
    let bindings = app.state::<HotkeyBindings>();
    *bindings.inner.lock().unwrap() = Bindings::default();
}

fn register_one(app: &AppHandle, key: &str) -> Option<Shortcut> {
    let shortcut = Shortcut::from_str(key).ok()?;
    app.global_shortcut().register(shortcut).ok()?;
    Some(shortcut)
}

/// Dispatch a fired shortcut to the matching hotkey action, the handler wired
/// into the global-shortcut plugin. Only key-press edges act, since Python hooks
/// `on_press_key`.
pub fn on_shortcut(app: &AppHandle, shortcut: &Shortcut, state: ShortcutState) {
    if state != ShortcutState::Pressed {
        return;
    }
    let which = {
        let bindings = app.state::<HotkeyBindings>();
        let b = bindings.inner.lock().unwrap();
        if b.record.as_ref() == Some(shortcut) {
            Action::Record
        } else if b.play.as_ref() == Some(shortcut) {
            Action::Play
        } else if b.stop.as_ref() == Some(shortcut) {
            Action::Stop
        } else {
            return;
        }
    };
    let app_state = app.state::<AppState>();
    match which {
        Action::Record => app_state.core.hotkey_record(),
        // The play hotkey opens the macro launcher — a Raycast/PowerToys-style
        // picker listing every macro with its play count and time played — so the
        // user chooses what to run instead of the old "replay the last macro"
        // behavior. The tray's "Replay last macro" still calls `hotkey_play`.
        Action::Play => crate::shell::launcher::toggle(app),
        // The stop hotkey is the global emergency stop (`Api.emergency_stop`),
        // which also reaches the vision agent on `AppState`.
        Action::Stop => {
            crate::commands::playback::emergency_stop_impl(&app_state);
        }
    }
}

enum Action {
    Record,
    Play,
    Stop,
}
