//! `get_status` heartbeat payload.
//!
//! Response-only DTO reproducing the exact shape `Api.get_status` returns, which
//! the frontend polls on an interval. Not persisted.

use serde::Serialize;

use crate::logbuf::LogEntry;

#[derive(Debug, Serialize)]
pub struct WindowStatus {
    pub found: bool,
    pub title: String,
    /// `"WxH"` when found, `"—"` otherwise.
    pub size: String,
}

#[derive(Debug, Serialize)]
pub struct CaptureStatus {
    pub backend: String,
    pub fps: f64,
}

#[derive(Debug, Serialize)]
pub struct StatusConfig {
    pub resolution: [u32; 2],
    pub capture_backend: String,
    pub hotkey_record: String,
    pub hotkey_play: String,
    pub hotkey_stop: String,
    pub indicator_on_top: bool,
}

#[derive(Debug, Serialize)]
pub struct Status {
    /// `"idle"`, `"recording"`, `"playing"`, or `"paused"`.
    pub mode: String,
    pub elapsed: f64,
    pub record_paused: bool,
    pub play_iteration: i64,
    pub play_total_reps: i64,
    pub window: WindowStatus,
    pub capture: CaptureStatus,
    pub recorded_count: i64,
    pub last_macro: String,
    pub macro_count: i64,
    pub indicator_alive: bool,
    pub config: StatusConfig,
    pub log: Vec<LogEntry>,
}
