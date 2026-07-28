//! Clawmation: Rust/Tauri backend.
//!
//! A faithful port of the Python "Clawmation" macro recorder/player. This module
//! wires the managed application state and registers the Tauri command surface
//! (the 1:1 replacement for the pywebview `api.*` bridge).

mod commands;
mod core;
pub mod engine;
pub mod hardware;
mod logbuf;
mod migrations;
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
use tauri::{Emitter, Manager};
use tauri_plugin_window_state::StateFlags;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Per-Monitor-V2 for the whole process BEFORE any thread, window, or
    // capture exists: every thread spawned afterwards then records and replays
    // in physical pixels at any display scaling. Not leaning on tao raising it
    // at event-loop creation — see hardware::dpi for the full guarantee.
    hardware::dpi::raise_process_to_per_monitor_v2();
    paths::ensure_dirs();

    let app_state = AppState::new(MacroConfig::load());
    let startup_arguments: Vec<String> = std::env::args().collect();
    let startup_cwd = std::env::current_dir().ok();
    let migration = migrations::migrate_legacy_macros(&paths::macros_dir());
    if let Some(summary) = migration.summary() {
        app_state.core.emit(
            if migration.errors.is_empty() {
                "ok"
            } else {
                "warn"
            },
            summary,
        );
    }
    for error in migration.errors {
        app_state.core.emit(
            "err",
            format!("Macro upgrade left a file unchanged: {error}"),
        );
    }

    tauri::Builder::default()
        // Single-instance first: a duplicate launch is folded back into the
        // running app (focus its window and exit) before any window, hotkey, or
        // capture device is claimed, the port of `_acquire_single_instance` +
        // `_focus_existing_instance`.
        .plugin(tauri_plugin_single_instance::init(|app, argv, cwd| {
            import_associated_files(app, &argv, Some(std::path::Path::new(&cwd)));
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
        //
        // DECORATIONS is dropped from the saved flags: the main window is
        // undecorated and draws its own title bar, and a state file written
        // before that change would otherwise restore the native caption on top
        // of ours at every launch.
        //
        // VISIBLE is dropped too, because closing now hides the window to the
        // tray: quitting from the tray while it is away would otherwise persist
        // `visible: false` and the next launch would start with no window at all.
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(
                    StateFlags::all() & !StateFlags::DECORATIONS & !StateFlags::VISIBLE,
                )
                .with_denylist(&[
                    shell::indicator::LABEL,
                    shell::detections::LABEL,
                    shell::launcher::LABEL,
                ])
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(app_state)
        .manage(HotkeyBindings::default())
        .setup(move |app| {
            // Bind the notifier's app handle, then start the backend-initiated
            // shell (global hotkeys from config and the system tray), mirroring
            // `launch_ui`'s `_register_hotkeys()` / `_start_tray()` startup calls.
            let handle = app.handle().clone();
            app.state::<AppState>().core.notifier.attach(handle.clone());
            let _ = shell::hotkeys::register_from_config(&handle);
            shell::tray::build(&handle)?;
            // Build the transparent pixel-cat overlay (hidden until record/play).
            // Like Python's `_start_indicator` try/except, a failure is logged and
            // startup continues; the overlay is optional and must never abort the
            // app. `indicator_alive` flips true only when the window really exists,
            // keeping `get_status`'s report honest (`_indicator is not None`).
            // Attach before creation so a transient creation failure can be
            // retried automatically on the next record/play transition.
            app.state::<AppState>()
                .core
                .indicator
                .attach(handle.clone());
            match shell::indicator::create(&handle) {
                Ok(()) => {
                    app.state::<AppState>()
                        .core
                        .runtime
                        .lock()
                        .unwrap()
                        .indicator_alive = true;
                }
                Err(e) => eprintln!("Clawmation: recording indicator unavailable: {e}"),
            }
            // The live detection overlay, on the same terms: hidden until a
            // detection loop arms it, and optional: without it the triggers run
            // exactly as before, just unwatched.
            match shell::detections::create(&handle) {
                Ok(()) => app
                    .state::<AppState>()
                    .core
                    .detections
                    .attach(handle.clone()),
                Err(e) => eprintln!("Clawmation: detection overlay unavailable: {e}"),
            }
            // The macro launcher (the play hotkey's Raycast-style picker), built
            // hidden and centered; the hotkey toggles it. Optional like the
            // overlays: a failure is logged and startup continues, the hotkey just
            // won't open a panel.
            if let Err(e) = shell::launcher::create(&handle) {
                eprintln!("Clawmation: macro launcher unavailable: {e}");
            }
            // The launcher dismisses itself the moment focus leaves it — click
            // back into a game, Alt-Tab away — the Raycast behavior that keeps a
            // hotkey-summoned palette from lingering over the thing you summoned
            // it to use. Hiding on blur is idempotent, so the hotkey's own
            // dismiss (which also hides) double-firing is harmless.
            if let Some(launcher) = app.get_webview_window(shell::launcher::LABEL) {
                let blur_handle = handle.clone();
                launcher.on_window_event(move |event| {
                    if matches!(event, tauri::WindowEvent::Focused(false)) {
                        shell::launcher::hide(&blur_handle);
                    }
                });
            }
            import_associated_files(&handle, &startup_arguments, startup_cwd.as_deref());
            // Look for a new release once, off the startup path. Nothing waits on
            // it and a failure is silent; an offline machine must still start.
            commands::misc::check_in_background(&handle);
            // Close puts Clawmation away instead of quitting it: a macro, a guard
            // or a scheduled chain is usually still running, and killing the app
            // out from under a farming loop is never what the X meant. The window
            // hides, the tray icon brings it back, and Quit on the tray menu is
            // the one path that actually ends the process.
            //
            // The window-state plugin has its own CloseRequested handler that
            // banks the geometry, and it runs regardless of `prevent_close`, so
            // where you left the window survives the trip to the tray.
            if let Some(main) = app.get_webview_window("main") {
                let hide_handle = handle.clone();
                main.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        if let Some(main) = hide_handle.get_webview_window("main") {
                            let _ = main.hide();
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
            commands::config::hotkeys_suspend,
            commands::config::hotkeys_resume,
            commands::config::get_data_paths,
            commands::config::open_data_folder,
            commands::anti_afk::anti_afk_list_windows,
            commands::anti_afk::anti_afk_get,
            commands::anti_afk::anti_afk_update,
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
            commands::ai::node_graph_list,
            commands::ai::node_graph_create,
            commands::ai::node_graph_load,
            commands::ai::node_graph_rename,
            commands::ai::node_graph_delete,
            commands::ai::node_graph_validate,
            commands::ai::node_graph_save,
            commands::ai::node_graph_run,
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
            commands::guards::save_template_upload,
            commands::guards::surgical_capture,
            commands::vision::vision_save,
            commands::vision::vision_load,
            commands::vision::vision_start,
            commands::vision::vision_stop,
            commands::vision::vision_status,
            commands::vision::get_detections,
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
            commands::misc::install_update,
            commands::misc::dpi_report,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn import_associated_files(
    app: &tauri::AppHandle,
    arguments: &[String],
    cwd: Option<&std::path::Path>,
) {
    for (path, result) in commands::transfer::import_associated_arguments(arguments, cwd) {
        match result {
            Ok(name) => {
                app.state::<AppState>()
                    .emit("ok", format!("Imported '{name}' from {}", path.display()));
                let _ = app.emit("macros-changed", name);
            }
            Err(error) => app.state::<AppState>().emit(
                "err",
                format!("Couldn't import {}: {error}", path.display()),
            ),
        }
    }
}
