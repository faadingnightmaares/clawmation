//! Guard commands — list, count, and save the per-macro guard sets.
//!
//! `guard_list` returns the stored guard dicts verbatim (no re-serialization, so
//! the UI sees exactly what is on disk); `guard_save` parses each dict through
//! the [`Guard`] model — the analogue of `Guard.from_dict`, which keeps known
//! keys, drops unknown ones, and defaults the missing. Ports of `Api.guard_list`
//! / `get_all_guard_counts` / `guard_save`.

use std::path::Path;

use serde_json::{json, Value};
use tauri::State;

use crate::models::guard::{Guard, GuardFile};
use crate::paths;
use crate::state::AppState;

#[tauri::command]
pub fn guard_list(macro_name: String) -> Value {
    let stem = macro_name.strip_suffix(".json").unwrap_or(&macro_name);
    let path = paths::guards_dir().join(format!("{stem}.json"));
    if !path.exists() {
        return json!({ "ok": true, "guards": [] });
    }
    match load_json(&path) {
        Ok(data) => {
            let guards = data.get("guards").cloned().unwrap_or_else(|| json!([]));
            json!({ "ok": true, "guards": guards })
        }
        Err(e) => json!({ "ok": false, "error": e }),
    }
}

#[tauri::command]
pub fn get_all_guard_counts() -> Value {
    let mut counts = serde_json::Map::new();
    // A missing guards dir lists as empty, matching `if not GUARDS_DIR.exists()`.
    if let Ok(entries) = std::fs::read_dir(paths::guards_dir()) {
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            // Per-file failures are swallowed, as in the source's `except: pass`.
            let Ok(data) = load_json(&path) else { continue };
            let n = data.get("guards").and_then(Value::as_array).map_or(0, Vec::len);
            if n > 0 {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    counts.insert(stem.to_string(), json!(n));
                }
            }
        }
    }
    json!({ "ok": true, "counts": Value::Object(counts) })
}

/// Sum of guard counts across every macro's guard file — `Api._count_all_guards`,
/// which the stats summary totals. Non-JSON entries and unreadable files count as
/// zero, matching the source's `glob("*.json")` plus `except: pass`.
pub fn count_all_guards() -> i64 {
    let mut total = 0i64;
    if let Ok(entries) = std::fs::read_dir(paths::guards_dir()) {
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let Ok(data) = load_json(&path) else { continue };
            total += data.get("guards").and_then(Value::as_array).map_or(0, Vec::len) as i64;
        }
    }
    total
}

#[tauri::command]
pub fn guard_save(state: State<AppState>, macro_name: String, guards: Option<Vec<Value>>) -> Value {
    let parsed: Result<Vec<Guard>, _> = guards
        .unwrap_or_default()
        .into_iter()
        .map(serde_json::from_value)
        .collect();
    let objs = match parsed {
        Ok(objs) => objs,
        Err(e) => {
            state.emit("err", format!("Guard save failed: {e}"));
            return json!({ "ok": false, "error": e.to_string() });
        }
    };
    let file = GuardFile { guards: objs };
    // `save_guards` uses the macro name verbatim (no `.json` strip).
    let path = paths::guards_dir().join(format!("{macro_name}.json"));
    if let Err(e) = file.save_to(&path) {
        state.emit("err", format!("Guard save failed: {e}"));
        return json!({ "ok": false, "error": e.to_string() });
    }
    state.emit("ok", format!("Saved {} guard(s) for {macro_name}", file.guards.len()));
    json!({ "ok": true })
}

/// Dry-run a guard against a fresh frame — the guard editor's Test button, shared
/// by per-macro guards and vision triggers. The incoming dict is parsed through
/// [`Guard`] (the `Guard.from_dict` analogue, as `guard_save` does), then handed to
/// the vision sidecar, which grabs a frame, runs the verbatim `detect_guard`, and
/// returns the match summary plus a preview JPEG. The sidecar's result dict is
/// returned verbatim so the editor renders exactly what the desktop app did. Port
/// of `Api.guard_test`.
#[tauri::command]
pub fn guard_test(state: State<AppState>, guard: Value) -> Value {
    let g: Guard = match serde_json::from_value(guard) {
        Ok(g) => g,
        Err(e) => return json!({ "ok": false, "error": format!("Bad guard: {e}") }),
    };
    let (w, h) = state.core.resolve_screen();
    let backend = state.core.config.lock().unwrap().capture_backend.clone();
    match state.core.vision.guard_test(w as i64, h as i64, &backend, &g) {
        Ok(v) => v,
        Err(e) => json!({ "ok": false, "error": e.to_string() }),
    }
}

/// The four interactive pickers the guard/trigger editor opens, each a 1:1
/// replacement for the desktop `Api.*` method of the same name. The overlay runs
/// in the vision sidecar (brought up on first use exactly as `guard_test`); this
/// side just resolves the screen + capture backend and, for the template pickers,
/// hands over the Rust-owned save directory. Every one returns the sidecar's
/// result dict verbatim so the editor renders exactly what the desktop app did; a
/// transport failure collapses to `{ok:false, error}`, the shape the editor's
/// catch already handles (and, for `add_template_image`, filters "cancelled" from).
#[tauri::command]
pub fn guard_pick_color(state: State<AppState>) -> Value {
    let (w, h) = state.core.resolve_screen();
    let backend = state.core.config.lock().unwrap().capture_backend.clone();
    match state.core.vision.guard_pick_color(w as i64, h as i64, &backend) {
        Ok(v) => v,
        Err(e) => json!({ "ok": false, "error": e.to_string() }),
    }
}

#[tauri::command]
pub fn guard_pick_region(state: State<AppState>) -> Value {
    let (w, h) = state.core.resolve_screen();
    let backend = state.core.config.lock().unwrap().capture_backend.clone();
    match state.core.vision.guard_pick_region(w as i64, h as i64, &backend) {
        Ok(v) => v,
        Err(e) => json!({ "ok": false, "error": e.to_string() }),
    }
}

#[tauri::command]
pub fn capture_template(state: State<AppState>) -> Value {
    let (w, h) = state.core.resolve_screen();
    let backend = state.core.config.lock().unwrap().capture_backend.clone();
    let templates_dir = paths::templates_dir().to_string_lossy().into_owned();
    match state.core.vision.capture_template(w as i64, h as i64, &backend, &templates_dir) {
        Ok(v) => v,
        Err(e) => json!({ "ok": false, "error": e.to_string() }),
    }
}

#[tauri::command]
pub fn add_template_image(state: State<AppState>) -> Value {
    let (w, h) = state.core.resolve_screen();
    let backend = state.core.config.lock().unwrap().capture_backend.clone();
    let templates_dir = paths::templates_dir().to_string_lossy().into_owned();
    match state.core.vision.add_template_image(w as i64, h as i64, &backend, &templates_dir) {
        Ok(v) => v,
        Err(e) => json!({ "ok": false, "error": e.to_string() }),
    }
}

#[tauri::command]
pub fn surgical_capture(state: State<AppState>) -> Value {
    let (w, h) = state.core.resolve_screen();
    let backend = state.core.config.lock().unwrap().capture_backend.clone();
    let templates_dir = paths::templates_dir().to_string_lossy().into_owned();
    match state.core.vision.surgical_capture(w as i64, h as i64, &backend, &templates_dir) {
        Ok(v) => v,
        Err(e) => json!({ "ok": false, "error": e.to_string() }),
    }
}

/// Read a JSON file into a `Value`, collapsing IO and parse failures into one
/// error string — the single `except Exception` the Python readers use.
fn load_json(path: &Path) -> Result<Value, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}
