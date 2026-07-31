//! Stats commands: the dashboard's aggregate play statistics and run log.
//!
//! Ports of `Api.get_stats_summary` and `get_run_history`. The summary composes
//! five sources: per-macro play counts ([`PlayStats::all`](crate::engine::stats::PlayStats::all)),
//! the macro-file count, the chain count, the guard total across all macros
//! ([`crate::commands::guards::count_all_guards`]), and the schedule count.

use std::collections::HashSet;
use std::path::Path;

use serde_json::{json, Value};
use tauri::State;

use crate::engine::stats::PlayStats;
use crate::models::stats::HistoryEntry;
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
    json!(run_history_in(
        &state.core.play_stats,
        &paths::macros_dir(),
        limit.unwrap_or(30),
    ))
}

/// `len(list(MACROS_DIR.glob("*.json")))`: number of macro files on disk.
fn count_macro_files() -> usize {
    macro_names_in(&paths::macros_dir()).len()
}

fn macro_names_in(macros_dir: &Path) -> HashSet<String> {
    std::fs::read_dir(macros_dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            (path.is_file() && path.extension().and_then(|value| value.to_str()) == Some("json"))
                .then(|| path.file_stem()?.to_str().map(str::to_string))
                .flatten()
        })
        .collect()
}

fn run_history_in(stats: &PlayStats, macros_dir: &Path, limit: usize) -> Vec<HistoryEntry> {
    let existing = macro_names_in(macros_dir);
    stats
        .history(usize::MAX)
        .into_iter()
        .filter(|entry| existing.contains(&entry.name))
        .take(limit)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::stats::PlayStats;
    use crate::test_support::temp_dir;

    #[test]
    fn run_history_excludes_deleted_macro_files_before_applying_limit() {
        let macros = temp_dir("visible_run_history");
        let stats = PlayStats::new(macros.join("stats.json"));
        stats.record("kept");
        stats.record("deleted");
        std::fs::write(macros.join("kept.json"), "{}").expect("macro fixture");

        let history = run_history_in(&stats, &macros, 1);

        assert_eq!(history.len(), 1);
        assert_eq!(history[0].name, "kept");
    }
}
