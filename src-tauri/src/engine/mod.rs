//! Stateful engines: the app's runtime logic, hardware-free.
//!
//! Each engine mirrors a Python manager class (`chains.py`, `scheduler.py`,
//! `stats.py`, `guards.py`), owning its own state behind a lock and taking
//! injected callbacks (`play_macro`, `is_idle`, `fire`, `detect`, `actuate`, …)
//! so it can run and be tested without any real capture, input, or playback.
//! The Tauri layer wires these to the real player, input controller, and
//! detector in `state.rs`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub mod ai;
pub mod anti_afk;
pub mod chains;
pub mod guards;
pub mod scheduler;
pub mod stats;
pub mod vision_agent;

/// Sleep up to `total`, waking early the moment `running` is cleared.
///
/// Rust's std threads can't be force-joined with a timeout, so any engine whose
/// work sleeps longer than its poll interval (a guard's `resume_delay`, a drag
/// sweep) must poll the flag while it waits, or `stop()` would block for the
/// full sleep. Chunked at 20ms: negligible overhead, snappy shutdown.
pub(crate) fn sleep_interruptible(running: &AtomicBool, total: Duration) {
    const CHUNK: Duration = Duration::from_millis(20);
    let start = Instant::now();
    while running.load(Ordering::SeqCst) {
        let elapsed = start.elapsed();
        if elapsed >= total {
            break;
        }
        std::thread::sleep((total - elapsed).min(CHUNK));
    }
}
