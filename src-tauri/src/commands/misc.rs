//! Version and update commands.
//!
//! The Python app reports a static version and never finds an update; reproduced
//! as-is. `VERSION` is the single source of truth.

use serde::Serialize;

pub const VERSION: &str = "1.0.0";

#[derive(Debug, Serialize)]
pub struct VersionInfo {
    pub version: String,
}

#[tauri::command(async)]
pub fn get_version() -> VersionInfo {
    VersionInfo {
        version: VERSION.to_string(),
    }
}

#[derive(Debug, Serialize)]
pub struct UpdateInfo {
    pub update_available: bool,
    pub current: String,
    pub latest: String,
}

#[tauri::command(async)]
pub fn check_update() -> UpdateInfo {
    UpdateInfo {
        update_available: false,
        current: VERSION.to_string(),
        latest: VERSION.to_string(),
    }
}
