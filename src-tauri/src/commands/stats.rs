//! Stats commands: the dashboard's aggregate play statistics and run log.
//!
//! Ports of `Api.get_stats_summary` and `get_run_history`. The summary composes
//! five sources: per-macro play counts ([`PlayStats::all`](crate::engine::stats::PlayStats::all)),
//! the macro-file count, the chain count, the guard total across all macros
//! ([`crate::commands::guards::count_all_guards`]), and the schedule count.

use serde_json::{json, Value};
use tauri::State;

use crate::paths;
use crate::state::AppState;

#[tauri::command(async)]
pub fn get_stats_summary(state: State<AppState>) -> Value {
    let all_stats = state.core.play_stats.all();
    let total_plays: i64 = all_stats.values().map(|s| s.count).sum();
    let macros_played = all_stats.values().filter(|s| s.count > 0).count();

    // The first macro with the strictly-highest count wins ties (Python's `>`).
    let mut most_played: Option<&str> = None;
    let mut most_count = 0i64;
    for (name, s) in &all_stats {
        if s.count > most_count {
            most_count = s.count;
            most_played = Some(name);
        }
    }

    json!({
        "total_plays": total_plays,
        "macros_played": macros_played,
        "most_played": most_played,
        "most_played_count": most_count,
        "total_macros": count_macro_files(),
        "total_chains": state.chains.list().len(),
        "total_guards": crate::commands::guards::count_all_guards(),
        "total_schedules": state.scheduler.list().len(),
    })
}

#[tauri::command(async)]
pub fn get_run_history(state: State<AppState>, limit: Option<usize>) -> Value {
    json!(state.core.play_stats.history(limit.unwrap_or(30)))
}

/// `len(list(MACROS_DIR.glob("*.json")))`: number of macro files on disk.
fn count_macro_files() -> usize {
    match std::fs::read_dir(paths::macros_dir()) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("json"))
            .count(),
        Err(_) => 0,
    }
}
