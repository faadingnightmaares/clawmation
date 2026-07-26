//! Macro listing, deletion, and template commands.
//!
//! Each mutating command is split into a thin `#[tauri::command]` wrapper (which
//! resolves the data directory and shared state, then emits the same log line as
//! the Python source) and a pure `*_in(macros_dir, …)` function that does the
//! file work. The pure functions are unit-tested against a temp directory, which
//! is how the Python `test_features.py` suite exercised the same surface.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::{json, Value};
use tauri::State;

use crate::engine::stats::PlayStats;
use crate::models::guard::GuardFile;
use crate::models::macro_def::Macro;
use crate::models::stats::StatsFile;
use crate::paths;
use crate::state::AppState;
use crate::util::round1;

#[derive(Debug, Serialize)]
pub struct MacroListItem {
    pub name: String,
    pub events: i64,
    pub duration: f64,
    /// `"WxH"` from the recording resolution.
    pub resolution: String,
    #[serde(rename = "loop")]
    pub loop_enabled: bool,
    pub loop_count: i64,
    pub category: String,
    pub notes: String,
    pub play_count: i64,
    pub last_played: f64,
    /// Cumulative seconds this macro has been played; the launcher's "time
    /// played" column. 0.0 until the first completed run banks a duration.
    pub played: f64,
}

/// One entry of `list_templates`, matching the Python dict shape.
#[derive(Debug, Serialize)]
pub struct TemplateItem {
    pub name: String,
    pub events: i64,
    pub duration: f64,
    pub category: String,
}

#[tauri::command(async)]
pub fn list_macros() -> Vec<MacroListItem> {
    let dir = paths::macros_dir();
    let stats = StatsFile::load(&paths::config_dir().join("stats.json"));

    // Top-level *.json files, newest-first by modified time.
    let mut files: Vec<(PathBuf, SystemTime)> = match std::fs::read_dir(&dir) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("json"))
            .map(|p| {
                let mtime = std::fs::metadata(&p)
                    .and_then(|m| m.modified())
                    .unwrap_or(UNIX_EPOCH);
                (p, mtime)
            })
            .collect(),
        Err(_) => Vec::new(),
    };
    files.sort_by(|a, b| b.1.cmp(&a.1));

    files
        .into_iter()
        .filter_map(|(path, _)| {
            // A file that fails to load (e.g. missing the required `name`/`type`
            // keys) is skipped, matching Python's `try/except: continue`.
            let macro_def = Macro::load(&path).ok()?;
            let (w, h) = macro_def.record_resolution;
            let stat = stats.stats.get(&macro_def.name);
            Some(MacroListItem {
                events: macro_def.events.len() as i64,
                duration: round1(macro_def.duration()),
                resolution: format!("{w}x{h}"),
                loop_enabled: macro_def.loop_enabled,
                loop_count: macro_def.loop_count,
                category: macro_def.category,
                notes: macro_def.notes,
                play_count: stat.map(|s| s.count).unwrap_or(0),
                last_played: stat.map(|s| s.last_played).unwrap_or(0.0),
                played: stat.map(|s| s.total_duration).unwrap_or(0.0),
                name: macro_def.name,
            })
        })
        .collect()
}

// ── Deletion ────────────────────────────────────────────────────────────────

#[tauri::command(async)]
pub fn delete_macro(state: State<AppState>, name: String) -> Value {
    let result = delete_macro_in(&paths::macros_dir(), &state.core.play_stats, &name);
    if result["ok"] == json!(true) {
        state.emit("warn", format!("Deleted {name}"));
    }
    result
}

#[tauri::command(async)]
pub fn bulk_delete(state: State<AppState>, names: Vec<String>) -> Value {
    let result = bulk_delete_in(&paths::macros_dir(), &state.core.play_stats, &names);
    if let Some(count) = result["deleted"]
        .as_array()
        .map(|a| a.len())
        .filter(|n| *n > 0)
    {
        state.emit("warn", format!("Bulk deleted {count} macro(s)"));
    }
    result
}

// ── Templates ───────────────────────────────────────────────────────────────

#[tauri::command(async)]
pub fn save_as_template(state: State<AppState>, name: String, template_name: String) -> Value {
    let result = save_as_template_in(&paths::macros_dir(), &name, &template_name);
    if result["ok"] == json!(true) {
        state.emit("ok", format!("Saved template '{}'", template_name.trim()));
    }
    result
}

#[tauri::command(async)]
pub fn list_templates() -> Vec<TemplateItem> {
    list_templates_in(&paths::macros_dir())
}

#[tauri::command(async)]
pub fn create_from_template(state: State<AppState>, template_name: String, new_name: String) -> Value {
    let result = create_from_template_in(&paths::macros_dir(), &template_name, &new_name);
    if result["ok"] == json!(true) {
        state.emit(
            "ok",
            format!("Created '{}' from template '{}'", new_name.trim(), template_name),
        );
    }
    result
}

#[tauri::command(async)]
pub fn delete_template(state: State<AppState>, template_name: String) -> Value {
    let result = delete_template_in(&paths::macros_dir(), &template_name);
    if result["ok"] == json!(true) {
        state.emit("warn", format!("Deleted template '{template_name}'"));
    }
    result
}

// ── Editing (rename, duplicate, repeat, category, notes) ─────────────────────

#[tauri::command(async)]
pub fn rename_macro(state: State<AppState>, old_name: String, new_name: String) -> Value {
    let result = rename_macro_in(&paths::macros_dir(), &paths::guards_dir(), &old_name, &new_name);
    if result["ok"] == json!(true) {
        let safe = result["name"].as_str().unwrap_or_default().to_string();
        let old = old_name.trim();
        // Keep the "last macro" pointer in sync: `Api._last_macro_name`.
        {
            let mut rt = state.core.runtime.lock().unwrap();
            if rt.last_macro == old {
                rt.last_macro = safe.clone();
            }
        }
        state.emit("ok", format!("Renamed {old} \u{2192} {safe}"));
    }
    result
}

#[tauri::command(async)]
pub fn duplicate_macro(state: State<AppState>, name: String) -> Value {
    let result = duplicate_macro_in(&paths::macros_dir(), &paths::guards_dir(), &name);
    if result["ok"] == json!(true) {
        let new_name = result["name"].as_str().unwrap_or_default();
        state.emit("ok", format!("Duplicated '{}' \u{2192} '{new_name}'", strip_json(&name)));
    }
    result
}

/// `set_repeat` is silent: the repeat dial persists on every drag, so a log line
/// per change would flood the activity feed (the source emits nothing here).
#[tauri::command(async)]
pub fn set_repeat(name: String, repeat: i64) -> Value {
    set_repeat_in(&paths::macros_dir(), &name, repeat)
}

#[tauri::command(async)]
pub fn set_category(state: State<AppState>, name: String, category: String) -> Value {
    let result = set_category_in(&paths::macros_dir(), &name, &category);
    if result["ok"] == json!(true) {
        let stem = strip_json(&name);
        let cat = result["category"].as_str().unwrap_or_default();
        if cat.is_empty() {
            state.emit("ok", format!("'{stem}' category cleared"));
        } else {
            state.emit("ok", format!("'{stem}' category set to '{cat}'"));
        }
    }
    result
}

#[tauri::command(async)]
pub fn set_notes(state: State<AppState>, name: String, notes: String) -> Value {
    let result = set_notes_in(&paths::macros_dir(), &name, &notes);
    if result["ok"] == json!(true) {
        let stem = strip_json(&name);
        let notes_val = result["notes"].as_str().unwrap_or_default();
        if notes_val.is_empty() {
            state.emit("ok", format!("'{stem}' notes cleared"));
        } else {
            state.emit("ok", format!("'{stem}' notes updated"));
        }
    }
    result
}

// ── Pure implementations (unit-tested against a temp dir) ────────────────────

/// Strip a trailing `.json`, matching Python's `name[:-5] if endswith(".json")`.
fn strip_json(name: &str) -> &str {
    name.strip_suffix(".json").unwrap_or(name)
}

/// `delete_macro`: the on-disk name is used verbatim (no `.json` strip), exactly
/// like the source's `MACROS_DIR / f"{name}.json"`.
fn delete_macro_in(macros_dir: &Path, stats: &PlayStats, name: &str) -> Value {
    let path = macros_dir.join(format!("{name}.json"));
    if !path.exists() {
        return json!({ "ok": false, "error": "Not found" });
    }
    match std::fs::remove_file(&path) {
        Ok(()) => {
            stats.remove(name);
            json!({ "ok": true })
        }
        Err(e) => json!({ "ok": false, "error": e.to_string() }),
    }
}

/// `bulk_delete`: also uses each name verbatim. A file that exists but cannot be
/// removed is reported in `failed` and the batch continues. See MIGRATION-NOTES.
fn bulk_delete_in(macros_dir: &Path, stats: &PlayStats, names: &[String]) -> Value {
    let mut deleted: Vec<String> = Vec::new();
    let mut failed: Vec<String> = Vec::new();
    for name in names {
        let path = macros_dir.join(format!("{name}.json"));
        if path.exists() && std::fs::remove_file(&path).is_ok() {
            deleted.push(name.clone());
            stats.remove(name);
        } else {
            failed.push(name.clone());
        }
    }
    json!({ "ok": true, "deleted": deleted, "failed": failed })
}

/// `save_as_template`: the source macro name is `.json`-stripped; the template
/// name is trimmed and used for both the stored `name` field and the filename.
fn save_as_template_in(macros_dir: &Path, name: &str, template_name: &str) -> Value {
    let src = macros_dir.join(format!("{}.json", strip_json(name)));
    if !src.exists() {
        return json!({ "ok": false, "error": "Not found" });
    }
    let tpl = template_name.trim();
    let mut macro_def = match Macro::load(&src) {
        Ok(m) => m,
        Err(e) => return json!({ "ok": false, "error": e.to_string() }),
    };
    macro_def.name = tpl.to_string();
    let templates_dir = macros_dir.join("templates");
    if let Err(e) = std::fs::create_dir_all(&templates_dir) {
        return json!({ "ok": false, "error": e.to_string() });
    }
    match macro_def.save_to(&templates_dir.join(format!("{tpl}.json"))) {
        Ok(()) => json!({ "ok": true, "name": tpl }),
        Err(e) => json!({ "ok": false, "error": e.to_string() }),
    }
}

fn list_templates_in(macros_dir: &Path) -> Vec<TemplateItem> {
    let dir = macros_dir.join("templates");
    // A missing directory lists as empty, matching `if not exists: return []`.
    let mut files: Vec<(PathBuf, SystemTime)> = match std::fs::read_dir(&dir) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("json"))
            .map(|p| {
                let mtime = std::fs::metadata(&p)
                    .and_then(|m| m.modified())
                    .unwrap_or(UNIX_EPOCH);
                (p, mtime)
            })
            .collect(),
        Err(_) => Vec::new(),
    };
    files.sort_by(|a, b| b.1.cmp(&a.1));

    files
        .into_iter()
        .filter_map(|(path, _)| {
            let m = Macro::load(&path).ok()?;
            Some(TemplateItem {
                events: m.events.len() as i64,
                duration: round1(m.duration()),
                name: m.name,
                category: m.category,
            })
        })
        .collect()
}

/// `create_from_template`: the template name is used verbatim (no strip); the new
/// macro is trimmed and saved to `MACROS_DIR/{new_name}.json` (Python's default
/// `macro.save()` path).
fn create_from_template_in(macros_dir: &Path, template_name: &str, new_name: &str) -> Value {
    let src = macros_dir
        .join("templates")
        .join(format!("{template_name}.json"));
    if !src.exists() {
        return json!({ "ok": false, "error": "Template not found" });
    }
    let new = new_name.trim();
    let mut macro_def = match Macro::load(&src) {
        Ok(m) => m,
        Err(e) => return json!({ "ok": false, "error": e.to_string() }),
    };
    macro_def.name = new.to_string();
    match macro_def.save_to(&macros_dir.join(format!("{new}.json"))) {
        Ok(()) => json!({ "ok": true, "name": new }),
        Err(e) => json!({ "ok": false, "error": e.to_string() }),
    }
}

/// `delete_template`: template name used verbatim (no strip).
fn delete_template_in(macros_dir: &Path, template_name: &str) -> Value {
    let path = macros_dir
        .join("templates")
        .join(format!("{template_name}.json"));
    if !path.exists() {
        return json!({ "ok": false, "error": "Not found" });
    }
    match std::fs::remove_file(&path) {
        Ok(()) => json!({ "ok": true }),
        Err(e) => json!({ "ok": false, "error": e.to_string() }),
    }
}

/// `set_category`: `.json`-stripped name; the value is trimmed and an empty value
/// clears it (`category.strip() if category else ""`).
fn set_category_in(macros_dir: &Path, name: &str, category: &str) -> Value {
    let stem = strip_json(name);
    let path = macros_dir.join(format!("{stem}.json"));
    if !path.exists() {
        return json!({ "ok": false, "error": format!("Not found: {stem}") });
    }
    let mut macro_def = match Macro::load(&path) {
        Ok(m) => m,
        Err(e) => return json!({ "ok": false, "error": e.to_string() }),
    };
    macro_def.category = category.trim().to_string();
    match macro_def.save_to(&path) {
        Ok(()) => json!({ "ok": true, "category": macro_def.category }),
        Err(e) => json!({ "ok": false, "error": e.to_string() }),
    }
}

/// `set_notes`: `.json`-stripped name; the value is trimmed and an empty value
/// clears it.
fn set_notes_in(macros_dir: &Path, name: &str, notes: &str) -> Value {
    let stem = strip_json(name);
    let path = macros_dir.join(format!("{stem}.json"));
    if !path.exists() {
        return json!({ "ok": false, "error": format!("Not found: {stem}") });
    }
    let mut macro_def = match Macro::load(&path) {
        Ok(m) => m,
        Err(e) => return json!({ "ok": false, "error": e.to_string() }),
    };
    macro_def.notes = notes.trim().to_string();
    match macro_def.save_to(&path) {
        Ok(()) => json!({ "ok": true, "notes": macro_def.notes }),
        Err(e) => json!({ "ok": false, "error": e.to_string() }),
    }
}

/// `set_repeat`: persist the loop setting (`0` infinite, `1` once, `N` N times),
/// the same mapping `play_macro` applies to a `repeat` argument.
fn set_repeat_in(macros_dir: &Path, name: &str, repeat: i64) -> Value {
    let stem = strip_json(name);
    let path = macros_dir.join(format!("{stem}.json"));
    if !path.exists() {
        return json!({ "ok": false, "error": format!("Not found: {stem}") });
    }
    let mut macro_def = match Macro::load(&path) {
        Ok(m) => m,
        Err(e) => return json!({ "ok": false, "error": e.to_string() }),
    };
    if repeat == 0 {
        macro_def.loop_enabled = true;
        macro_def.loop_count = 0;
    } else if repeat == 1 {
        macro_def.loop_enabled = false;
        macro_def.loop_count = 1;
    } else {
        macro_def.loop_enabled = true;
        macro_def.loop_count = repeat;
    }
    match macro_def.save_to(&path) {
        Ok(()) => {
            json!({ "ok": true, "loop": macro_def.loop_enabled, "loop_count": macro_def.loop_count })
        }
        Err(e) => json!({ "ok": false, "error": e.to_string() }),
    }
}

/// `duplicate_macro`: copy to `{stem}_copy`, `{stem}_copy2`, … (first free name)
/// and clone any guards so the duplicate is fully functional. `.json`-stripped
/// name; the "not found" error carries no stem, matching the source.
fn duplicate_macro_in(macros_dir: &Path, guards_dir: &Path, name: &str) -> Value {
    let stem = strip_json(name);
    let src = macros_dir.join(format!("{stem}.json"));
    if !src.exists() {
        return json!({ "ok": false, "error": "Not found" });
    }
    let mut macro_def = match Macro::load(&src) {
        Ok(m) => m,
        Err(e) => return json!({ "ok": false, "error": e.to_string() }),
    };
    let base = format!("{stem}_copy");
    let mut new_name = base.clone();
    let mut counter = 2;
    while macros_dir.join(format!("{new_name}.json")).exists() {
        new_name = format!("{base}{counter}");
        counter += 1;
    }
    macro_def.name = new_name.clone();
    if let Err(e) = macro_def.save_to(&macros_dir.join(format!("{new_name}.json"))) {
        return json!({ "ok": false, "error": e.to_string() });
    }
    // `if guards: save_guards(...)`. Only write a sidecar when the source has one.
    let guards = GuardFile::load(&guards_dir.join(format!("{stem}.json"))).guards;
    if !guards.is_empty() {
        let _ = GuardFile { guards }.save_to(&guards_dir.join(format!("{new_name}.json")));
    }
    json!({ "ok": true, "name": new_name })
}

/// `rename_macro`: whitespace-trim both names (no `.json` strip), sanitize the new
/// name to alphanumerics plus `_`/`-`/space, then move the macro file and its
/// guard sidecar. A case-only change is treated as in-place; Windows paths are
/// case-insensitive, matching the source's `pathlib` comparison.
fn rename_macro_in(macros_dir: &Path, guards_dir: &Path, old_name: &str, new_name: &str) -> Value {
    let old_name = old_name.trim();
    let new_name = new_name.trim();
    if old_name.is_empty() || new_name.is_empty() {
        return json!({ "ok": false, "error": "Empty name" });
    }
    let safe: String = new_name
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | ' '))
        .collect::<String>()
        .trim()
        .to_string();
    if safe.is_empty() {
        return json!({ "ok": false, "error": "Invalid name" });
    }
    let src = macros_dir.join(format!("{old_name}.json"));
    let dst = macros_dir.join(format!("{safe}.json"));
    if !src.exists() {
        return json!({ "ok": false, "error": "Not found" });
    }
    let same_file = safe.eq_ignore_ascii_case(old_name);
    if dst.exists() && !same_file {
        return json!({ "ok": false, "error": "Name already exists" });
    }
    let mut m = match Macro::load(&src) {
        Ok(m) => m,
        Err(e) => return json!({ "ok": false, "error": e.to_string() }),
    };
    m.name = safe.clone();
    if !same_file {
        if let Err(e) = std::fs::remove_file(&src) {
            return json!({ "ok": false, "error": e.to_string() });
        }
    }
    if let Err(e) = m.save_to(&dst) {
        return json!({ "ok": false, "error": e.to_string() });
    }
    // Move the guard sidecar file along with the macro (best-effort).
    let gsrc = guards_dir.join(format!("{old_name}.json"));
    if gsrc.exists() {
        let _ = std::fs::rename(&gsrc, guards_dir.join(format!("{safe}.json")));
    }
    json!({ "ok": true, "name": safe })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::guard::Guard;
    use crate::models::macro_def::{InputEventType, MacroEvent};
    use crate::test_support::temp_dir;

    /// Write a macro with `n_events` mouse-move events, mirroring the Python
    /// tests' `make_macro` helper.
    fn make_macro(dir: &Path, name: &str, n_events: usize) -> PathBuf {
        let events = (0..n_events)
            .map(|i| MacroEvent {
                event_type: InputEventType::MouseMove,
                timestamp: i as f64 * 0.1,
                x: 10 * i as i64,
                y: 10 * i as i64,
                button: "left".to_string(),
                key: String::new(),
                delta: 0,
                duration: 0.0,
                checkpoint: None,
            })
            .collect();
        let m = Macro {
            name: name.to_string(),
            events,
            ..Default::default()
        };
        let path = dir.join(format!("{name}.json"));
        m.save_to(&path).unwrap();
        path
    }

    #[test]
    fn templates_save_list_create_delete() {
        let macros = temp_dir("tpl_macros");
        let src = "__test___tpl_src";
        make_macro(&macros, src, 4);
        let tpl = "__test___tpl";

        let r = save_as_template_in(&macros, src, tpl);
        assert_eq!(r["ok"], json!(true), "save_as_template ok: {r}");
        let tpl_path = macros.join("templates").join(format!("{tpl}.json"));
        assert!(tpl_path.exists(), "template file written");

        let listed = list_templates_in(&macros);
        let item = listed
            .iter()
            .find(|t| t.name == tpl)
            .expect("template appears in list");
        assert_eq!(item.events, 4, "template reports 4 events");

        let new_name = "__test___from_tpl";
        let r = create_from_template_in(&macros, tpl, new_name);
        assert_eq!(r["ok"], json!(true), "create_from_template ok: {r}");
        let created =
            Macro::load(&macros.join(format!("{new_name}.json"))).expect("created macro loads");
        assert_eq!(created.events.len(), 4, "created macro keeps the 4 source events");
        assert_eq!(created.name, new_name, "created macro is renamed");

        let r = delete_template_in(&macros, tpl);
        assert_eq!(r["ok"], json!(true), "delete_template ok");
        assert!(!tpl_path.exists(), "template file removed");

        // Deleting a missing template reports Not found.
        let r = delete_template_in(&macros, "__test___missing_tpl");
        assert_eq!(r["ok"], json!(false));
        assert_eq!(r["error"], json!("Not found"));
    }

    #[test]
    fn save_as_template_missing_source_is_not_found() {
        let macros = temp_dir("tpl_missing");
        let r = save_as_template_in(&macros, "__test___nope", "whatever");
        assert_eq!(r["ok"], json!(false));
        assert_eq!(r["error"], json!("Not found"));
    }

    #[test]
    fn bulk_delete_removes_files_and_stats() {
        let macros = temp_dir("bulk_macros");
        let stats = PlayStats::new(macros.join("stats.json"));
        let names: Vec<String> = (0..3).map(|i| format!("__test___bulk_{i}")).collect();
        for n in &names {
            make_macro(&macros, n, 3);
        }
        stats.record(&names[0]);
        assert!(stats.get(&names[0]).is_some(), "stat present before delete");

        let mut targets = names.clone();
        targets.push("__test___bulk_missing".to_string());
        let r = bulk_delete_in(&macros, &stats, &targets);

        assert_eq!(r["ok"], json!(true));
        assert_eq!(r["deleted"].as_array().unwrap().len(), 3, "3 deleted");
        assert_eq!(r["failed"].as_array().unwrap().len(), 1, "1 failed (missing)");
        assert_eq!(r["failed"][0], json!("__test___bulk_missing"));
        for n in &names {
            assert!(!macros.join(format!("{n}.json")).exists(), "{n} removed");
        }
        assert!(stats.get(&names[0]).is_none(), "stat removed for deleted macro");
    }

    #[test]
    fn delete_macro_removes_file_and_stat() {
        let macros = temp_dir("del_macro");
        let stats = PlayStats::new(macros.join("stats.json"));
        let name = "__test___del";
        make_macro(&macros, name, 2);
        stats.record(name);

        let r = delete_macro_in(&macros, &stats, name);
        assert_eq!(r["ok"], json!(true));
        assert!(!macros.join(format!("{name}.json")).exists(), "file removed");
        assert!(stats.get(name).is_none(), "stat removed");

        let r = delete_macro_in(&macros, &stats, "__test___nope");
        assert_eq!(r["ok"], json!(false));
        assert_eq!(r["error"], json!("Not found"));
    }

    #[test]
    fn set_repeat_persists_loop_mapping() {
        let macros = temp_dir("set_repeat");
        let name = "__test___rep";
        make_macro(&macros, name, 2);
        let load = || Macro::load(&macros.join(format!("{name}.json"))).unwrap();

        // 0 → infinite loop.
        let r = set_repeat_in(&macros, name, 0);
        assert_eq!(r["ok"], json!(true));
        assert_eq!((r["loop"].clone(), r["loop_count"].clone()), (json!(true), json!(0)));
        let m = load();
        assert!(m.loop_enabled && m.loop_count == 0, "infinite persisted");

        // 1 → play once (loop off).
        set_repeat_in(&macros, name, 1);
        let m = load();
        assert!(!m.loop_enabled && m.loop_count == 1, "once persisted");

        // N → fixed count.
        let r = set_repeat_in(&macros, name, 5);
        assert_eq!(r["loop_count"], json!(5));
        let m = load();
        assert!(m.loop_enabled && m.loop_count == 5, "fixed count persisted");

        let r = set_repeat_in(&macros, "__test___missing", 3);
        assert_eq!(r["ok"], json!(false));
        assert_eq!(r["error"], json!("Not found: __test___missing"));
    }

    #[test]
    fn set_category_and_notes_trim_set_and_clear() {
        let macros = temp_dir("set_cat_notes");
        let name = "__test___cn";
        make_macro(&macros, name, 1);
        let load = || Macro::load(&macros.join(format!("{name}.json"))).unwrap();

        let r = set_category_in(&macros, name, "  Farming  ");
        assert_eq!(r["ok"], json!(true));
        assert_eq!(r["category"], json!("Farming"), "value is trimmed");
        assert_eq!(load().category, "Farming");

        let r = set_category_in(&macros, name, "");
        assert_eq!(r["category"], json!(""), "empty clears");
        assert_eq!(load().category, "");

        let r = set_notes_in(&macros, name, "  does the thing  ");
        assert_eq!(r["ok"], json!(true));
        assert_eq!(r["notes"], json!("does the thing"));
        assert_eq!(load().notes, "does the thing");

        let r = set_notes_in(&macros, "__test___nope", "x");
        assert_eq!(r["ok"], json!(false));
        assert_eq!(r["error"], json!("Not found: __test___nope"));
    }

    #[test]
    fn duplicate_macro_finds_unique_name_and_clones_guards() {
        let macros = temp_dir("dup_macros");
        let guards = temp_dir("dup_guards");
        let name = "__test___dup";
        make_macro(&macros, name, 3);
        GuardFile {
            guards: vec![Guard { id: "g1".into(), name: "Recon".into(), ..Default::default() }],
        }
        .save_to(&guards.join(format!("{name}.json")))
        .unwrap();

        let r = duplicate_macro_in(&macros, &guards, name);
        assert_eq!(r["ok"], json!(true));
        assert_eq!(r["name"], json!("__test___dup_copy"));
        let copy = Macro::load(&macros.join("__test___dup_copy.json")).unwrap();
        assert_eq!(copy.name, "__test___dup_copy");
        assert_eq!(copy.events.len(), 3, "events copied");
        let copied = GuardFile::load(&guards.join("__test___dup_copy.json")).guards;
        assert_eq!(copied.len(), 1, "guard cloned");
        assert_eq!(copied[0].name, "Recon");

        // A second duplicate bumps the counter to `_copy2`.
        let r2 = duplicate_macro_in(&macros, &guards, name);
        assert_eq!(r2["name"], json!("__test___dup_copy2"));

        let r3 = duplicate_macro_in(&macros, &guards, "__test___missing");
        assert_eq!(r3["ok"], json!(false));
        assert_eq!(r3["error"], json!("Not found"));
    }

    #[test]
    fn rename_macro_moves_file_and_guard_sidecar() {
        let macros = temp_dir("rename_macros");
        let guards = temp_dir("rename_guards");
        let old = "__test___old";
        make_macro(&macros, old, 2);
        GuardFile { guards: vec![Guard { id: "g1".into(), ..Default::default() }] }
            .save_to(&guards.join(format!("{old}.json")))
            .unwrap();

        // "!!" is stripped by the sanitizer, surrounding whitespace trimmed.
        let r = rename_macro_in(&macros, &guards, old, "  new name!!  ");
        assert_eq!(r["ok"], json!(true));
        assert_eq!(r["name"], json!("new name"));
        assert!(!macros.join(format!("{old}.json")).exists(), "old macro removed");
        let moved = Macro::load(&macros.join("new name.json")).unwrap();
        assert_eq!(moved.name, "new name", "stored name updated");
        assert!(!guards.join(format!("{old}.json")).exists(), "old guard removed");
        assert!(guards.join("new name.json").exists(), "guard moved alongside");
    }

    #[test]
    fn rename_macro_rejects_empty_invalid_missing_and_collision() {
        let macros = temp_dir("rename_reject");
        let guards = temp_dir("rename_reject_g");
        make_macro(&macros, "__test___a", 1);
        make_macro(&macros, "__test___b", 1);

        assert_eq!(rename_macro_in(&macros, &guards, "  ", "x")["error"], json!("Empty name"));
        assert_eq!(
            rename_macro_in(&macros, &guards, "__test___a", "   ")["error"],
            json!("Empty name")
        );
        // Only punctuation → sanitizes to empty → Invalid name.
        assert_eq!(
            rename_macro_in(&macros, &guards, "__test___a", "!!!")["error"],
            json!("Invalid name")
        );
        assert_eq!(
            rename_macro_in(&macros, &guards, "__test___missing", "y")["error"],
            json!("Not found")
        );
        // Collision with an existing different macro.
        assert_eq!(
            rename_macro_in(&macros, &guards, "__test___a", "__test___b")["error"],
            json!("Name already exists")
        );
    }
}
