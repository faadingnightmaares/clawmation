//! System tray icon + menu: the port of `Api._start_tray` and `SystemTray._run`.
//!
//! Menu: "Show Clawmation" (the default left-click action), "Replay last macro",
//! a separator, then "Quit". Left-clicking the icon restores the window; the
//! item labels match the Python tray character-for-character.
//!
//! Quit is the only way out of the app: the window's close button hides to the
//! tray rather than ending the process (see the `CloseRequested` handler in
//! `lib.rs`), so this menu is the exit.

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

use crate::state::AppState;

/// Build and register the tray icon. Called once from `setup`.
pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Show Clawmation", true, None::<&str>)?;
    let replay = MenuItem::with_id(app, "replay", "Replay last macro", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &replay, &sep, &quit])?;

    TrayIconBuilder::with_id("clawmation")
        .icon(app.default_window_icon().unwrap().clone())
        .tooltip("Clawmation: Macro Recorder")
        .menu(&menu)
        // Left-click restores the window (pystray's `default=True` item); the menu
        // opens on right-click, Tauri's default.
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_main_window(app),
            "replay" => app.state::<AppState>().core.hotkey_play(),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

/// Restore and focus the main window: `Api._tray_show` (SW_RESTORE +
/// SetForegroundWindow). Also used by the single-instance callback when a second
/// launch is folded back into the running one.
pub fn show_main_window(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
    }
}
