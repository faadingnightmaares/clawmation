//! Status heartbeat command.

use tauri::State;

use crate::logbuf;
use crate::models::status::{CaptureStatus, Status, StatusConfig, WindowStatus};
use crate::paths;
use crate::state::AppState;
use crate::util::round1;

#[tauri::command(async)]
pub fn get_status(state: State<AppState>) -> Status {
    let core = &state.core;
    let config = core.config.lock().unwrap().clone();
    let log = core.log.lock().unwrap().tail(logbuf::TAIL);

    let (mode, mode_since, last_macro, stored_recorded) = {
        let rt = core.runtime.lock().unwrap();
        (
            rt.mode.clone(),
            rt.mode_since,
            rt.last_macro.clone(),
            rt.recorded_count,
        )
    };

    // `elapsed` only advances while recording or playing (from `_mode_since`).
    let elapsed = match (mode.as_str(), mode_since) {
        ("recording" | "playing", Some(since)) => round1(since.elapsed().as_secs_f64()),
        _ => 0.0,
    };

    // Play counters are live off the player only while playing; the recorded
    // count and paused flag come off the live recorder only while recording,
    // exactly as `Api.get_status` reads them from the player/recorder objects.
    let (play_iteration, play_total_reps) = if mode == "playing" {
        (
            core.player.iteration() as i64,
            core.player.total_reps() as i64,
        )
    } else {
        (0, 0)
    };
    let (recorded_count, record_paused) = if mode == "recording" {
        match core.recorder.lock().unwrap().as_ref() {
            Some(rec) => (rec.event_count() as i64, rec.is_paused()),
            None => (stored_recorded, false),
        }
    } else {
        (stored_recorded, false)
    };

    // Resolved backend once capture has opened; the configured name until then.
    let capture_backend = core
        .vision
        .resolved_backend()
        .unwrap_or_else(|| config.capture_backend.clone());
    let capture_fps = core.vision.capture_fps_cached();

    Status {
        mode,
        elapsed,
        record_paused,
        play_iteration,
        play_total_reps,
        window: WindowStatus {
            found: false,
            title: String::new(),
            size: String::new(),
        },
        capture: CaptureStatus {
            backend: capture_backend,
            fps: capture_fps,
        },
        recorded_count,
        last_macro,
        macro_count: paths::count_ext(&paths::macros_dir(), "json"),
        indicator_alive: core.indicator.is_alive(),
        config: StatusConfig {
            resolution: [config.resolution.0, config.resolution.1],
            capture_backend: config.capture_backend.clone(),
            hotkey_record: config.hotkey_record.clone(),
            hotkey_play: config.hotkey_play.clone(),
            hotkey_stop: config.hotkey_stop.clone(),
            indicator_on_top: config.indicator_on_top,
        },
        log,
    }
}
