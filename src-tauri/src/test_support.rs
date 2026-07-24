//! Test-only helpers. Compiled only under `cfg(test)`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// Create and return a fresh, unique temporary directory, the Rust analogue of
/// the Python tests' `tempfile.mkdtemp()`. Uniqueness comes from the process id
/// plus a monotonic counter, so parallel tests never collide.
pub fn temp_dir(label: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let dir = std::env::temp_dir().join(format!("clawmation_test_{label}_{pid}_{n}"));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}
