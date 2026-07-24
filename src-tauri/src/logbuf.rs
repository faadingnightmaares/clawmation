//! In-memory log ring buffer feeding the UI heartbeat.
//!
//! Mirrors `Api._emit`: entries are `{t: "HH:MM:SS", level, msg}`, the buffer
//! keeps the most recent 200, and `get_status` returns the last 60.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

/// Maximum entries retained in the buffer (matches `_emit`'s 200-cap).
pub const CAPACITY: usize = 200;
/// Number of entries surfaced in each `get_status` payload.
pub const TAIL: usize = 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub t: String,
    pub level: String,
    pub msg: String,
}

#[derive(Debug, Default)]
pub struct LogBuffer {
    entries: VecDeque<LogEntry>,
}

impl LogBuffer {
    pub fn push(&mut self, level: &str, msg: impl Into<String>) {
        let t = chrono::Local::now().format("%H:%M:%S").to_string();
        self.entries.push_back(LogEntry {
            t,
            level: level.to_string(),
            msg: msg.into(),
        });
        while self.entries.len() > CAPACITY {
            self.entries.pop_front();
        }
    }

    /// The most recent `n` entries, oldest-first (same order the UI renders).
    pub fn tail(&self, n: usize) -> Vec<LogEntry> {
        let start = self.entries.len().saturating_sub(n);
        self.entries.iter().skip(start).cloned().collect()
    }
}
