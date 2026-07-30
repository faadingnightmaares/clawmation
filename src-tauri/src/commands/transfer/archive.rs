//! Compact, versioned `.clawmation` and `.clawbundle` containers.
//!
//! JSON payloads use high-ratio Zstandard compression. PNG/JPEG/WebP assets are
//! already compressed, so they are stored byte-for-byte; BMP assets use Zstd.
//! Every payload is BLAKE3-verified, identical bundle images are
//! content-deduplicated, and all reads are bounded before allocation.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{Read, Seek, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::models::guard::GuardFile;
use crate::models::macro_def::{Macro, CURRENT_MACRO_FORMAT_VERSION};
use crate::models::node_graph::NodeGraph;
use crate::models::node_graph_assets::{image_paths, remap_image_paths};
use crate::models::vision_images::{candidate_limit_error, candidate_paths};

const CONTAINER_VERSION: u32 = 1;
const MACRO_FORMAT: &str = "com.clawmation.macro";
const BUNDLE_FORMAT: &str = "com.clawmation.bundle";
const LOOP_FORMAT: &str = "com.clawmation.loop";
const MANIFEST_PATH: &str = "manifest.json";
const MACRO_PATH: &str = "payload/macro.json";
const GUARDS_PATH: &str = "payload/guards.json";
const LOOP_PATH: &str = "payload/loop.json";

const MAX_ARCHIVE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_EXPANDED_BYTES: u64 = 768 * 1024 * 1024;
const MAX_MACRO_BYTES: u64 = 256 * 1024 * 1024;
const MAX_GUARDS_BYTES: u64 = 16 * 1024 * 1024;
const MAX_LOOP_BYTES: u64 = 32 * 1024 * 1024;
const MAX_ASSET_BYTES: u64 = 64 * 1024 * 1024;
const MAX_MANIFEST_BYTES: u64 = 256 * 1024;
const MAX_ENTRIES: usize = 2_048;

type ArchiveResult<T> = Result<T, Box<dyn std::error::Error>>;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Manifest {
    #[serde(rename = "format")]
    format_id: String,
    version: u32,
    app_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    macro_file: Option<FileRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    guards_file: Option<FileRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    loop_file: Option<FileRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    assets: Vec<FileRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FileRecord {
    path: String,
    blake3: String,
    bytes: u64,
}

struct PreparedFile {
    record: FileRecord,
    bytes: Vec<u8>,
    compress: bool,
}

pub(super) fn write_macro(macro_path: &Path, dest: &Path) -> ArchiveResult<()> {
    let macro_bytes = compact_macro(macro_path)?;
    let macro_file = prepared(MACRO_PATH, macro_bytes, true);
    let manifest = Manifest {
        format_id: MACRO_FORMAT.to_string(),
        version: CONTAINER_VERSION,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        macro_file: Some(macro_file.record.clone()),
        guards_file: None,
        loop_file: None,
        assets: Vec::new(),
    };
    write_archive(dest, manifest, vec![macro_file])
}

pub(super) fn read_macro(src: &Path, macros_dir: &Path) -> ArchiveResult<String> {
    reject_large_archive(src)?;
    if !is_zip(src)? {
        return install_legacy_json(src, macros_dir);
    }

    let mut archive = zip::ZipArchive::new(File::open(src)?)?;
    let names = validate_archive(&mut archive)?;
    if !names.contains(MANIFEST_PATH) {
        return invalid("missing manifest.json");
    }
    let manifest = read_manifest(&mut archive)?;
    validate_manifest(&manifest, MACRO_FORMAT)?;
    if manifest.guards_file.is_some() || !manifest.assets.is_empty() {
        return invalid("a .clawmation file cannot contain bundle data");
    }
    validate_declared_entries(&names, &manifest)?;
    let macro_file = manifest
        .macro_file
        .as_ref()
        .ok_or_else(|| invalid_error("missing macro payload"))?;
    let bytes = read_record(&mut archive, macro_file, MAX_MACRO_BYTES)?;
    let macro_def: Macro = serde_json::from_slice(&bytes)?;
    install_macro(macro_def, macros_dir)
}

pub(super) fn write_bundle(
    macro_path: &Path,
    guards_path: &Path,
    dest: &Path,
) -> ArchiveResult<()> {
    let macro_bytes = compact_macro(macro_path)?;
    let macro_file = prepared(MACRO_PATH, macro_bytes, true);
    let mut extras = Vec::new();
    let mut asset_records = Vec::new();
    let guards_file = if guards_path.exists() {
        let raw = std::fs::read(guards_path)?;
        if raw.len() as u64 > MAX_GUARDS_BYTES {
            return invalid("guard data is too large");
        }
        let mut guards: GuardFile = serde_json::from_slice(&raw)?;
        let mut content_paths: HashMap<String, String> = HashMap::new();

        for guard in &mut guards.guards {
            if let Some(error) = candidate_limit_error(&guard.template_path, &guard.template_paths)
            {
                return invalid(format!("guard '{}': {error}", guard.name));
            }
            let configured = candidate_paths(&guard.template_path, &guard.template_paths)
                .into_iter()
                .map(str::to_string)
                .collect::<Vec<_>>();
            let mut archived = Vec::with_capacity(configured.len());
            for configured_path in configured {
                let source = resolve_template_path(&configured_path);
                if !source.is_file() {
                    return invalid(format!("vision image is missing: {}", source.display()));
                }
                let bytes = std::fs::read(&source)?;
                if bytes.len() as u64 > MAX_ASSET_BYTES {
                    return invalid(format!("vision image is too large: {}", source.display()));
                }
                let digest = digest(&bytes);
                let archive_path = if let Some(existing) = content_paths.get(&digest) {
                    existing.clone()
                } else {
                    let extension = safe_image_extension(&source)?;
                    let archive_path = format!("assets/{digest}.{extension}");
                    let compress = extension == "bmp";
                    let file = prepared(&archive_path, bytes, compress);
                    asset_records.push(file.record.clone());
                    extras.push(file);
                    content_paths.insert(digest, archive_path.clone());
                    archive_path
                };
                archived.push(archive_path);
            }
            set_guard_candidates(guard, archived);
        }

        let bytes = serde_json::to_vec(&guards)?;
        Some(prepared(GUARDS_PATH, bytes, true))
    } else {
        None
    };

    let manifest = Manifest {
        format_id: BUNDLE_FORMAT.to_string(),
        version: CONTAINER_VERSION,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        macro_file: Some(macro_file.record.clone()),
        guards_file: guards_file.as_ref().map(|file| file.record.clone()),
        loop_file: None,
        assets: asset_records,
    };
    extras.push(macro_file);
    if let Some(guards_file) = guards_file {
        extras.push(guards_file);
    }
    write_archive(dest, manifest, extras)
}

/// Returns `Ok(None)` for a legacy ZIP with no `macro.json`, preserving the
/// command's existing, user-friendly invalid-bundle error.
pub(super) fn read_bundle(
    src: &Path,
    macros_dir: &Path,
    templates_dir: &Path,
    guards_dir: &Path,
) -> ArchiveResult<Option<String>> {
    reject_large_archive(src)?;
    let mut archive = zip::ZipArchive::new(File::open(src)?)?;
    let names = validate_archive(&mut archive)?;
    if !names.contains(MANIFEST_PATH) {
        return read_legacy_bundle(archive, names, macros_dir, templates_dir, guards_dir);
    }

    let manifest = read_manifest(&mut archive)?;
    validate_manifest(&manifest, BUNDLE_FORMAT)?;
    validate_declared_entries(&names, &manifest)?;

    let macro_file = manifest
        .macro_file
        .as_ref()
        .ok_or_else(|| invalid_error("missing macro payload"))?;
    let macro_bytes = read_record(&mut archive, macro_file, MAX_MACRO_BYTES)?;
    let macro_def: Macro = serde_json::from_slice(&macro_bytes)?;
    validate_macro(&macro_def)?;

    // Parse and validate every logical payload before writing anything. A bad
    // guard file or dangling image reference must not leave a partial import.
    let mut guards = if let Some(record) = &manifest.guards_file {
        let bytes = read_record(&mut archive, record, MAX_GUARDS_BYTES)?;
        Some(serde_json::from_slice::<GuardFile>(&bytes)?)
    } else {
        None
    };
    let declared_assets: HashSet<&str> = manifest
        .assets
        .iter()
        .map(|record| record.path.as_str())
        .collect();
    if let Some(guards) = &guards {
        for guard in &guards.guards {
            if let Some(error) = candidate_limit_error(&guard.template_path, &guard.template_paths)
            {
                return invalid(format!("guard '{}': {error}", guard.name));
            }
            for path in candidate_paths(&guard.template_path, &guard.template_paths) {
                if !declared_assets.contains(path) {
                    return invalid(format!("guard references an undeclared image: {path}"));
                }
            }
        }
    }

    std::fs::create_dir_all(templates_dir)?;
    let mut installed_assets = HashMap::new();
    for record in &manifest.assets {
        if !record.path.starts_with("assets/") {
            return invalid("bundle asset is outside assets/");
        }
        let extension = safe_image_extension(Path::new(&record.path))?;
        let bytes = read_record(&mut archive, record, MAX_ASSET_BYTES)?;
        let filename = format!("{}.{}", record.blake3, extension);
        let installed = templates_dir.join(filename);
        if !installed.exists() || digest(&std::fs::read(&installed)?) != record.blake3 {
            crate::util::write_atomic(&installed, &bytes)?;
        }
        installed_assets.insert(record.path.clone(), installed);
    }

    let name = install_macro(macro_def, macros_dir)?;
    if let Some(guards) = &mut guards {
        for guard in &mut guards.guards {
            let installed = candidate_paths(&guard.template_path, &guard.template_paths)
                .into_iter()
                .filter_map(|path| installed_assets.get(path))
                .map(|path| path.to_string_lossy().into_owned())
                .collect();
            set_guard_candidates(guard, installed);
        }
        guards.save_to(&guards_dir.join(format!("{name}.json")))?;
    }
    Ok(Some(name))
}

pub(super) fn write_loop(graph_path: &Path, dest: &Path) -> ArchiveResult<()> {
    let mut graph = NodeGraph::load(graph_path).map_err(invalid_error)?;
    let report = graph.validate();
    if !report.ok {
        return invalid(format!("invalid Loop: {}", report.errors.join("; ")));
    }

    let mut extras = Vec::new();
    let mut asset_records = Vec::new();
    let mut content_paths: HashMap<String, String> = HashMap::new();
    let mut remap = HashMap::new();

    for configured_path in image_paths(&graph) {
        let source = resolve_template_path(&configured_path);
        if !source.is_file() {
            return invalid(format!("Loop image is missing: {}", source.display()));
        }
        let bytes = std::fs::read(&source)?;
        if bytes.len() as u64 > MAX_ASSET_BYTES {
            return invalid(format!("Loop image is too large: {}", source.display()));
        }
        let digest = digest(&bytes);
        let archive_path = if let Some(existing) = content_paths.get(&digest) {
            existing.clone()
        } else {
            let extension = safe_image_extension(&source)?;
            let archive_path = format!("assets/{digest}.{extension}");
            let file = prepared(&archive_path, bytes, extension == "bmp");
            asset_records.push(file.record.clone());
            extras.push(file);
            content_paths.insert(digest, archive_path.clone());
            archive_path
        };
        remap.insert(configured_path, archive_path);
    }
    remap_image_paths(&mut graph, &remap);

    let loop_bytes = serde_json::to_vec(&graph)?;
    if loop_bytes.len() as u64 > MAX_LOOP_BYTES {
        return invalid("Loop data is too large");
    }
    let loop_file = prepared(LOOP_PATH, loop_bytes, true);
    let manifest = Manifest {
        format_id: LOOP_FORMAT.to_string(),
        version: CONTAINER_VERSION,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        macro_file: None,
        guards_file: None,
        loop_file: Some(loop_file.record.clone()),
        assets: asset_records,
    };
    extras.push(loop_file);
    write_archive(dest, manifest, extras)
}

pub(super) fn read_loop(
    src: &Path,
    nodes_dir: &Path,
    templates_dir: &Path,
) -> ArchiveResult<String> {
    reject_large_archive(src)?;
    let mut archive = zip::ZipArchive::new(File::open(src)?)?;
    let names = validate_archive(&mut archive)?;
    if !names.contains(MANIFEST_PATH) {
        return invalid("legacy bundles do not contain Loops");
    }
    let manifest = read_manifest(&mut archive)?;
    validate_manifest(&manifest, LOOP_FORMAT)?;
    validate_declared_entries(&names, &manifest)?;

    let loop_file = manifest
        .loop_file
        .as_ref()
        .ok_or_else(|| invalid_error("missing Loop payload"))?;
    let graph_bytes = read_record(&mut archive, loop_file, MAX_LOOP_BYTES)?;
    let mut graph: NodeGraph = serde_json::from_slice(&graph_bytes)?;
    let report = graph.validate();
    if !report.ok {
        return invalid(format!("invalid Loop: {}", report.errors.join("; ")));
    }

    let declared_assets = manifest
        .assets
        .iter()
        .map(|record| record.path.as_str())
        .collect::<HashSet<_>>();
    for path in image_paths(&graph) {
        if !declared_assets.contains(path.as_str()) {
            return invalid(format!("Loop references an undeclared image: {path}"));
        }
    }

    std::fs::create_dir_all(templates_dir)?;
    let mut installed_assets = HashMap::new();
    for record in &manifest.assets {
        if !record.path.starts_with("assets/") {
            return invalid("Loop asset is outside assets/");
        }
        let extension = safe_image_extension(Path::new(&record.path))?;
        let bytes = read_record(&mut archive, record, MAX_ASSET_BYTES)?;
        let filename = format!("{}.{}", record.blake3, extension);
        let installed = templates_dir.join(filename);
        if !installed.exists() || digest(&std::fs::read(&installed)?) != record.blake3 {
            crate::util::write_atomic(&installed, &bytes)?;
        }
        installed_assets.insert(
            record.path.clone(),
            installed.to_string_lossy().into_owned(),
        );
    }
    remap_image_paths(&mut graph, &installed_assets);
    install_loop(graph, nodes_dir)
}

fn resolve_template_path(configured: &str) -> PathBuf {
    let path = Path::new(configured);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        crate::paths::templates_dir().join(path)
    }
}

fn set_guard_candidates(guard: &mut crate::models::guard::Guard, candidates: Vec<String>) {
    let mut candidates = candidates.into_iter();
    guard.template_path = candidates.next().unwrap_or_default();
    guard.template_paths = candidates.collect();
}

fn install_loop(mut graph: NodeGraph, nodes_dir: &Path) -> ArchiveResult<String> {
    validate_macro_name(&graph.name)?;
    std::fs::create_dir_all(nodes_dir)?;
    let base = graph.name.clone();
    let mut name = base.clone();
    let mut destination = nodes_dir.join(format!("{name}.json"));
    let mut counter = 2_u32;
    while destination.exists() {
        name = format!("{base} {counter}");
        destination = nodes_dir.join(format!("{name}.json"));
        counter = counter
            .checked_add(1)
            .ok_or_else(|| invalid_error("too many Loops with the same name"))?;
    }
    graph.name = name.clone();
    graph.save_to(&destination).map_err(invalid_error)?;
    Ok(name)
}

fn prepared(path: &str, bytes: Vec<u8>, compress: bool) -> PreparedFile {
    PreparedFile {
        record: FileRecord {
            path: path.to_string(),
            blake3: digest(&bytes),
            bytes: bytes.len() as u64,
        },
        bytes,
        compress,
    }
}

fn compact_macro(path: &Path) -> ArchiveResult<Vec<u8>> {
    let macro_def = Macro::load(path)?;
    validate_macro(&macro_def)?;
    Ok(serde_json::to_vec(&macro_def)?)
}

fn validate_macro(macro_def: &Macro) -> ArchiveResult<()> {
    validate_macro_name(&macro_def.name)?;
    if macro_def.format_version == 0 || macro_def.format_version > CURRENT_MACRO_FORMAT_VERSION {
        return invalid(format!(
            "unsupported macro format {}",
            macro_def.format_version
        ));
    }
    if !macro_def.events.is_empty() {
        macro_def
            .validate_for_playback()
            .map_err(|error| invalid_error(format!("invalid macro: {error}")))?;
    }
    Ok(())
}

fn validate_macro_name(name: &str) -> ArchiveResult<()> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed == "." || trimmed == ".." || trimmed.chars().count() > 128 {
        return invalid("macro name is empty, reserved, or too long");
    }
    if trimmed.chars().any(|c| {
        c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
    }) {
        return invalid("macro name contains characters Windows cannot save");
    }
    let stem = trimmed
        .split('.')
        .next()
        .unwrap_or(trimmed)
        .to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit()
            && stem.as_bytes()[3] != b'0');
    if reserved {
        return invalid("macro name is reserved by Windows");
    }
    Ok(())
}

fn install_macro(mut macro_def: Macro, macros_dir: &Path) -> ArchiveResult<String> {
    validate_macro(&macro_def)?;
    std::fs::create_dir_all(macros_dir)?;
    let base = macro_def.name.clone();
    let mut dest = macros_dir.join(format!("{base}.json"));
    let mut counter = 2_u32;
    while dest.exists() {
        macro_def.name = format!("{base}_{counter}");
        dest = macros_dir.join(format!("{}.json", macro_def.name));
        counter = counter
            .checked_add(1)
            .ok_or_else(|| invalid_error("too many macros with the same name"))?;
    }
    macro_def.save_to(&dest)?;
    Ok(macro_def.name)
}

fn install_legacy_json(src: &Path, macros_dir: &Path) -> ArchiveResult<String> {
    let metadata = std::fs::metadata(src)?;
    if metadata.len() > MAX_MACRO_BYTES {
        return invalid("legacy macro is too large");
    }
    let bytes = std::fs::read(src)?;
    let macro_def: Macro = serde_json::from_slice(&bytes)?;
    install_macro(macro_def, macros_dir)
}

fn write_archive(
    dest: &Path,
    manifest: Manifest,
    mut extras: Vec<PreparedFile>,
) -> ArchiveResult<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = temporary_sibling(dest);
    let result = (|| -> ArchiveResult<()> {
        let file = File::create(&tmp)?;
        let mut zip = zip::ZipWriter::new(file);
        let manifest_bytes = serde_json::to_vec(&manifest)?;
        write_zip_file(&mut zip, MANIFEST_PATH, &manifest_bytes, true)?;
        let primary = manifest
            .macro_file
            .as_ref()
            .or(manifest.loop_file.as_ref())
            .ok_or_else(|| invalid_error("internal archive has no primary payload"))?;
        write_zip_file(
            &mut zip,
            &primary.path,
            &extract_manifest_payload(primary, &mut extras)?,
            true,
        )?;
        if let Some(record) = &manifest.guards_file {
            let bytes = extract_manifest_payload(record, &mut extras)?;
            write_zip_file(&mut zip, &record.path, &bytes, true)?;
        }
        for file in extras {
            write_zip_file(&mut zip, &file.record.path, &file.bytes, file.compress)?;
        }
        let file = zip.finish()?;
        file.sync_all()?;
        Ok(())
    })();
    if let Err(error) = result {
        let _ = std::fs::remove_file(&tmp);
        return Err(error);
    }
    if dest.exists() {
        std::fs::remove_file(dest)?;
    }
    std::fs::rename(&tmp, dest)?;
    Ok(())
}

/// `Manifest` owns records while prepared payload bytes live in `extras`. The
/// primary/guards entries are inserted there by the constructor below.
fn extract_manifest_payload(
    record: &FileRecord,
    extras: &mut Vec<PreparedFile>,
) -> ArchiveResult<Vec<u8>> {
    let Some(index) = extras
        .iter()
        .position(|file| file.record.path == record.path)
    else {
        return invalid(format!("internal archive payload missing: {}", record.path));
    };
    Ok(extras.remove(index).bytes)
}

fn write_zip_file<W: Write + Seek>(
    zip: &mut zip::ZipWriter<W>,
    path: &str,
    bytes: &[u8],
    compress: bool,
) -> ArchiveResult<()> {
    let method = if compress {
        zip::CompressionMethod::Zstd
    } else {
        zip::CompressionMethod::Stored
    };
    let mut options = zip::write::SimpleFileOptions::default()
        .compression_method(method)
        .unix_permissions(0o644);
    if compress {
        // Level 10 is the useful size/speed knee for large event timelines:
        // noticeably denser than the default while exports remain interactive.
        options = options.compression_level(Some(10));
    }
    zip.start_file(path, options)?;
    zip.write_all(bytes)?;
    Ok(())
}

fn temporary_sibling(dest: &Path) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let filename = dest
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("clawmation");
    dest.with_file_name(format!(".{filename}.{nonce}.tmp"))
}

fn read_manifest<R: Read + Seek>(archive: &mut zip::ZipArchive<R>) -> ArchiveResult<Manifest> {
    let bytes = read_named(archive, MANIFEST_PATH, MAX_MANIFEST_BYTES)?;
    Ok(serde_json::from_slice(&bytes)?)
}

fn validate_manifest(manifest: &Manifest, expected_format: &str) -> ArchiveResult<()> {
    if manifest.format_id != expected_format {
        return invalid(format!(
            "wrong container type '{}' (expected '{expected_format}')",
            manifest.format_id
        ));
    }
    if manifest.version != CONTAINER_VERSION {
        return invalid(format!(
            "unsupported container version {} (supported: {})",
            manifest.version, CONTAINER_VERSION
        ));
    }
    match expected_format {
        MACRO_FORMAT | BUNDLE_FORMAT => {
            if manifest
                .macro_file
                .as_ref()
                .is_none_or(|record| record.path != MACRO_PATH)
                || manifest.loop_file.is_some()
            {
                return invalid("invalid macro payload path");
            }
            if manifest
                .guards_file
                .as_ref()
                .is_some_and(|record| record.path != GUARDS_PATH)
            {
                return invalid("invalid guards payload path");
            }
            if expected_format == MACRO_FORMAT
                && (manifest.guards_file.is_some() || !manifest.assets.is_empty())
            {
                return invalid("a .clawmation file cannot contain bundle data");
            }
        }
        LOOP_FORMAT => {
            if manifest
                .loop_file
                .as_ref()
                .is_none_or(|record| record.path != LOOP_PATH)
                || manifest.macro_file.is_some()
                || manifest.guards_file.is_some()
            {
                return invalid("invalid Loop payload path");
            }
        }
        _ => return invalid("unsupported container type"),
    }
    Ok(())
}

fn validate_declared_entries(names: &HashSet<String>, manifest: &Manifest) -> ArchiveResult<()> {
    let primary = manifest
        .macro_file
        .as_ref()
        .or(manifest.loop_file.as_ref())
        .ok_or_else(|| invalid_error("manifest has no primary payload"))?;
    let mut declared = HashSet::from([MANIFEST_PATH.to_string(), primary.path.clone()]);
    if let Some(record) = &manifest.guards_file {
        declared.insert(record.path.clone());
    }
    for record in &manifest.assets {
        if !declared.insert(record.path.clone()) {
            return invalid(format!("duplicate manifest path: {}", record.path));
        }
    }
    if names != &declared {
        return invalid("archive contains missing, duplicate, or undeclared files");
    }
    Ok(())
}

fn validate_archive<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
) -> ArchiveResult<HashSet<String>> {
    if archive.len() > MAX_ENTRIES {
        return invalid("archive contains too many files");
    }
    let mut names = HashSet::with_capacity(archive.len());
    let mut expanded = 0_u64;
    for index in 0..archive.len() {
        let file = archive.by_index(index)?;
        let name = file.name().to_string();
        validate_archive_path(&name)?;
        if file.is_dir() {
            return invalid("directory entries are not allowed");
        }
        if !names.insert(name) {
            return invalid("archive contains duplicate file names");
        }
        expanded = expanded
            .checked_add(file.size())
            .ok_or_else(|| invalid_error("expanded archive size overflow"))?;
        if expanded > MAX_EXPANDED_BYTES {
            return invalid("expanded archive is too large");
        }
    }
    Ok(names)
}

fn validate_archive_path(name: &str) -> ArchiveResult<()> {
    if name.is_empty() || name.contains('\\') || name.contains(':') {
        return invalid("archive contains an unsafe path");
    }
    let path = Path::new(name);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return invalid(format!("archive contains an unsafe path: {name}"));
    }
    Ok(())
}

fn read_record<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
    record: &FileRecord,
    max: u64,
) -> ArchiveResult<Vec<u8>> {
    validate_archive_path(&record.path)?;
    if record.bytes > max {
        return invalid(format!("{} is too large", record.path));
    }
    let bytes = read_named(archive, &record.path, max)?;
    if bytes.len() as u64 != record.bytes {
        return invalid(format!("{} size does not match its manifest", record.path));
    }
    if digest(&bytes) != record.blake3 {
        return invalid(format!("{} failed its integrity check", record.path));
    }
    Ok(bytes)
}

fn read_named<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
    max: u64,
) -> ArchiveResult<Vec<u8>> {
    let mut file = archive.by_name(name)?;
    if file.size() > max {
        return invalid(format!("{name} is too large"));
    }
    let mut bytes = Vec::with_capacity(file.size() as usize);
    file.by_ref().take(max + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max {
        return invalid(format!("{name} expands beyond its size limit"));
    }
    Ok(bytes)
}

fn read_legacy_bundle<R: Read + Seek>(
    mut archive: zip::ZipArchive<R>,
    names: HashSet<String>,
    macros_dir: &Path,
    templates_dir: &Path,
    guards_dir: &Path,
) -> ArchiveResult<Option<String>> {
    if !names.contains("macro.json") {
        return Ok(None);
    }

    std::fs::create_dir_all(templates_dir)?;
    let mut remap = HashMap::new();
    for name in names.iter().filter(|name| name.starts_with("templates/")) {
        let Some(basename) = Path::new(name).file_name().and_then(|part| part.to_str()) else {
            return invalid("legacy bundle contains an invalid template name");
        };
        safe_image_extension(Path::new(basename))?;
        let bytes = read_named(&mut archive, name, MAX_ASSET_BYTES)?;
        let installed = collision_safe_asset_path(templates_dir, basename, &bytes);
        if !installed.exists() {
            crate::util::write_atomic(&installed, &bytes)?;
        }
        remap.insert(basename.to_string(), installed);
    }

    let macro_bytes = read_named(&mut archive, "macro.json", MAX_MACRO_BYTES)?;
    let macro_def: Macro = serde_json::from_slice(&macro_bytes)?;
    let name = install_macro(macro_def, macros_dir)?;

    if names.contains("guards.json") {
        let guard_bytes = read_named(&mut archive, "guards.json", MAX_GUARDS_BYTES)?;
        let mut guards: GuardFile = serde_json::from_slice(&guard_bytes)?;
        for guard in &mut guards.guards {
            let installed = candidate_paths(&guard.template_path, &guard.template_paths)
                .into_iter()
                .filter_map(|path| {
                    Path::new(path)
                        .file_name()
                        .and_then(|part| part.to_str())
                        .and_then(|part| remap.get(part))
                })
                .map(|path| path.to_string_lossy().into_owned())
                .collect();
            set_guard_candidates(guard, installed);
        }
        guards.save_to(&guards_dir.join(format!("{name}.json")))?;
    }
    Ok(Some(name))
}

fn collision_safe_asset_path(dir: &Path, basename: &str, bytes: &[u8]) -> PathBuf {
    let direct = dir.join(basename);
    if !direct.exists() {
        return direct;
    }
    if std::fs::read(&direct).ok().as_deref() == Some(bytes) {
        return direct;
    }
    let source = Path::new(basename);
    let stem = source
        .file_stem()
        .and_then(|part| part.to_str())
        .unwrap_or("image");
    let extension = source
        .extension()
        .and_then(|part| part.to_str())
        .unwrap_or("png");
    dir.join(format!("{stem}_{}.{}", &digest(bytes)[..12], extension))
}

fn safe_image_extension(path: &Path) -> ArchiveResult<String> {
    let extension = path
        .extension()
        .and_then(|part| part.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "png" | "jpg" | "jpeg" | "webp" | "bmp" => Ok(extension),
        _ => invalid("unsupported vision image type"),
    }
}

fn reject_large_archive(path: &Path) -> ArchiveResult<()> {
    if std::fs::metadata(path)?.len() > MAX_ARCHIVE_BYTES {
        return invalid("archive is too large");
    }
    Ok(())
}

fn is_zip(path: &Path) -> ArchiveResult<bool> {
    let mut file = File::open(path)?;
    let mut signature = [0_u8; 4];
    let count = file.read(&mut signature)?;
    Ok(count == 4 && matches!(&signature, b"PK\x03\x04" | b"PK\x05\x06" | b"PK\x07\x08"))
}

fn digest(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

fn invalid<T>(message: impl Into<String>) -> ArchiveResult<T> {
    Err(invalid_error(message))
}

fn invalid_error(message: impl Into<String>) -> Box<dyn std::error::Error> {
    Box::new(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        message.into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::guard::Guard;
    use crate::models::macro_def::{InputEventType, MacroEvent};
    use crate::models::node_graph::{GraphEdge, GraphNode};
    use crate::models::step::Step;
    use crate::test_support::temp_dir;
    use serde_json::{json, Value};

    fn sample_macro(name: &str, events: usize) -> Macro {
        Macro {
            name: name.to_string(),
            events: (0..events)
                .map(|index| MacroEvent {
                    event_type: InputEventType::Wait,
                    timestamp: index as f64 / 100.0,
                    x: 0,
                    y: 0,
                    mouse_motion: None,
                    dx: 0,
                    dy: 0,
                    button: "left".to_string(),
                    key: String::new(),
                    delta: 0,
                    duration: 0.01,
                    checkpoint: None,
                })
                .collect(),
            ..Default::default()
        }
    }

    fn sample_loop(name: &str, direct: &Path, alternative: &Path, embedded: &Path) -> NodeGraph {
        let vision_step = Step {
            id: "vision-step".into(),
            step_type: "find_click".into(),
            detect_mode: "template".into(),
            template: direct.to_string_lossy().into_owned(),
            templates: vec![alternative.to_string_lossy().into_owned()],
            confidence: 0.8,
            ..Default::default()
        };
        let embedded_step = Step {
            id: "embedded-step".into(),
            step_type: "wait_for".into(),
            detect_mode: "template".into(),
            template: embedded.to_string_lossy().into_owned(),
            confidence: 0.8,
            ..Default::default()
        };
        NodeGraph {
            name: name.into(),
            entry: "start".into(),
            nodes: vec![
                GraphNode {
                    id: "start".into(),
                    node_type: "start".into(),
                    label: "Start".into(),
                    ..Default::default()
                },
                GraphNode {
                    id: "vision".into(),
                    node_type: "vision".into(),
                    label: "Find button".into(),
                    config: json!({ "step": vision_step }),
                    ..Default::default()
                },
                GraphNode {
                    id: "macro".into(),
                    node_type: "sub_macro".into(),
                    label: "Embedded macro".into(),
                    config: json!({
                        "macro_name": "Snapshot",
                        "repeat": 1,
                        "embedded_steps": [embedded_step],
                    }),
                    ..Default::default()
                },
            ],
            edges: vec![GraphEdge {
                id: "start-next".into(),
                from: "start".into(),
                output: "next".into(),
                to: "vision".into(),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn zip_entries(path: &Path) -> HashMap<String, Vec<u8>> {
        let mut archive = zip::ZipArchive::new(File::open(path).unwrap()).unwrap();
        let names = archive.file_names().map(str::to_string).collect::<Vec<_>>();
        names
            .into_iter()
            .map(|name| {
                let mut bytes = Vec::new();
                archive
                    .by_name(&name)
                    .unwrap()
                    .read_to_end(&mut bytes)
                    .unwrap();
                (name, bytes)
            })
            .collect()
    }

    fn write_zip_entries(path: &Path, entries: &HashMap<String, Vec<u8>>) {
        let mut zip = zip::ZipWriter::new(File::create(path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        let mut names = entries.keys().cloned().collect::<Vec<_>>();
        names.sort();
        for name in names {
            zip.start_file(&name, options).unwrap();
            zip.write_all(&entries[&name]).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn clawmation_is_compact_checksummed_and_round_trips() {
        let root = temp_dir("clawmation_round_trip");
        let source = root.join("source.json");
        sample_macro("portable", 2_000).save_to(&source).unwrap();
        let exported = root.join("portable.clawmation");

        write_macro(&source, &exported).unwrap();
        assert!(
            std::fs::metadata(&exported).unwrap().len()
                < std::fs::metadata(&source).unwrap().len() / 4,
            "repetitive macro JSON should compress substantially"
        );
        let mut archive = zip::ZipArchive::new(File::open(&exported).unwrap()).unwrap();
        assert_eq!(
            archive.by_name(MACRO_PATH).unwrap().compression(),
            zip::CompressionMethod::Zstd
        );

        let installed = read_macro(&exported, &root.join("imported")).unwrap();
        assert_eq!(installed, "portable");
        let loaded = Macro::load(&root.join("imported/portable.json")).unwrap();
        assert_eq!(loaded.events.len(), 2_000);
    }

    #[test]
    fn bundle_deduplicates_identical_images_and_remaps_every_guard() {
        let root = temp_dir("bundle_dedup");
        let macro_path = root.join("macro.json");
        sample_macro("bundle", 2).save_to(&macro_path).unwrap();
        let first = root.join("first.png");
        let second = root.join("second.png");
        std::fs::write(&first, b"\x89PNG\r\n\x1a\nsame-lossless-bytes").unwrap();
        std::fs::write(&second, b"\x89PNG\r\n\x1a\nsame-lossless-bytes").unwrap();
        let guards = GuardFile {
            guards: vec![
                Guard {
                    template_path: first.to_string_lossy().into_owned(),
                    ..Default::default()
                },
                Guard {
                    template_path: second.to_string_lossy().into_owned(),
                    ..Default::default()
                },
            ],
        };
        let guards_path = root.join("guards.json");
        guards.save_to(&guards_path).unwrap();
        let bundle = root.join("bundle.clawbundle");

        write_bundle(&macro_path, &guards_path, &bundle).unwrap();
        let archive = zip::ZipArchive::new(File::open(&bundle).unwrap()).unwrap();
        let assets = archive
            .file_names()
            .filter(|name| name.starts_with("assets/"))
            .count();
        assert_eq!(assets, 1, "identical images are stored once");

        let imported = read_bundle(
            &bundle,
            &root.join("dst/macros"),
            &root.join("dst/templates"),
            &root.join("dst/macros/guards"),
        )
        .unwrap()
        .unwrap();
        let installed = GuardFile::load(&root.join(format!("dst/macros/guards/{imported}.json")));
        assert_eq!(
            installed.guards[0].template_path,
            installed.guards[1].template_path
        );
        assert!(Path::new(&installed.guards[0].template_path).exists());
    }

    #[test]
    fn bundle_packages_and_remaps_every_guard_image_alternative() {
        let root = temp_dir("bundle_alternatives");
        let macro_path = root.join("macro.json");
        sample_macro("bundle_alternatives", 1)
            .save_to(&macro_path)
            .unwrap();
        let normal = root.join("normal.png");
        let hovered = root.join("hovered.png");
        std::fs::write(&normal, b"\x89PNG\r\n\x1a\nnormal").unwrap();
        std::fs::write(&hovered, b"\x89PNG\r\n\x1a\nhovered").unwrap();
        let guards = GuardFile {
            guards: vec![Guard {
                name: "Open".into(),
                method: "template".into(),
                template_path: normal.to_string_lossy().into_owned(),
                template_paths: vec![hovered.to_string_lossy().into_owned()],
                ..Default::default()
            }],
        };
        let guards_path = root.join("guards.json");
        guards.save_to(&guards_path).unwrap();
        let bundle = root.join("bundle.clawbundle");

        write_bundle(&macro_path, &guards_path, &bundle).unwrap();
        let imported = read_bundle(
            &bundle,
            &root.join("dst/macros"),
            &root.join("dst/templates"),
            &root.join("dst/guards"),
        )
        .unwrap()
        .unwrap();
        let installed = GuardFile::load(&root.join(format!("dst/guards/{imported}.json")));
        let candidates = candidate_paths(
            &installed.guards[0].template_path,
            &installed.guards[0].template_paths,
        );
        assert_eq!(candidates.len(), 2);
        assert!(candidates.iter().all(|path| Path::new(path).is_file()));
    }

    #[test]
    fn loop_bundle_round_trip_includes_direct_and_embedded_images_once() {
        let root = temp_dir("loop_round_trip");
        let normal = root.join("normal.png");
        let hovered = root.join("hovered.png");
        let embedded = root.join("embedded-copy.png");
        std::fs::write(&normal, b"\x89PNG\r\n\x1a\nnormal").unwrap();
        std::fs::write(&hovered, b"\x89PNG\r\n\x1a\nhovered").unwrap();
        std::fs::write(&embedded, b"\x89PNG\r\n\x1a\nhovered").unwrap();
        let graph = sample_loop("Portable Loop", &normal, &hovered, &embedded);
        let graph_path = root.join("loop.json");
        graph.save_to(&graph_path).unwrap();
        let bundle = root.join("loop.clawbundle");

        write_loop(&graph_path, &bundle).unwrap();
        let archive = zip::ZipArchive::new(File::open(&bundle).unwrap()).unwrap();
        assert_eq!(
            archive
                .file_names()
                .filter(|name| name.starts_with("assets/"))
                .count(),
            2,
            "the embedded duplicate should share the hovered asset"
        );

        let imported_name = read_loop(
            &bundle,
            &root.join("dst/nodes"),
            &root.join("dst/templates"),
        )
        .unwrap();
        assert_eq!(imported_name, "Portable Loop");
        let imported = NodeGraph::load(&root.join("dst/nodes/Portable Loop.json")).unwrap();
        assert_eq!(imported.nodes.len(), graph.nodes.len());
        assert_eq!(imported.edges.len(), graph.edges.len());
        let installed = image_paths(&imported);
        assert_eq!(
            installed.len(),
            2,
            "content-identical paths normalize on import"
        );
        assert!(installed.iter().all(|path| Path::new(path).is_file()));
        assert!(installed
            .iter()
            .all(|path| Path::new(path).starts_with(root.join("dst/templates"))));
    }

    #[test]
    fn importing_a_loop_never_overwrites_an_existing_name() {
        let root = temp_dir("loop_collision");
        let image = root.join("image.png");
        std::fs::write(&image, b"\x89PNG\r\n\x1a\nimage").unwrap();
        let graph = sample_loop("Daily", &image, &image, &image);
        let graph_path = root.join("loop.json");
        graph.save_to(&graph_path).unwrap();
        let bundle = root.join("loop.clawbundle");
        write_loop(&graph_path, &bundle).unwrap();
        let nodes = root.join("nodes");
        std::fs::create_dir_all(&nodes).unwrap();
        std::fs::write(nodes.join("Daily.json"), b"keep me").unwrap();

        let imported = read_loop(&bundle, &nodes, &root.join("templates")).unwrap();

        assert_eq!(imported, "Daily 2");
        assert_eq!(std::fs::read(nodes.join("Daily.json")).unwrap(), b"keep me");
        assert!(nodes.join("Daily 2.json").is_file());
    }

    #[test]
    fn loop_export_rejects_a_missing_referenced_image() {
        let root = temp_dir("loop_missing_export");
        let missing = root.join("missing.png");
        let graph = sample_loop("Broken", &missing, &missing, &missing);
        let graph_path = root.join("loop.json");
        graph.save_to(&graph_path).unwrap();

        let error = write_loop(&graph_path, &root.join("broken.clawbundle"))
            .unwrap_err()
            .to_string();

        assert!(error.contains("Loop image is missing"));
        assert!(error.contains("missing.png"));
    }

    #[test]
    fn loop_import_rejects_missing_undeclared_and_corrupt_assets() {
        let root = temp_dir("loop_invalid_assets");
        let image = root.join("image.png");
        std::fs::write(&image, b"\x89PNG\r\n\x1a\nimage").unwrap();
        let graph = sample_loop("Portable", &image, &image, &image);
        let graph_path = root.join("loop.json");
        graph.save_to(&graph_path).unwrap();
        let bundle = root.join("valid.clawbundle");
        write_loop(&graph_path, &bundle).unwrap();
        let valid_entries = zip_entries(&bundle);
        let asset_name = valid_entries
            .keys()
            .find(|name| name.starts_with("assets/"))
            .unwrap()
            .clone();

        let mut missing = valid_entries.clone();
        missing.remove(&asset_name);
        let missing_bundle = root.join("missing.clawbundle");
        write_zip_entries(&missing_bundle, &missing);
        assert!(read_loop(
            &missing_bundle,
            &root.join("missing/nodes"),
            &root.join("missing/templates")
        )
        .unwrap_err()
        .to_string()
        .contains("missing, duplicate, or undeclared"));

        let mut corrupt = valid_entries.clone();
        corrupt.get_mut(&asset_name).unwrap().push(0);
        let corrupt_bundle = root.join("corrupt.clawbundle");
        write_zip_entries(&corrupt_bundle, &corrupt);
        assert!(read_loop(
            &corrupt_bundle,
            &root.join("corrupt/nodes"),
            &root.join("corrupt/templates")
        )
        .unwrap_err()
        .to_string()
        .contains("size does not match"));

        let mut undeclared = valid_entries;
        let mut manifest: Manifest = serde_json::from_slice(&undeclared[MANIFEST_PATH]).unwrap();
        let mut imported_graph: NodeGraph = serde_json::from_slice(&undeclared[LOOP_PATH]).unwrap();
        imported_graph.nodes[1].config["step"]["template"] =
            Value::String("assets/undeclared.png".into());
        let payload = serde_json::to_vec(&imported_graph).unwrap();
        let record = manifest.loop_file.as_mut().unwrap();
        record.bytes = payload.len() as u64;
        record.blake3 = digest(&payload);
        undeclared.insert(LOOP_PATH.into(), payload);
        undeclared.insert(MANIFEST_PATH.into(), serde_json::to_vec(&manifest).unwrap());
        let undeclared_bundle = root.join("undeclared.clawbundle");
        write_zip_entries(&undeclared_bundle, &undeclared);
        assert!(read_loop(
            &undeclared_bundle,
            &root.join("undeclared/nodes"),
            &root.join("undeclared/templates"),
        )
        .unwrap_err()
        .to_string()
        .contains("undeclared image"));
    }

    #[test]
    fn legacy_json_and_legacy_bundle_remain_importable() {
        let root = temp_dir("legacy_transfer");
        let json_path = root.join("old.json");
        sample_macro("old_json", 1).save_to(&json_path).unwrap();
        assert_eq!(
            read_macro(&json_path, &root.join("json_dst")).unwrap(),
            "old_json"
        );

        let bundle = root.join("old.clawbundle");
        {
            let file = File::create(&bundle).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default();
            zip.start_file("macro.json", options).unwrap();
            zip.write_all(&serde_json::to_vec(&sample_macro("old_bundle", 1)).unwrap())
                .unwrap();
            zip.finish().unwrap();
        }
        assert_eq!(
            read_bundle(
                &bundle,
                &root.join("bundle_dst/macros"),
                &root.join("bundle_dst/templates"),
                &root.join("bundle_dst/guards"),
            )
            .unwrap(),
            Some("old_bundle".to_string())
        );
    }

    #[test]
    fn traversal_entries_are_rejected_before_extraction() {
        let root = temp_dir("bundle_traversal");
        let bundle = root.join("bad.clawbundle");
        {
            let file = File::create(&bundle).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            zip.start_file("../outside.png", zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(b"bad").unwrap();
            zip.finish().unwrap();
        }
        let error = read_bundle(&bundle, &root.join("m"), &root.join("t"), &root.join("g"))
            .unwrap_err()
            .to_string();
        assert!(error.contains("unsafe path"));
        assert!(!root.join("outside.png").exists());
    }
}
