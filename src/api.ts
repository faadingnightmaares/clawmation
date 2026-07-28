// Typed wrapper over the Tauri command seam. Every backend command the
// frontend calls goes through a function here so the payload types live in one
// place and match the Rust structs exactly. New slices extend this file.
//
// Two conventions live side by side, and they differ by direction:
//   • Arguments IN use camelCase keys: Tauri v2 auto-maps them onto the Rust
//     command's snake_case params (e.g. `{ macroName }` → `macro_name`).
//   • Data OUT keeps Rust's serde field names, which are snake_case, so the
//     return interfaces below are snake_case. Do not "tidy" a return field to
//     camelCase; it must match the JSON the command actually emits.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/** Arbitrary JSON object, used where a command takes or returns an open map. */
export type Json = Record<string, unknown>;

/**
 * The near-universal command result. Mutating commands return this shape
 * (sometimes with extra fields; see the specific interfaces). The UI reads
 * live state off the `get_status` heartbeat, not these payloads, so
 * fire-and-forget commands are typed as a bare `OpResult`.
 */
export interface OpResult {
  ok: boolean;
  error?: string;
}

// ─── Status heartbeat ───

/** One activity-log line. Mirrors Rust `logbuf::LogEntry`. */
export interface LogEntry {
  t: string;
  level: string;
  msg: string;
}

export interface WindowStatus {
  found: boolean;
  title: string;
  /** `"WxH"` when found, empty otherwise. */
  size: string;
}

export interface CaptureStatus {
  backend: string;
  fps: number;
}

export interface StatusConfig {
  resolution: [number, number];
  capture_backend: string;
  hotkey_record: string;
  hotkey_play: string;
  hotkey_stop: string;
  indicator_on_top: boolean;
}

/** The `get_status` heartbeat payload. Mirrors Rust `models::status::Status`. */
export interface Status {
  /** `"idle"`, `"recording"`, `"playing"`, or `"paused"`. */
  mode: string;
  elapsed: number;
  record_paused: boolean;
  play_iteration: number;
  play_total_reps: number;
  window: WindowStatus;
  capture: CaptureStatus;
  recorded_count: number;
  last_macro: string;
  macro_count: number;
  indicator_alive: boolean;
  config: StatusConfig;
  log: LogEntry[];
}

/** Poll the app's live status (mode, counts, recent log). */
export function getStatus(): Promise<Status> {
  return invoke<Status>("get_status");
}

// ─── Anti-AFK ───

export interface SelectableWindow {
  id: string;
  title: string;
  pid: number;
}

export type AntiAfkAction = "jump" | "walk" | "camera" | "random";

export interface AntiAfkState {
  enabled: boolean;
  target_id: string | null;
  interval_min: number;
  action: AntiAfkAction;
  status: "off" | "active" | "acting" | "target_unavailable" | "error";
  error: string | null;
}

export interface AntiAfkUpdate {
  target_id?: string;
  interval_min?: number;
  action?: AntiAfkAction;
  enabled?: boolean;
}

export interface AntiAfkUpdateResult extends OpResult {
  state?: AntiAfkState;
}

export function antiAfkListWindows(): Promise<SelectableWindow[]> {
  return invoke<SelectableWindow[]>("anti_afk_list_windows");
}

export function antiAfkGet(): Promise<AntiAfkState> {
  return invoke<AntiAfkState>("anti_afk_get");
}

export function antiAfkUpdate(patch: AntiAfkUpdate): Promise<AntiAfkUpdateResult> {
  return invoke<AntiAfkUpdateResult>("anti_afk_update", {
    targetId: patch.target_id,
    intervalMin: patch.interval_min,
    action: patch.action,
    enabled: patch.enabled,
  });
}

// ─── Config & data folders ───

/** `get_config` payload. Mirrors Rust `ConfigDto`. */
export interface ConfigDto {
  capture_backend: string;
  hotkey_record: string;
  hotkey_play: string;
  hotkey_stop: string;
  indicator_on_top: boolean;
  humanize_clicks: boolean;
  notify_on_schedule: boolean;
  notify_on_complete: boolean;
}

export function getConfig(): Promise<ConfigDto> {
  return invoke<ConfigDto>("get_config");
}

/** A config write, plus any shortcut the OS refused to hand over (another app
 *  already owns it). Empty unless the patch touched a `hotkey_*` key. */
export interface ConfigSaveResult extends OpResult {
  unbound?: string[];
}

/** Merge a partial config patch over the stored config; unknown keys round-trip. */
export function updateConfig(patch: Json): Promise<ConfigSaveResult> {
  return invoke<ConfigSaveResult>("update_config", { patch });
}

/** Release the global shortcuts so a keystroke reaches the webview to be
 *  captured; they're exclusive, so an already-bound key would fire instead. */
export function hotkeysSuspend(): Promise<OpResult> {
  return invoke<OpResult>("hotkeys_suspend");
}

/** Re-arm the global shortcuts after a capture ends. */
export function hotkeysResume(): Promise<ConfigSaveResult> {
  return invoke<ConfigSaveResult>("hotkeys_resume");
}

export interface DataPaths {
  ok: boolean;
  root: string;
  macros_dir: string;
  templates_dir: string;
  snapshots_dir: string;
  config_dir: string;
  macro_count: number;
  template_count: number;
  snapshot_count: number;
}

export function getDataPaths(): Promise<DataPaths> {
  return invoke<DataPaths>("get_data_paths");
}

/** Open a data folder in the OS file manager. `kind`: root|macros|templates|snapshots|config. */
export function openDataFolder(kind: string): Promise<OpResult> {
  return invoke<OpResult>("open_data_folder", { kind });
}

// ─── Macros & templates ───

/** One row in the macro list. Mirrors Rust `MacroListItem`. */
export interface MacroListItem {
  name: string;
  events: number;
  duration: number;
  /** `"WxH"`. */
  resolution: string;
  /** Serde-renamed from `loop_enabled`. */
  loop: boolean;
  loop_count: number;
  category: string;
  notes: string;
  play_count: number;
  last_played: number;
  /** Cumulative seconds this macro has been played (0 until the first run). */
  played: number;
}

export function listMacros(): Promise<MacroListItem[]> {
  return invoke<MacroListItem[]>("list_macros");
}

export function deleteMacro(name: string): Promise<OpResult> {
  return invoke<OpResult>("delete_macro", { name });
}

export interface BulkDeleteResult {
  ok: boolean;
  deleted: string[];
  failed: string[];
}

export function bulkDelete(names: string[]): Promise<BulkDeleteResult> {
  return invoke<BulkDeleteResult>("bulk_delete", { names });
}

/** Result of a save that names the created artifact. */
export interface SaveNamedResult {
  ok: boolean;
  name?: string;
  error?: string;
}

export function saveAsTemplate(name: string, templateName: string): Promise<SaveNamedResult> {
  return invoke<SaveNamedResult>("save_as_template", { name, templateName });
}

export interface TemplateItem {
  name: string;
  events: number;
  duration: number;
  category: string;
}

export function listTemplates(): Promise<TemplateItem[]> {
  return invoke<TemplateItem[]>("list_templates");
}

export function createFromTemplate(templateName: string, newName: string): Promise<SaveNamedResult> {
  return invoke<SaveNamedResult>("create_from_template", { templateName, newName });
}

export function deleteTemplate(templateName: string): Promise<OpResult> {
  return invoke<OpResult>("delete_template", { templateName });
}

// ─── Macro editing (rename, duplicate, repeat, category, notes) ───

/** Rename a macro (and move its guard sidecar). Returns the sanitized name. */
export function renameMacro(oldName: string, newName: string): Promise<SaveNamedResult> {
  return invoke<SaveNamedResult>("rename_macro", { oldName, newName });
}

/** Copy a macro to a unique `{name}_copy` name, cloning its guards. */
export function duplicateMacro(name: string): Promise<SaveNamedResult> {
  return invoke<SaveNamedResult>("duplicate_macro", { name });
}

/** Result of `set_repeat`: the persisted loop settings. */
export interface SetRepeatResult {
  ok: boolean;
  loop?: boolean;
  loop_count?: number;
  error?: string;
}

/**
 * Persist a macro's repeat setting. `0` loops forever, `1` plays once, `N`
 * repeats N times. Fire-and-forget from the UI: no toast, no heartbeat read.
 */
export function setRepeat(name: string, repeat: number): Promise<SetRepeatResult> {
  return invoke<SetRepeatResult>("set_repeat", { name, repeat });
}

export interface SetCategoryResult {
  ok: boolean;
  category?: string;
  error?: string;
}

/** Set (or clear, with `""`) a macro's category label. */
export function setCategory(name: string, category: string): Promise<SetCategoryResult> {
  return invoke<SetCategoryResult>("set_category", { name, category });
}

export interface SetNotesResult {
  ok: boolean;
  notes?: string;
  error?: string;
}

/** Set (or clear, with `""`) a macro's notes. */
export function setNotes(name: string, notes: string): Promise<SetNotesResult> {
  return invoke<SetNotesResult>("set_notes", { name, notes });
}

// ─── Recording ───

export function startRecord(): Promise<OpResult> {
  return invoke<OpResult>("start_record");
}

/** `stop_record` reports the saved macro so the UI can toast it and arm rename. */
export interface StopRecordResult {
  ok: boolean;
  name?: string;
  events?: number;
  duration?: number;
  resolution?: string;
  error?: string;
}

export function stopRecord(): Promise<StopRecordResult> {
  return invoke<StopRecordResult>("stop_record");
}

/** `pause_record` returns the new paused flag so the UI can toast the right verb. */
export interface PauseRecordResult {
  ok: boolean;
  paused?: boolean;
  error?: string;
}

export function pauseRecord(): Promise<PauseRecordResult> {
  return invoke<PauseRecordResult>("pause_record");
}

// ─── Playback ───

/**
 * Play a macro. `repeat`: omit for the macro's saved loop settings; `0`/`""`/`"∞"`
 * loop forever; `N` plays N times. `speed` defaults to 1.0.
 */
export function playMacro(
  name: string,
  repeat?: number | string,
  speed?: number,
): Promise<OpResult> {
  return invoke<OpResult>("play_macro", { name, repeat, speed });
}

export function stopPlayback(): Promise<OpResult> {
  return invoke<OpResult>("stop_playback");
}

export interface EmergencyStopResult {
  ok: boolean;
  stopped: string[];
}

/** The global panic button: stops recording, playback, and vision at once. */
export function emergencyStop(): Promise<EmergencyStopResult> {
  return invoke<EmergencyStopResult>("emergency_stop");
}

// ─── Guards ───

/**
 * One guard / vision-trigger definition. The exact field set is pinned when the
 * shared guard editor lands; list views read the display fields below and any
 * extra keys pass through untyped.
 */
export interface Guard {
  [key: string]: unknown;
  name?: string;
  method?: string;
  action?: string;
  enabled?: boolean;
  hsv_low?: number[];
  hsv_high?: number[];
}

export interface GuardListResult {
  ok: boolean;
  guards: Guard[];
  error?: string;
}

export function guardList(macroName: string): Promise<GuardListResult> {
  return invoke<GuardListResult>("guard_list", { macroName });
}

export interface GuardCountsResult {
  ok: boolean;
  counts: Record<string, number>;
}

export function getAllGuardCounts(): Promise<GuardCountsResult> {
  return invoke<GuardCountsResult>("get_all_guard_counts");
}

export function guardSave(macroName: string, guards?: Guard[]): Promise<OpResult> {
  return invoke<OpResult>("guard_save", { macroName, guards });
}

/** The guard editor's Test result: a match summary plus a base64 `preview`
 * (region box, match rings, best-match crosshair). */
export interface GuardTestResult {
  ok: boolean;
  matched?: number;
  found_x?: number;
  found_y?: number;
  confidence?: number;
  message?: string;
  preview?: string;
  /// MIME type of `preview`. Always `image/png`, since every preview is drawn
  /// in Rust.
  preview_mime?: string;
  error?: string;
}

export function guardTest(guard: Guard): Promise<GuardTestResult> {
  return invoke<GuardTestResult>("guard_test", { guard });
}

// ─── Guard-editor pickers (native capture flows) ───
// Interactive screen selections the guard editor drives: sample a colour, drag a
// watch region, snip a button image, or add one from a file. Each resolves once
// the user finishes the on-screen selection, or with `ok: false` on cancel. Ports
// of `Api.guard_pick_color` / `guard_pick_region` / `capture_template` /
// `add_template_image`.

/** A sampled pixel → the HSV tolerance range the guard matches on, plus the
 * picked colour as `#rrggbb` for the confirmation toast. */
export interface PickColorResult {
  ok: boolean;
  hsv_low?: number[];
  hsv_high?: number[];
  hex?: string;
  error?: string;
}

export function guardPickColor(): Promise<PickColorResult> {
  return invoke<PickColorResult>("guard_pick_color");
}

/** A dragged watch region. The backend returns the pixel rect (`x/y/w/h`) AND
 * `region`, the same box as `[x1,y1,x2,y2]` percentages of the captured frame;
 * the editor stores `region` directly (it is computed against the picker's own
 * frame, so it stays correct across monitors and DPI scales). */
export interface PickRegionResult {
  ok: boolean;
  x?: number;
  y?: number;
  w?: number;
  h?: number;
  region?: number[];
  error?: string;
}

export function guardPickRegion(): Promise<PickRegionResult> {
  return invoke<PickRegionResult>("guard_pick_region");
}

/** A snipped button image: the saved template `path`, a base64 PNG `thumb`, and
 * its pixel size for the toast. Shared by `captureTemplate` and `addTemplateImage`. */
export interface CaptureTemplateResult {
  ok: boolean;
  path?: string;
  thumb?: string;
  w?: number;
  h?: number;
  error?: string;
}

export function captureTemplate(): Promise<CaptureTemplateResult> {
  return invoke<CaptureTemplateResult>("capture_template");
}

/** Add an existing image file as a template. Returns `error: "cancelled"` when
 * the file dialog is dismissed, a case the editor silently ignores. */
export function addTemplateImage(): Promise<CaptureTemplateResult> {
  return invoke<CaptureTemplateResult>("add_template_image");
}

export function saveTemplateUpload(dataBase64: string): Promise<CaptureTemplateResult> {
  return invoke<CaptureTemplateResult>("save_template_upload", { dataBase64 });
}

/** A surgical capture: the saved crop `path`, a base64 PNG `thumb` with the drawn
 * strokes rendered on it, the crop size, and the click strokes as crop-relative
 * offsets; `click_lines` is every stroke, `click_line`/`offset` mirror the first
 * for the single-target fields. Returns `error: "cancelled"` when the overlay is
 * dismissed (Esc), which the editor ignores like `addTemplateImage`. */
export interface SurgicalCaptureResult {
  ok: boolean;
  path?: string;
  click_lines?: number[][];
  click_line?: number[];
  offset?: number[];
  w?: number;
  h?: number;
  thumb?: string;
  error?: string;
}

export function surgicalCapture(): Promise<SurgicalCaptureResult> {
  return invoke<SurgicalCaptureResult>("surgical_capture");
}

// ─── Vision (autonomous triggers) ───

export function visionSave(triggers?: Guard[]): Promise<OpResult> {
  return invoke<OpResult>("vision_save", { triggers });
}

export interface VisionLoadResult {
  ok: boolean;
  triggers: Guard[];
  error?: string;
}

export function visionLoad(): Promise<VisionLoadResult> {
  return invoke<VisionLoadResult>("vision_load");
}

export interface VisionStartResult {
  ok: boolean;
  triggers?: number;
  error?: string;
}

export function visionStart(): Promise<VisionStartResult> {
  return invoke<VisionStartResult>("vision_start");
}

export function visionStop(): Promise<OpResult> {
  return invoke<OpResult>("vision_stop");
}

export interface VisionLogEntry {
  kind: string;
  msg: string;
}

export interface VisionStatusResult {
  ok: boolean;
  running: boolean;
  fired: number;
  log: VisionLogEntry[];
}

export function visionStatus(): Promise<VisionStatusResult> {
  return invoke<VisionStatusResult>("vision_status");
}

// ─── Vision checkpoints (per-macro, mid-run detections) ───
// Insert / list / remove `CHECKPOINT` events inside a recorded macro. A
// checkpoint pauses playback to detect an image or colour, then acts (click /
// key / hold-and-follow). Ports of `Api.list_checkpoints` / `add_checkpoint` /
// `remove_checkpoint`.

/** One checkpoint with its event index in the macro. `checkpoint` is the open
 * config map the vision editor builds (mode, method, region, action, …). */
export interface CheckpointItem {
  index: number;
  checkpoint: Json;
}

export interface ListCheckpointsResult {
  ok: boolean;
  checkpoints: CheckpointItem[];
  error?: string;
}

export function listCheckpoints(name: string): Promise<ListCheckpointsResult> {
  return invoke<ListCheckpointsResult>("list_checkpoints", { name });
}

/** `add_checkpoint` reports the settled insertion index (after the source's
 * negative-index → append rewrite); `index: -1` appends at the end. */
export interface AddCheckpointResult {
  ok: boolean;
  index?: number;
  error?: string;
}

export function addCheckpoint(
  name: string,
  index: number,
  checkpoint: Json,
): Promise<AddCheckpointResult> {
  return invoke<AddCheckpointResult>("add_checkpoint", { name, index, checkpoint });
}

export function removeCheckpoint(name: string, index: number): Promise<OpResult> {
  return invoke<OpResult>("remove_checkpoint", { name, index });
}

// ─── AI steps (recorded macro → editable action list) ───
// Convert a recording into a flat, editable list of steps, then edit / test /
// run / save it back as an "AI macro". Rust owns the whole thing: the run loop,
// action execution, wait_for polling, conversion, and load/save. Ports of
// `Api.macro_to_steps` / `steps_save` / `steps_run` / `steps_test`
// (index.html:3211-3339).

/** One AI macro step. Every field is always present (`macroToSteps` emits
 * complete objects and the editor's inserts default to complete objects), so
 * they read as required here, matching the Rust `Step`
 * (src-tauri/src/models/step.rs). */
export interface Step {
  id: string;
  type: string;
  enabled: boolean;
  label: string;
  // Action params
  x: number;
  y: number;
  key: string;
  text: string;
  delay: number;
  scroll_amount: number; // +N = up, -N = down
  // Detection params (find_click / wait_for)
  detect_mode: string; // 'color' | 'template' | 'features'
  hsv_low: number[];
  hsv_high: number[];
  template: string;
  region: number[]; // [x1, y1, x2, y2] percentages
  min_area: number;
  timeout: number; // wait_for
  confidence: number; // template matching
}

export interface MacroToStepsResult {
  ok: boolean;
  steps?: Step[];
  count?: number;
  error?: string;
}

export function macroToSteps(macroName: string): Promise<MacroToStepsResult> {
  return invoke<MacroToStepsResult>("macro_to_steps", { macroName });
}

export function stepsSave(macroName: string, steps: Step[]): Promise<OpResult> {
  return invoke<OpResult>("steps_save", { macroName, steps });
}

export function stepsRun(steps: Step[]): Promise<OpResult> {
  return invoke<OpResult>("steps_run", { steps });
}

/** Dry-run one step's detection against a fresh frame, without clicking. `preview`
 * is a base64 JPEG (no data-URI prefix); the caller prepends it. Ports
 * `Api.steps_test`. */
export interface StepTestResult {
  ok: boolean;
  message?: string;
  found_x?: number;
  found_y?: number;
  matched?: number;
  confidence?: number;
  preview?: string;
  error?: string;
}

export function stepsTest(step: Step): Promise<StepTestResult> {
  return invoke<StepTestResult>("steps_test", { step });
}

// Directed node graphs

export interface GraphPosition {
  x: number;
  y: number;
}

export type GraphNodeType =
  | "start"
  | "action"
  | "vision"
  | "branch"
  | "loop"
  | "sub_macro"
  | "chain"
  | "note"
  | "stop";

export interface GraphNode {
  id: string;
  type: GraphNodeType;
  label: string;
  position: GraphPosition;
  enabled: boolean;
  config: Record<string, unknown>;
}

export interface GraphEdge {
  id: string;
  from: string;
  output: string;
  to: string;
}

export interface NodeGraph {
  version: number;
  name: string;
  entry: string;
  nodes: GraphNode[];
  edges: GraphEdge[];
}

export interface NodeGraphValidation {
  ok: boolean;
  errors: string[];
  warnings: string[];
}

export interface NodeGraphLoadResult extends OpResult {
  graph?: NodeGraph;
  source?: "saved";
}

export interface NodeLoopItem {
  name: string;
  nodes: number;
  valid_file: boolean;
  updated_at?: number;
}

export interface NodeLoopResult extends OpResult {
  name?: string;
  graph?: NodeGraph;
}

export function nodeGraphList(): Promise<NodeLoopItem[]> {
  return invoke<NodeLoopItem[]>("node_graph_list");
}

export function nodeGraphCreate(name = "Loop"): Promise<NodeLoopResult> {
  return invoke<NodeLoopResult>("node_graph_create", { name });
}

export function nodeGraphLoad(loopName: string): Promise<NodeGraphLoadResult> {
  return invoke<NodeGraphLoadResult>("node_graph_load", { loopName });
}

export function nodeGraphRename(oldName: string, newName: string): Promise<NodeLoopResult> {
  return invoke<NodeLoopResult>("node_graph_rename", { oldName, newName });
}

export function nodeGraphDelete(name: string): Promise<OpResult> {
  return invoke<OpResult>("node_graph_delete", { name });
}

export function nodeGraphValidate(graph: NodeGraph): Promise<NodeGraphValidation> {
  return invoke<NodeGraphValidation>("node_graph_validate", { graph });
}

export function nodeGraphSave(loopName: string, graph: NodeGraph): Promise<OpResult> {
  return invoke<OpResult>("node_graph_save", { loopName, graph });
}

export function nodeGraphRun(graph: NodeGraph): Promise<OpResult> {
  return invoke<OpResult>("node_graph_run", { graph });
}

// ─── Schedules ───

/** A saved schedule row. Fields beyond these pass through untyped. */
export interface Schedule {
  [key: string]: unknown;
  id?: string;
  macro_name?: string;
  kind?: string;
  interval_min?: number;
  at_time?: string;
  repeat?: number;
  enabled?: boolean;
}

export function listSchedules(): Promise<Schedule[]> {
  return invoke<Schedule[]>("list_schedules");
}

export interface ScheduleResult {
  ok: boolean;
  schedule?: Schedule;
  error?: string;
}

export function addSchedule(
  macroName: string,
  kind?: string,
  intervalMin?: number,
  atTime?: string,
  repeat?: number,
  enabled?: boolean,
): Promise<ScheduleResult> {
  return invoke<ScheduleResult>("add_schedule", {
    macroName,
    kind,
    intervalMin,
    atTime,
    repeat,
    enabled,
  });
}

export function removeSchedule(scheduleId: string): Promise<OpResult> {
  return invoke<OpResult>("remove_schedule", { scheduleId });
}

export function setScheduleEnabled(scheduleId: string, enabled: boolean): Promise<OpResult> {
  return invoke<OpResult>("set_schedule_enabled", { scheduleId, enabled });
}

export function scheduleChain(
  chainId: string,
  kind?: string,
  intervalMin?: number,
  atTime?: string,
): Promise<ScheduleResult> {
  return invoke<ScheduleResult>("schedule_chain", { chainId, kind, intervalMin, atTime });
}

// ─── Chains ───

/** A saved chain. Fields beyond these pass through untyped. */
export interface Chain {
  [key: string]: unknown;
  id?: string;
  name?: string;
  macro_names?: string[];
  delay_between?: number;
  repeat?: number;
}

export function listChains(): Promise<Chain[]> {
  return invoke<Chain[]>("list_chains");
}

export interface ChainResult {
  ok: boolean;
  chain?: Chain;
  error?: string;
}

export function addChain(
  name: string,
  macroNames: string[],
  delayBetween?: number,
  repeat?: number,
): Promise<ChainResult> {
  return invoke<ChainResult>("add_chain", { name, macroNames, delayBetween, repeat });
}

export function removeChain(chainId: string): Promise<OpResult> {
  return invoke<OpResult>("remove_chain", { chainId });
}

export interface DuplicateChainResult {
  ok: boolean;
  id?: string;
  name?: string;
  error?: string;
}

export function duplicateChain(chainId: string): Promise<DuplicateChainResult> {
  return invoke<DuplicateChainResult>("duplicate_chain", { chainId });
}

export function updateChain(
  chainId: string,
  name?: string,
  macroNames?: string[],
  delayBetween?: number,
  repeat?: number,
): Promise<OpResult> {
  return invoke<OpResult>("update_chain", { chainId, name, macroNames, delayBetween, repeat });
}

/** Delegates to `state::run_chain`; the UI reads progress off `get_running_chain`. */
export function runChain(chainId: string): Promise<Json> {
  return invoke<Json>("run_chain", { chainId });
}

export interface ValidateChainResult {
  ok: boolean;
  missing?: string[];
  total?: number;
  error?: string;
}

export function validateChain(chainId: string): Promise<ValidateChainResult> {
  return invoke<ValidateChainResult>("validate_chain", { chainId });
}

export interface ChainDurationResult {
  ok: boolean;
  duration?: number;
  error?: string;
}

export function getChainDuration(chainId: string): Promise<ChainDurationResult> {
  return invoke<ChainDurationResult>("get_chain_duration", { chainId });
}

export function stopChain(): Promise<OpResult> {
  return invoke<OpResult>("stop_chain");
}

/** `get_running_chain`: `{running:false}` or `{running:true, id, name, ...}`. */
export interface RunningChain {
  [key: string]: unknown;
  running: boolean;
  id?: string;
  name?: string;
  current_index?: number;
  current_macro?: string;
  total_macros?: number;
  iteration?: number;
  total_iterations?: number;
}

export function getRunningChain(): Promise<RunningChain> {
  return invoke<RunningChain>("get_running_chain");
}

// ─── Stats & history ───

export interface StatsSummary {
  total_plays: number;
  macros_played: number;
  most_played: string;
  most_played_count: number;
  total_macros: number;
  total_chains: number;
  total_guards: number;
  total_schedules: number;
}

export function getStatsSummary(): Promise<StatsSummary> {
  return invoke<StatsSummary>("get_stats_summary");
}

export interface HistoryEntry {
  name: string;
  timestamp: number;
  duration: number;
  status: string;
}

export function getRunHistory(limit?: number): Promise<HistoryEntry[]> {
  return invoke<HistoryEntry[]>("get_run_history", { limit });
}

// ─── App version ───

export interface VersionInfo {
  version: string;
}

export function getVersion(): Promise<VersionInfo> {
  return invoke<VersionInfo>("get_version");
}

export interface UpdateInfo {
  update_available: boolean;
  current: string;
  latest: string;
  /** Release notes from the update manifest, when it carries any. */
  notes?: string | null;
}

/** Ask the release endpoint whether a newer signed build exists. Rejects when
 *  the endpoint is unreachable or the manifest doesn't verify; a rejection is
 *  "couldn't check", never "up to date". */
export function checkUpdate(): Promise<UpdateInfo> {
  return invoke<UpdateInfo>("check_update");
}

/** Download the newer build, run the installer, and restart into it. On success
 *  the process exits mid-call, so this promise simply never settles; it only
 *  rejects, with the reason the update could not be applied. */
export function installUpdate(): Promise<void> {
  return invoke<void>("install_update");
}

/** `(bytes downloaded, total bytes)`. `total` is absent when the server sends
 *  no content length. Emitted only while `installUpdate` is running. */
export type UpdateProgress = [number, number | null];

/** Subscribe to download progress. Resolves to the unsubscribe function. */
export function onUpdateProgress(fn: (p: UpdateProgress) => void): Promise<UnlistenFn> {
  return listen<UpdateProgress>("update-progress", (e) => fn(e.payload));
}

/** Fires when the startup check found a newer build, so the window can mention
 *  it without the user having asked. */
export function onUpdateAvailable(fn: (info: UpdateInfo) => void): Promise<UnlistenFn> {
  return listen<UpdateInfo>("update-available", (e) => fn(e.payload));
}

// ─── Import / export / bundles ───
// Native save/open/folder dialogs live inside the Rust command (blocking), so
// each call resolves once the user finishes the OS dialog, or with
// `error: "cancelled"` when it's dismissed. Ports of `Api.export_chain` /
// `import_chain` / `export_macro` / `import_macro` / `bulk_export` /
// `export_bundle` / `import_bundle`.

/** Result of an export that names the written file. */
export interface ExportResult {
  ok: boolean;
  path?: string;
  error?: string;
}

/** Write a chain to a user-chosen `.chain.json` file. */
export function exportChain(chainId: string): Promise<ExportResult> {
  return invoke<ExportResult>("export_chain", { chainId });
}

/** Import a chain from a user-chosen JSON file; it gets a fresh id. */
export interface ImportChainResult {
  ok: boolean;
  id?: string;
  name?: string;
  error?: string;
}

export function importChain(): Promise<ImportChainResult> {
  return invoke<ImportChainResult>("import_chain");
}

/** Copy a macro to a user-chosen `.json` file (for sharing between machines). */
export function exportMacro(name: string): Promise<ExportResult> {
  return invoke<ExportResult>("export_macro", { name });
}

/** Load a macro from a user-chosen `.json` file into the library (clobber-safe). */
export function importMacro(): Promise<SaveNamedResult> {
  return invoke<SaveNamedResult>("import_macro");
}

/** Copy several macros into a user-chosen folder. */
export interface BulkExportResult {
  ok: boolean;
  exported: string[];
  failed: string[];
  dir?: string;
  error?: string;
}

export function bulkExport(names: string[]): Promise<BulkExportResult> {
  return invoke<BulkExportResult>("bulk_export", { names });
}

/** Export a macro + its guards + referenced templates as a `.clawbundle` zip. */
export function exportBundle(name: string): Promise<ExportResult> {
  return invoke<ExportResult>("export_bundle", { name });
}

/** Import a `.clawbundle`: installs the macro, its guards, and templates. */
export function importBundle(): Promise<SaveNamedResult> {
  return invoke<SaveNamedResult>("import_bundle");
}
