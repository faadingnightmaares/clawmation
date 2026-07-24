//! Clawmation — Rust/Tauri backend.
//!
//! A faithful port of the Python "Clawmation" macro recorder/player. This module
//! wires the managed application state and registers the Tauri command surface
//! (the 1:1 replacement for the pywebview `api.*` bridge).

mod commands;
mod core;
pub mod engine;
pub mod hardware;
mod logbuf;
mod models;
mod notify;
mod paths;
mod shell;
mod state;
mod util;

#[cfg(test)]
mod test_support;

use models::config::MacroConfig;
use shell::hotkeys::HotkeyBindings;
use state::AppState;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    paths::ensure_dirs();

    let app_state = AppState::new(MacroConfig::load());

    tauri::Builder::default()
        // Single-instance first: a duplicate launch is folded back into the
        // running app (focus its window and exit) before any window, hotkey, or
        // capture device is claimed — the port of `_acquire_single_instance` +
        // `_focus_existing_instance`.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            shell::tray::show_main_window(app);
        }))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    shell::hotkeys::on_shortcut(app, shortcut, event.state());
                })
                .build(),
        )
        .plugin(tauri_plugin_notification::init())
        // Exclude the indicator overlay from window-state persistence: it must
        // always come up hidden and repositioned top-right, never restored to a
        // saved spot, size, or visibility. (Python's overlay persisted nothing.)
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_denylist(&[shell::indicator::LABEL])
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(app_state)
        .manage(HotkeyBindings::default())
        .setup(|app| {
            // Bind the notifier's app handle, then start the backend-initiated
            // shell — global hotkeys from config and the system tray — mirroring
            // `launch_ui`'s `_register_hotkeys()` / `_start_tray()` startup calls.
            let handle = app.handle().clone();
            app.state::<AppState>().core.notifier.attach(handle.clone());
            shell::hotkeys::register_from_config(&handle);
            shell::tray::build(&handle)?;
            // Build the transparent pixel-cat overlay (hidden until record/play).
            // Like Python's `_start_indicator` try/except, a failure is logged and
            // startup continues — the overlay is optional and must never abort the
            // app. `indicator_alive` flips true only when the window really exists,
            // keeping `get_status`'s report honest (`_indicator is not None`).
            match shell::indicator::create(&handle) {
                Ok(()) => {
                    app.state::<AppState>().core.indicator.attach(handle.clone());
                    app.state::<AppState>().core.runtime.lock().unwrap().indicator_alive = true;
                }
                Err(e) => eprintln!("Clawmation: recording indicator unavailable: {e}"),
            }
            // Closing the main window quits the app, as in Python (`window_close`
            // ends `webview.start()`). The always-open indicator would otherwise
            // outlive it and keep the process running headless, so tear it down when
            // main closes and let Tauri exit on the last window. Minimize-to-tray
            // only *hides* main, so this fires solely on a real close.
            if let Some(main) = app.get_webview_window("main") {
                let close_handle = handle.clone();
                main.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { .. } = event {
                        if let Some(indicator) =
                            close_handle.get_webview_window(shell::indicator::LABEL)
                        {
                            let _ = indicator.close();
                        }
                    }
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::status::get_status,
            commands::config::get_config,
            commands::config::update_config,
            commands::config::get_data_paths,
            commands::config::open_data_folder,
            commands::macros::list_macros,
            commands::macros::delete_macro,
            commands::macros::bulk_delete,
            commands::macros::save_as_template,
            commands::macros::list_templates,
            commands::macros::create_from_template,
            commands::macros::delete_template,
            commands::macros::rename_macro,
            commands::macros::duplicate_macro,
            commands::macros::set_repeat,
            commands::macros::set_category,
            commands::macros::set_notes,
            commands::checkpoints::add_checkpoint,
            commands::checkpoints::remove_checkpoint,
            commands::checkpoints::list_checkpoints,
            commands::ai::macro_to_steps,
            commands::ai::steps_save,
            commands::ai::steps_run,
            commands::ai::steps_test,
            commands::record::start_record,
            commands::record::stop_record,
            commands::record::pause_record,
            commands::playback::play_macro,
            commands::playback::stop_playback,
            commands::playback::emergency_stop,
            commands::guards::guard_list,
            commands::guards::get_all_guard_counts,
            commands::guards::guard_save,
            commands::guards::guard_test,
            commands::guards::guard_pick_color,
            commands::guards::guard_pick_region,
            commands::guards::capture_template,
            commands::guards::add_template_image,
            commands::guards::surgical_capture,
            commands::vision::vision_save,
            commands::vision::vision_load,
            commands::vision::vision_start,
            commands::vision::vision_stop,
            commands::vision::vision_status,
            commands::scheduler::list_schedules,
            commands::scheduler::add_schedule,
            commands::scheduler::remove_schedule,
            commands::scheduler::set_schedule_enabled,
            commands::scheduler::schedule_chain,
            commands::chains::list_chains,
            commands::chains::add_chain,
            commands::chains::remove_chain,
            commands::chains::duplicate_chain,
            commands::chains::update_chain,
            commands::chains::run_chain,
            commands::chains::validate_chain,
            commands::chains::get_chain_duration,
            commands::chains::stop_chain,
            commands::chains::get_running_chain,
            commands::transfer::export_chain,
            commands::transfer::import_chain,
            commands::transfer::export_macro,
            commands::transfer::import_macro,
            commands::transfer::bulk_export,
            commands::transfer::export_bundle,
            commands::transfer::import_bundle,
            commands::stats::get_stats_summary,
            commands::stats::get_run_history,
            commands::misc::get_version,
            commands::misc::check_update,
            commands::window::window_minimize_to_tray,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
