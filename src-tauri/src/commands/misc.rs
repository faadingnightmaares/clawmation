//! Version and update commands.
//!
//! `VERSION` is the single source of truth for what the app calls itself, and it
//! is the crate version so the string the About card shows is the same one the
//! updater compares against the release manifest.
//!
//! The whole update flow is driven from Rust: [`check_update`] asks the endpoint
//! in `tauri.conf.json` whether a newer signed build exists, and
//! [`install_update`] downloads it, hands it to the Windows installer, and
//! restarts. The frontend therefore keeps invoking plain commands and needs
//! neither the plugin's JS bindings nor its capability permissions. Publishing a
//! release that these commands can see is documented in `docs/RELEASING.md`.

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::UpdaterExt;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Event carrying download progress to the About card's progress bar. Fired only
/// while [`install_update`] runs.
const PROGRESS_EVENT: &str = "update-progress";

/// Event announcing the result of the startup check, so a running window can
/// mention the update without having asked for it.
pub const AVAILABLE_EVENT: &str = "update-available";

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
pub struct DpiReport {
    pub ok: bool,
    pub report: String,
}

/// Live probe of the DPI pipeline on a fresh worker thread: the same check the
/// app logs at startup, on demand, so a scaling regression can be confirmed on
/// the machine it breaks on rather than inferred from missed clicks.
#[tauri::command]
pub fn dpi_report() -> DpiReport {
    let (ok, report) = crate::hardware::dpi::report();
    DpiReport { ok, report }
}

#[derive(Clone, Debug, Serialize)]
pub struct UpdateInfo {
    pub update_available: bool,
    pub current: String,
    pub latest: String,
    /// The release notes from the manifest, when it carries any.
    pub notes: Option<String>,
}

impl UpdateInfo {
    fn up_to_date() -> Self {
        Self {
            update_available: false,
            current: VERSION.to_string(),
            latest: VERSION.to_string(),
            notes: None,
        }
    }
}

/// Ask the release endpoint what the newest signed build is.
///
/// An unreachable endpoint, an unparseable manifest, or a bad signature all come
/// back as `Err` rather than "up to date", so the UI can say it couldn't check
/// instead of claiming the user is current when nothing was actually compared.
#[tauri::command]
pub async fn check_update(app: AppHandle) -> Result<UpdateInfo, String> {
    look_for_update(&app).await.map_err(|e| e.to_string())
}

/// The shared body of [`check_update`] and the startup check.
async fn look_for_update(app: &AppHandle) -> tauri_plugin_updater::Result<UpdateInfo> {
    Ok(match app.updater()?.check().await? {
        Some(update) => UpdateInfo {
            update_available: true,
            current: update.current_version.clone(),
            latest: update.version.clone(),
            notes: update.body.clone(),
        },
        None => UpdateInfo::up_to_date(),
    })
}

/// Download the newest build, run the installer, and restart into it.
///
/// The check is repeated here rather than caching the `Update` handle from
/// [`check_update`]: it costs one request, and it means an install can never act
/// on a manifest that was fetched before the app sat idle for an hour.
///
/// On success this never returns (`restart()` exits the process), so the
/// frontend's await simply stops. Everything before that point is recoverable
/// and reports as `Err`.
#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    let Some(update) = app
        .updater()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| e.to_string())?
    else {
        return Err("No update available".to_string());
    };

    let progress_app = app.clone();
    let mut downloaded = 0usize;
    update
        .download_and_install(
            move |chunk, total| {
                downloaded += chunk;
                let _ = progress_app.emit(PROGRESS_EVENT, (downloaded as u64, total));
            },
            || {},
        )
        .await
        .map_err(|e| e.to_string())?;

    app.restart();
}

/// Check once, in the background, shortly after launch: the "auto" half of auto
/// update. A failure is silent: the app is fully usable offline, and the About
/// card's button is there for anyone who wants a verdict.
pub fn check_in_background(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Ok(info) = look_for_update(&app).await {
            if info.update_available {
                let _ = app.emit(AVAILABLE_EVENT, info);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn windows_installer_identity_cannot_drift_again() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tauri.conf.json");
        let config: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();

        assert_eq!(config["identifier"], "com.a7mda.clawmation");
        assert_eq!(
            config["bundle"]["windows"]["nsis"]["installerHooks"],
            "windows/installer-hooks.nsh"
        );
    }

    #[test]
    fn release_versions_stay_in_sync() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let tauri: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(root.join("tauri.conf.json")).unwrap(),
        )
        .unwrap();
        let package: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(root.parent().unwrap().join("package.json")).unwrap(),
        )
        .unwrap();

        assert_eq!(tauri["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(package["version"], env!("CARGO_PKG_VERSION"));
    }
}
