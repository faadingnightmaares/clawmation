//! Data directory layout, mirroring `anime_macro/config.py`.
//!
//! Release builds keep data next to the executable (portable app, exactly like
//! the PyInstaller build). Debug builds resolve to the project root so
//! `tauri dev` finds the working tree's `config/`, `macros/`, etc.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

fn project_root() -> &'static PathBuf {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        if cfg!(debug_assertions) {
            // <root>/clawmation/src-tauri  ->  <root>/clawmation
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from("."))
        } else {
            std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(Path::to_path_buf))
                .unwrap_or_else(|| PathBuf::from("."))
        }
    })
}

pub fn root() -> PathBuf {
    project_root().clone()
}

pub fn config_dir() -> PathBuf {
    root().join("config")
}

pub fn macros_dir() -> PathBuf {
    root().join("macros")
}

pub fn templates_dir() -> PathBuf {
    root().join("templates")
}

pub fn snapshots_dir() -> PathBuf {
    root().join("snapshots")
}

pub fn guards_dir() -> PathBuf {
    macros_dir().join("guards")
}

pub fn ai_dir() -> PathBuf {
    macros_dir().join("ai")
}

/// Create every data directory if missing. Mirrors config.py's import-time mkdir.
pub fn ensure_dirs() {
    for dir in [
        config_dir(),
        macros_dir(),
        templates_dir(),
        snapshots_dir(),
        guards_dir(),
        ai_dir(),
    ] {
        let _ = std::fs::create_dir_all(dir);
    }
}

/// Count non-recursive top-level files with the given extension (no leading dot).
pub fn count_ext(dir: &Path, ext: &str) -> i64 {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some(ext))
                .count() as i64
        })
        .unwrap_or(0)
}
