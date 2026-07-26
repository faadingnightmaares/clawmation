//! Desktop shell wiring that lives outside the command surface: global hotkeys
//! and the system tray. These are the backend-initiated features Python starts
//! in `launch_ui` (`_register_hotkeys`, `_start_tray`), driven from Rust with
//! no JS round-trip, so they have no `#[tauri::command]` of their own.

pub mod detections;
pub mod hotkeys;
pub mod indicator;
pub mod launcher;
pub mod tray;
