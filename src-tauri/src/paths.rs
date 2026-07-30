//! Data directory layout, mirroring `anime_macro/config.py`.
//!
//! Release builds keep data in the per-user Roaming AppData folder
//! (`%APPDATA%\clawmation`), NOT beside the executable. Storing next to the exe
//! made an update able to orphan or destroy everything: a renamed bundle
//! identifier makes NSIS treat the build as a new product and install
//! side-by-side into a fresh folder, an uninstall wipes the folder, and a manual
//! install lands wherever the user points it — each leaves the new app staring at
//! empty `macros/`/`config/`. AppData is keyed to the Windows user, not the
//! install, so updates and reinstalls leave it untouched. On the first run at
//! that location we recover any data left beside a previous install, so existing
//! users lose nothing to the move. Debug builds still resolve to the project
//! root so `tauri dev` finds the working tree's `config/`, `macros/`, etc.

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
            release_root()
        }
    })
}

/// Release data root: AppData, with one-time recovery of beside-exe data. Runs
/// inside the `ROOT` initializer, so it happens on the very first `root()` access
/// — before `ensure_dirs` creates the empty directory skeleton — which is what
/// lets `has_data` tell a fresh install from a populated one.
fn release_root() -> PathBuf {
    let root = appdata_root().unwrap_or_else(beside_exe_root);
    recover_beside_exe_data(&root);
    root
}

fn appdata_root() -> Option<PathBuf> {
    if let Ok(appdata) = std::env::var("APPDATA") {
        return Some(PathBuf::from(appdata).join("clawmation"));
    }
    std::env::var("USERPROFILE").ok().map(|p| {
        PathBuf::from(p)
            .join("AppData")
            .join("Roaming")
            .join("clawmation")
    })
}

fn beside_exe_root() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Top-level data sub-trees. `macros` is copied recursively, which carries its
/// `guards/`, `ai/` and `templates/` children; the top-level `templates` and
/// `snapshots` are separate roots and are copied on their own.
const DATA_SUBDIRS: [&str; 4] = ["config", "macros", "templates", "snapshots"];

/// Copy any user data left beside a previous install into the new AppData root.
/// Runs once, only into an EMPTY root (never clobbers a live install), from the
/// first candidate location that actually has data. Best-effort: a failure to
/// copy one tree is logged and the rest still copy. If a user uninstalled an old
/// build whose uninstaller wiped its folder, there is nothing left to recover —
/// this rescues the orphaned-by-side-by-side case, not the wiped case.
fn recover_beside_exe_data(new_root: &Path) {
    if has_data(new_root) {
        return;
    }
    let Some(source) = candidate_legacy_roots()
        .into_iter()
        .find(|p| p != new_root && has_data(p))
    else {
        return;
    };
    for sub in DATA_SUBDIRS {
        let src = source.join(sub);
        if src.is_dir() {
            if let Err(e) = copy_dir_recursive(&src, &new_root.join(sub)) {
                eprintln!(
                    "Clawmation: recovering '{sub}' from {} failed: {e}",
                    source.display()
                );
            }
        }
    }
    eprintln!(
        "Clawmation: recovered user data from {} into {}",
        source.display(),
        new_root.display()
    );
}

/// Where a previous install may have left beside-exe data, most-likely first: the
/// current exe's own folder (an in-place update keeps the same path, the common
/// case), then the standard Tauri NSIS install dirs (a side-by-side install moves
/// the exe but leaves the old folder behind). A small fixed probe set recovers
/// the real cases without scanning the disk.
fn candidate_legacy_roots() -> Vec<PathBuf> {
    let mut out = vec![beside_exe_root()];
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        out.push(PathBuf::from(local).join("Programs").join("clawmation"));
    }
    for var in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Ok(pf) = std::env::var(var) {
            out.push(PathBuf::from(pf).join("clawmation"));
        }
    }
    out
}

/// A root counts as having data once its macros dir holds any entry, or its
/// config file exists. This is the "already claimed" guard, not a measure of
/// real content: recovery runs before `ensure_dirs` builds the skeleton, so on
/// the first launch the macros dir is absent and recovery proceeds, while on
/// every later launch the dir exists and recovery is skipped. Treating the
/// skeleton as claimed is what keeps recovery a one-time, never-clobber
/// operation instead of re-probing (and potentially overwriting) each start.
fn has_data(root: &Path) -> bool {
    let macros_non_empty = std::fs::read_dir(root.join("macros"))
        .map(|mut entries| entries.next().is_some())
        .unwrap_or(false);
    macros_non_empty || root.join("config").join("config.json").exists()
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_dir;

    #[test]
    fn copy_dir_recursive_copies_nested_files() {
        let src = temp_dir("paths_copy_src");
        let dst = temp_dir("paths_copy_dst");
        std::fs::create_dir_all(src.join("macros/guards")).unwrap();
        std::fs::write(src.join("macros/a.json"), b"a").unwrap();
        std::fs::write(src.join("macros/guards/g.json"), b"g").unwrap();
        std::fs::write(src.join("config.json"), b"c").unwrap();

        copy_dir_recursive(&src, &dst).unwrap();

        assert_eq!(std::fs::read(dst.join("macros/a.json")).unwrap(), b"a");
        assert_eq!(
            std::fs::read(dst.join("macros/guards/g.json")).unwrap(),
            b"g"
        );
        assert_eq!(std::fs::read(dst.join("config.json")).unwrap(), b"c");
    }

    #[test]
    fn has_data_detects_macros_or_config() {
        let empty = temp_dir("paths_hasdata_empty");
        assert!(!has_data(&empty), "an empty root has no data");

        // Any entry in macros/ claims the root — even the empty `guards` dir the
        // ensure_dirs skeleton creates. Recovery is a one-shot ordered before that
        // skeleton exists, so a root that already has one must never be recovered
        // into again (that is what makes the operation idempotent and clobber-free).
        std::fs::create_dir_all(empty.join("macros/guards")).unwrap();
        assert!(has_data(&empty), "a non-empty macros dir claims the root");

        let with_config = temp_dir("paths_hasdata_config");
        std::fs::create_dir_all(with_config.join("config")).unwrap();
        std::fs::write(with_config.join("config/config.json"), b"{}").unwrap();
        assert!(has_data(&with_config), "config.json counts as data");

        let with_macro = temp_dir("paths_hasdata_macro");
        std::fs::create_dir_all(with_macro.join("macros")).unwrap();
        std::fs::write(with_macro.join("macros/x.json"), b"x").unwrap();
        assert!(has_data(&with_macro), "a macro file counts as data");
    }
}
