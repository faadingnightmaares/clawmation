//! Play statistics engine — a thread-safe wrapper over `config/stats.json`.
//!
//! Mirrors `anime_macro/stats.py::PlayStats`. Tracks per-macro play counts and a
//! newest-first execution history (capped at 100). All mutations persist
//! immediately; a persistence failure is swallowed so it can never break
//! playback, exactly like the source.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::models::stats::{HistoryEntry, MacroStat, StatsFile, MAX_HISTORY};
use crate::util::{now_epoch, round1};

pub struct PlayStats {
    store_path: PathBuf,
    data: Mutex<StatsFile>,
}

impl PlayStats {
    pub fn new(store_path: impl Into<PathBuf>) -> Self {
        let store_path = store_path.into();
        let data = StatsFile::load(&store_path);
        Self {
            store_path,
            data: Mutex::new(data),
        }
    }

    /// Record one play of a macro, updating counts and prepending a history entry.
    pub fn record_play(&self, name: &str, duration: f64, status: &str) {
        let now = now_epoch();
        let mut data = self.data.lock().unwrap();
        match data.stats.get_mut(name) {
            Some(entry) => {
                entry.count += 1;
                entry.last_played = now;
            }
            None => {
                data.stats.insert(
                    name.to_string(),
                    MacroStat {
                        count: 1,
                        first_played: now,
                        last_played: now,
                    },
                );
            }
        }
        data.history.insert(
            0,
            HistoryEntry {
                name: name.to_string(),
                timestamp: now,
                duration,
                status: status.to_string(),
            },
        );
        if data.history.len() > MAX_HISTORY {
            data.history.truncate(MAX_HISTORY);
        }
        self.persist(&data);
    }

    /// Stats for a single macro, or `None` if it was never played.
    pub fn get(&self, name: &str) -> Option<MacroStat> {
        self.data.lock().unwrap().stats.get(name).cloned()
    }

    /// Total plays across every macro.
    pub fn total_plays(&self) -> i64 {
        self.data.lock().unwrap().stats.values().map(|s| s.count).sum()
    }

    /// Every macro's stats keyed by name — the analogue of Python's
    /// `PlayStats.all()`, consumed by `get_stats_summary`.
    pub fn all(&self) -> BTreeMap<String, MacroStat> {
        self.data.lock().unwrap().stats.clone()
    }

    /// Drop stats for a macro (e.g. when it is deleted).
    pub fn remove(&self, name: &str) {
        let mut data = self.data.lock().unwrap();
        if data.stats.remove(name).is_some() {
            self.persist(&data);
        }
    }

    /// The most recent execution history entries (newest first).
    pub fn history(&self, limit: usize) -> Vec<HistoryEntry> {
        self.data
            .lock()
            .unwrap()
            .history
            .iter()
            .take(limit)
            .cloned()
            .collect()
    }

    /// Fill duration/status onto the newest `running` history entry for `name`.
    /// No-op if there is no matching entry, matching the best-effort source.
    pub fn update_last_run(&self, name: &str, duration: f64, status: &str) {
        let mut data = self.data.lock().unwrap();
        if let Some(entry) = data
            .history
            .iter_mut()
            .find(|e| e.name == name && e.status == "running")
        {
            entry.duration = round1(duration);
            entry.status = status.to_string();
            self.persist(&data);
        }
    }

    fn persist(&self, data: &StatsFile) {
        let _ = data.save_to(&self.store_path);
    }
}

/// Convenience for the common "one completed play" case (`record_play(name)` in
/// Python, whose `duration`/`status` default to `0.0`/`"completed"`).
impl PlayStats {
    pub fn record(&self, name: &str) {
        self.record_play(name, 0.0, "completed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_dir;

    #[test]
    fn play_stats_record_get_remove() {
        let dir = temp_dir("stats");
        let store = dir.join("stats.json");
        let ps = PlayStats::new(&store);

        ps.record("__test___stat_macro");
        ps.record("__test___stat_macro");
        ps.record("__test___stat_macro");

        let stats = ps.get("__test___stat_macro").expect("stat present");
        assert_eq!(stats.count, 3, "count is 3");
        assert!(stats.first_played > 0.0, "first_played set");
        assert!(stats.last_played > 0.0, "last_played set");
        assert!(stats.last_played >= stats.first_played, "last >= first");
        assert_eq!(ps.total_plays(), 3, "total_plays is 3");

        // Persistence across restart.
        let ps2 = PlayStats::new(&store);
        assert_eq!(
            ps2.get("__test___stat_macro").expect("persisted").count,
            3,
            "persisted count is 3"
        );

        ps2.remove("__test___stat_macro");
        assert!(ps2.get("__test___stat_macro").is_none(), "removed");
        assert_eq!(ps2.total_plays(), 0, "total_plays is 0 after remove");
    }
}
