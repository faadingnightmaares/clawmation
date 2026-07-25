//! Import / export / bundle commands: the native file-dialog flows.
//!
//! 1:1 port of the pywebview `api.{export_chain, import_chain, export_macro,
//! import_macro, bulk_export, export_bundle, import_bundle}` methods. Python
//! opened a `tkinter.filedialog` modal synchronously inside each method and
//! returned `{ok, path|name, …}`; we keep that exact shape by driving
//! `tauri-plugin-dialog`'s `blocking_*` dialogs from the command body.
//!
//! Every command here MUST stay `#[tauri::command(async)]`. Tauri runs plain
//! sync handlers on the main thread, and `blocking_*` posts the dialog to the
//! main thread and then waits for it; from the main thread that is a
//! self-deadlock that hangs the whole window, not just the call.
//!
//! The tkinter "dialog failed to initialize" branches (`"Folder dialog failed:
//! …"`, `"File dialog failed: …"`) have no analogue here: a native dialog does
//! not fail to open the way a headless `tk.Tk()` can, so a dismissed dialog is
//! the single `"cancelled"` path, matching Python's `if not path` result.

use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::{DialogExt, FilePath};

use crate::models::chain::Chain;
use crate::models::guard::GuardFile;
use crate::models::macro_def::Macro;
use crate::paths;
use crate::state::AppState;

/// `name[:-5] if name.endswith(".json") else name`: the stem Python keys files by.
fn strip_json(name: &str) -> &str {
    name.strip_suffix(".json").unwrap_or(name)
}

/// A dialog result → a filesystem path, or `None` on cancel. Desktop always
/// yields the `Path` variant, so `into_path` never actually errors here.
fn picked(result: Option<FilePath>) -> Option<PathBuf> {
    result.and_then(|fp| fp.into_path().ok())
}

// ── Chains ────────────────────────────────────────────────────────────────

#[tauri::command(async)]
pub fn export_chain(app: AppHandle, state: State<AppState>, chain_id: String) -> Value {
    let Some(chain) = state.chains.list().into_iter().find(|c| c.id == chain_id) else {
        return json!({ "ok": false, "error": "Chain not found" });
    };
    let text = match serde_json::to_string_pretty(&chain) {
        Ok(t) => t,
        Err(e) => return json!({ "ok": false, "error": e.to_string() }),
    };
    let Some(path) = picked(
        app.dialog()
            .file()
            .add_filter("JSON files", &["json"])
            .add_filter("All files", &["*"])
            .set_file_name(format!("{}.chain.json", chain.name))
            .blocking_save_file(),
    ) else {
        return json!({ "ok": false, "error": "cancelled" });
    };
    match std::fs::write(&path, text) {
        Ok(()) => {
            state.emit("ok", format!("Exported chain '{}'", chain.name));
            json!({ "ok": true, "path": path.to_string_lossy() })
        }
        Err(e) => json!({ "ok": false, "error": e.to_string() }),
    }
}

#[tauri::command(async)]
pub fn import_chain(app: AppHandle, state: State<AppState>) -> Value {
    let Some(path) = picked(
        app.dialog()
            .file()
            .add_filter("JSON files", &["json"])
            .add_filter("All files", &["*"])
            .blocking_pick_file(),
    ) else {
        return json!({ "ok": false, "error": "cancelled" });
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => return json!({ "ok": false, "error": e.to_string() }),
    };
    let data: Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => return json!({ "ok": false, "error": e.to_string() }),
    };
    // Validate it's a chain before importing (Python's `"macro_names" in data`).
    if data.get("macro_names").is_none() {
        return json!({ "ok": false, "error": "Not a valid chain file" });
    }
    // `Chain`'s Deserialize applies the same `delay_between or 1.0` / `repeat or 1`
    // coercion as Python's `from_dict`; `ChainManager::add` then assigns a fresh id
    // and persists, exactly `data["id"] = new_id; _chains[new_id] = …; _save()`.
    let imported: Chain = match serde_json::from_value(data) {
        Ok(c) => c,
        Err(e) => return json!({ "ok": false, "error": e.to_string() }),
    };
    let c = state.chains.add(
        &imported.name,
        imported.macro_names,
        imported.delay_between,
        imported.repeat,
    );
    state.emit("ok", format!("Imported chain '{}'", c.name));
    json!({ "ok": true, "id": c.id, "name": c.name })
}

// ── Macros ────────────────────────────────────────────────────────────────

#[tauri::command(async)]
pub fn export_macro(app: AppHandle, state: State<AppState>, name: String) -> Value {
    let stem = strip_json(&name);
    let src = paths::macros_dir().join(format!("{stem}.json"));
    if !src.exists() {
        return json!({ "ok": false, "error": "Not found" });
    }
    let Some(dest) = picked(
        app.dialog()
            .file()
            .set_title("Export macro")
            .set_file_name(format!("{stem}.json"))
            .add_filter("Clawmation macro", &["json"])
            .add_filter("All files", &["*"])
            .blocking_save_file(),
    ) else {
        return json!({ "ok": false, "error": "cancelled" });
    };
    match std::fs::copy(&src, &dest) {
        Ok(_) => {
            state.emit("ok", format!("Exported '{stem}' → {}", dest.display()));
            json!({ "ok": true, "path": dest.to_string_lossy() })
        }
        Err(e) => json!({ "ok": false, "error": e.to_string() }),
    }
}

#[tauri::command(async)]
pub fn import_macro(app: AppHandle, state: State<AppState>) -> Value {
    let Some(src) = picked(
        app.dialog()
            .file()
            .set_title("Import macro")
            .add_filter("Clawmation macro", &["json"])
            .add_filter("All files", &["*"])
            .blocking_pick_file(),
    ) else {
        return json!({ "ok": false, "error": "cancelled" });
    };
    // Validate it's a real macro before importing.
    let mut macro_def = match Macro::load(&src) {
        Ok(m) => m,
        Err(e) => return json!({ "ok": false, "error": format!("Not a valid macro: {e}") }),
    };
    // Avoid clobbering an existing macro of the same name.
    let macros_dir = paths::macros_dir();
    let base = macro_def.name.clone();
    let mut dest = macros_dir.join(format!("{base}.json"));
    let mut counter = 2;
    while dest.exists() {
        macro_def.name = format!("{base}_{counter}");
        dest = macros_dir.join(format!("{}.json", macro_def.name));
        counter += 1;
    }
    match macro_def.save_to(&dest) {
        Ok(()) => {
            state.emit("ok", format!("Imported '{}'", macro_def.name));
            json!({ "ok": true, "name": macro_def.name })
        }
        Err(e) => json!({ "ok": false, "error": e.to_string() }),
    }
}

#[tauri::command(async)]
pub fn bulk_export(app: AppHandle, state: State<AppState>, names: Vec<String>) -> Value {
    if names.is_empty() {
        return json!({ "ok": false, "error": "No macros selected" });
    }
    let Some(dest_dir) = picked(
        app.dialog()
            .file()
            .set_title("Export macros to folder")
            .blocking_pick_folder(),
    ) else {
        return json!({ "ok": false, "error": "cancelled" });
    };
    let macros_dir = paths::macros_dir();
    let mut exported: Vec<String> = Vec::new();
    let mut failed: Vec<String> = Vec::new();
    for name in &names {
        let stem = strip_json(name);
        let src = macros_dir.join(format!("{stem}.json"));
        if src.exists() {
            match std::fs::copy(&src, dest_dir.join(format!("{stem}.json"))) {
                Ok(_) => exported.push(stem.to_string()),
                Err(_) => failed.push(stem.to_string()),
            }
        } else {
            failed.push(stem.to_string());
        }
    }
    if !exported.is_empty() {
        state.emit(
            "ok",
            format!("Exported {} macro(s) → {}", exported.len(), dest_dir.display()),
        );
    }
    json!({
        "ok": true,
        "exported": exported,
        "failed": failed,
        "dir": dest_dir.to_string_lossy(),
    })
}

// ── Bundles (.clawbundle = zip of macro.json + guards.json + templates/) ─────

#[tauri::command(async)]
pub fn export_bundle(app: AppHandle, state: State<AppState>, name: String) -> Value {
    let stem = strip_json(&name);
    let macro_path = paths::macros_dir().join(format!("{stem}.json"));
    if !macro_path.exists() {
        return json!({ "ok": false, "error": "Not found" });
    }
    let Some(dest) = picked(
        app.dialog()
            .file()
            .set_title("Export bundle")
            .set_file_name(format!("{stem}.clawbundle"))
            .add_filter("Clawmation bundle", &["clawbundle"])
            .add_filter("All files", &["*"])
            .blocking_save_file(),
    ) else {
        return json!({ "ok": false, "error": "cancelled" });
    };
    let guards_path = paths::guards_dir().join(format!("{stem}.json"));
    match write_bundle(&macro_path, &guards_path, &dest) {
        Ok(()) => {
            state.emit("ok", format!("Exported bundle '{stem}' → {}", dest.display()));
            json!({ "ok": true, "path": dest.to_string_lossy() })
        }
        Err(e) => json!({ "ok": false, "error": e.to_string() }),
    }
}

#[tauri::command(async)]
pub fn import_bundle(app: AppHandle, state: State<AppState>) -> Value {
    let Some(src) = picked(
        app.dialog()
            .file()
            .set_title("Import bundle")
            .add_filter("Clawmation bundle", &["clawbundle"])
            .add_filter("All files", &["*"])
            .blocking_pick_file(),
    ) else {
        return json!({ "ok": false, "error": "cancelled" });
    };
    match read_bundle(
        &src,
        &paths::macros_dir(),
        &paths::templates_dir(),
        &paths::guards_dir(),
    ) {
        // `Ok(None)` is the "no macro.json" sentinel: its own error string, not
        // the generic prefixed one, matching Python's two distinct messages.
        Ok(Some(name)) => {
            state.emit("ok", format!("Imported bundle → '{name}'"));
            json!({ "ok": true, "name": name })
        }
        Ok(None) => json!({ "ok": false, "error": "Not a valid bundle (no macro.json)" }),
        Err(e) => json!({ "ok": false, "error": format!("Not a valid bundle: {e}") }),
    }
}

// ── Bundle helpers (path-parameterized so the round-trip is unit-testable) ───

/// Write `dest` as a `.clawbundle` zip: `macro.json`, then `guards.json` and each
/// distinct template image its guards reference. Mirrors Python's `export_bundle`
/// zip assembly.
fn write_bundle(
    macro_path: &Path,
    guards_path: &Path,
    dest: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let file = std::fs::File::create(dest)?;
    let mut zip = zip::ZipWriter::new(file);
    let opts =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    // 1. The macro itself.
    zip.start_file("macro.json", opts)?;
    zip.write_all(&std::fs::read(macro_path)?)?;

    // 2. Guards (if any) + 3. any template images they reference.
    if guards_path.exists() {
        zip.start_file("guards.json", opts)?;
        zip.write_all(&std::fs::read(guards_path)?)?;
        let mut templates_added: HashSet<String> = HashSet::new();
        for g in GuardFile::load(guards_path).guards {
            if g.template_path.is_empty() {
                continue;
            }
            let tp = Path::new(&g.template_path);
            let Some(tpl_name) = tp.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if tp.exists() && templates_added.insert(tpl_name.to_string()) {
                zip.start_file(format!("templates/{tpl_name}"), opts)?;
                zip.write_all(&std::fs::read(tp)?)?;
            }
        }
    }
    zip.finish()?;
    Ok(())
}

/// Install a `.clawbundle`: templates first (so guard paths resolve), then the
/// macro (clobber-avoiding), then guards (template paths remapped to installed
/// locations). Returns the installed macro name, or `None` if there is no
/// `macro.json` entry. Mirrors Python's `import_bundle`.
fn read_bundle(
    src: &Path,
    macros_dir: &Path,
    templates_dir: &Path,
    guards_dir: &Path,
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let file = std::fs::File::open(src)?;
    let mut archive = zip::ZipArchive::new(file)?;
    let names: Vec<String> = archive.file_names().map(str::to_string).collect();
    if !names.iter().any(|n| n == "macro.json") {
        return Ok(None);
    }

    // 1. Extract + install templates first (so guard paths resolve).
    std::fs::create_dir_all(templates_dir)?;
    let mut tpl_remap: HashMap<String, String> = HashMap::new();
    for n in &names {
        if !(n.starts_with("templates/") && !n.ends_with('/')) {
            continue;
        }
        let Some(basename) = Path::new(n).file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let dest_tpl = templates_dir.join(basename);
        // Keep an existing template; only extract when the name is new. Either
        // way the basename remaps to the installed path.
        if !dest_tpl.exists() {
            let mut fin = archive.by_name(n)?;
            let mut buf = Vec::new();
            fin.read_to_end(&mut buf)?;
            std::fs::write(&dest_tpl, &buf)?;
        }
        tpl_remap.insert(basename.to_string(), dest_tpl.to_string_lossy().into_owned());
    }

    // 2. Install the macro (avoid name clobber).
    let mut macro_def: Macro = {
        let mut f = archive.by_name("macro.json")?;
        let mut buf = String::new();
        f.read_to_string(&mut buf)?;
        serde_json::from_str(&buf)?
    };
    let base = macro_def.name.clone();
    let mut dest_macro = macros_dir.join(format!("{base}.json"));
    let mut counter = 2;
    while dest_macro.exists() {
        macro_def.name = format!("{base}_{counter}");
        dest_macro = macros_dir.join(format!("{}.json", macro_def.name));
        counter += 1;
    }
    std::fs::create_dir_all(macros_dir)?;
    macro_def.save_to(&dest_macro)?;

    // 3. Install guards (remap template basenames to installed paths).
    if names.iter().any(|n| n == "guards.json") {
        let mut buf = String::new();
        archive.by_name("guards.json")?.read_to_string(&mut buf)?;
        let mut gdata: Value = serde_json::from_str(&buf)?;
        if let Some(guards) = gdata.get_mut("guards").and_then(Value::as_array_mut) {
            for g in guards.iter_mut() {
                let bn = g
                    .get("template_path")
                    .and_then(Value::as_str)
                    .filter(|tp| !tp.is_empty())
                    .and_then(|tp| Path::new(tp).file_name().and_then(|s| s.to_str()))
                    .map(str::to_string);
                if let Some(installed) = bn.as_deref().and_then(|bn| tpl_remap.get(bn)) {
                    g["template_path"] = json!(installed);
                }
            }
        }
        std::fs::create_dir_all(guards_dir)?;
        let out = serde_json::to_string_pretty(&gdata)?;
        std::fs::write(guards_dir.join(format!("{}.json", macro_def.name)), out)?;
    }

    Ok(Some(macro_def.name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_dir;

    /// A bundle written with a macro + a guard that references a template must
    /// round-trip: importing into a fresh data root installs all three, and the
    /// guard's `template_path` is remapped to the freshly-installed template.
    #[test]
    fn bundle_round_trip_installs_macro_guards_and_remapped_template() {
        let root = temp_dir("bundle_round_trip");
        let src_macros = root.join("src/macros");
        let src_guards = root.join("src/macros/guards");
        let src_templates = root.join("src/templates");
        for d in [&src_macros, &src_guards, &src_templates] {
            std::fs::create_dir_all(d).unwrap();
        }

        // A minimal but load-valid macro (requires `name`; one event with the
        // subscript-read `type`/`timestamp` keys).
        let macro_path = src_macros.join("__test___bundle.json");
        std::fs::write(
            &macro_path,
            r#"{"name":"__test___bundle","events":[{"type":"KEY_PRESS","timestamp":0.0,"key":"a"}]}"#,
        )
        .unwrap();

        // A template image and a guard that points at it by absolute path.
        let tpl_path = src_templates.join("__test___btn.png");
        std::fs::write(&tpl_path, b"\x89PNG\r\n\x1a\nfake-image-bytes").unwrap();
        let guards_path = src_guards.join("__test___bundle.json");
        std::fs::write(
            &guards_path,
            format!(
                r#"{{"guards":[{{"name":"g1","template_path":{}}}]}}"#,
                serde_json::to_string(&tpl_path.to_string_lossy()).unwrap()
            ),
        )
        .unwrap();

        // Export.
        let bundle = root.join("out.clawbundle");
        write_bundle(&macro_path, &guards_path, &bundle).expect("bundle writes");
        assert!(bundle.exists(), "bundle file created");

        // Import into a fresh, separate data root.
        let dst_macros = root.join("dst/macros");
        let dst_templates = root.join("dst/templates");
        let dst_guards = root.join("dst/macros/guards");
        let name = read_bundle(&bundle, &dst_macros, &dst_templates, &dst_guards)
            .expect("bundle reads")
            .expect("has macro.json");
        assert_eq!(name, "__test___bundle");

        // Macro installed.
        assert!(dst_macros.join("__test___bundle.json").exists(), "macro installed");
        // Template installed under the destination templates dir.
        let installed_tpl = dst_templates.join("__test___btn.png");
        assert!(installed_tpl.exists(), "template installed");
        // Guard installed with its template_path remapped to the new location.
        let gtext = std::fs::read_to_string(dst_guards.join("__test___bundle.json")).unwrap();
        let gjson: Value = serde_json::from_str(&gtext).unwrap();
        let remapped = gjson["guards"][0]["template_path"].as_str().unwrap();
        assert_eq!(
            Path::new(remapped),
            installed_tpl.as_path(),
            "guard template_path remapped to installed template"
        );
    }

    /// A zip without `macro.json` is rejected with the `Ok(None)` sentinel that
    /// the command maps to Python's "no macro.json" message.
    #[test]
    fn bundle_without_macro_json_is_rejected() {
        let root = temp_dir("bundle_no_macro");
        let bundle = root.join("bad.clawbundle");
        {
            let file = std::fs::File::create(&bundle).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let opts = zip::write::SimpleFileOptions::default();
            zip.start_file("notes.txt", opts).unwrap();
            zip.write_all(b"no macro here").unwrap();
            zip.finish().unwrap();
        }
        let out = read_bundle(
            &bundle,
            &root.join("m"),
            &root.join("t"),
            &root.join("g"),
        )
        .expect("reads");
        assert!(out.is_none(), "missing macro.json → Ok(None)");
    }
}
