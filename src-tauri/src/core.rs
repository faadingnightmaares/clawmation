//! The wiring hub — `Core` — that binds the leaf hardware Arcs to the permanent
//! guard engine and carries the playback logic `Api` kept as loose methods.
//!
//! `Core` is the Rust analogue of the mutable state `Api.__init__` builds: the
//! config, log buffer, runtime status, play stats, the reused input controller
//! and macro player, the (lazily spawned) vision sidecar, and one permanent
//! [`GuardEngine`]. It is cheaply `Clone` (every field is an `Arc`) so the
//! chains, scheduler, and playback watcher threads can each hold their own
//! handle.
//!
//! Acyclicity is deliberate: the guard engine's four callbacks capture only
//! *leaf* Arcs (vision, player, controller, log) — never `Core` — so guards can
//! drive the same player a macro plays through without forming a reference
//! cycle. The chains/scheduler callbacks (built in `state.rs`) capture `Core`
//! clones, keeping the whole graph a DAG rooted at `AppState`.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::engine::guards::{Action, Actuate, Detect, GuardEngine, OnFire, PlayerState};
use crate::engine::stats::PlayStats;
use crate::hardware::input::InputController;
use crate::hardware::player::{CheckpointDetect, MacroPlayer};
use crate::hardware::recorder::MacroRecorder;
use crate::hardware::vision::{Detection, VisionClient, VisionError};
use crate::logbuf::LogBuffer;
use crate::models::config::MacroConfig;
use crate::models::guard::{Guard, GuardFile};
use crate::models::macro_def::Macro;
use crate::models::step::Step;
use crate::notify::Notifier;
use crate::paths;
use crate::util::{py_float, round1};

/// Live status the UI heartbeat reads. `mode` is one of
/// `idle | recording | playing | paused`.
pub struct Runtime {
    pub mode: String,
    /// When the current recording/playing run started; drives `elapsed` and the
    /// completion-duration stat.
    pub mode_since: Option<Instant>,
    pub last_macro: String,
    pub recorded_count: i64,
    pub indicator_alive: bool,
}

impl Default for Runtime {
    fn default() -> Self {
        Self {
            mode: "idle".to_string(),
            mode_since: None,
            last_macro: String::new(),
            recorded_count: 0,
            indicator_alive: false,
        }
    }
}

/// The vision sidecar, lazily spawned. Wraps the low-level [`VisionClient`] with
/// the app-level concerns the monolith kept in `_get_capture`: spawn-once,
/// remember the *resolved* capture backend, and cache the cosmetic FPS off the
/// status heartbeat's hot path (the client's io lock must never be taken from a
/// heartbeat — see `hardware::vision`).
pub struct Vision {
    client: Mutex<Option<VisionClient>>,
    resolved: Mutex<Option<String>>,
    fps: Mutex<f64>,
    log: Arc<Mutex<LogBuffer>>,
}

impl Vision {
    fn new(log: Arc<Mutex<LogBuffer>>) -> Self {
        Self {
            client: Mutex::new(None),
            resolved: Mutex::new(None),
            fps: Mutex::new(0.0),
            log,
        }
    }

    /// Spawn and `init` the sidecar if it is not already running, then log
    /// `Capture ready (<resolved backend>)` exactly once — the Rust seat of
    /// Python's lazy `_get_capture`. Idempotent while the client is alive.
    pub fn ensure_spawned(
        &self,
        screen_w: i64,
        screen_h: i64,
        backend: &str,
    ) -> Result<(), VisionError> {
        let mut slot = self.client.lock().unwrap();
        if slot.is_some() {
            return Ok(());
        }
        let client = spawn_sidecar()?;
        let resolved = client.init(screen_w, screen_h, backend)?;
        *self.resolved.lock().unwrap() = Some(resolved.clone());
        *slot = Some(client);
        drop(slot);
        if let Ok(mut log) = self.log.lock() {
            log.push("ok", format!("Capture ready ({resolved})"));
        }
        Ok(())
    }

    /// One grab, every guard — the guard poll loop's detect callback. A dead
    /// sidecar (`Closed`) drops the client so the next start respawns it; any
    /// other error yields an empty map so a single bad cycle never aborts the
    /// run. Refreshes the cached FPS while a live client is in hand.
    pub fn detect_guards_faithful(&self, guards: &[Guard]) -> HashMap<String, Vec<Detection>> {
        let result = {
            let slot = self.client.lock().unwrap();
            match slot.as_ref() {
                Some(client) => {
                    let r = client.detect_guards(guards);
                    if r.is_ok() {
                        if let Ok(fps) = client.capture_fps() {
                            *self.fps.lock().unwrap() = fps;
                        }
                    }
                    r
                }
                None => return HashMap::new(),
            }
        };
        match result {
            Ok(map) => map,
            Err(VisionError::Closed) => {
                *self.client.lock().unwrap() = None;
                HashMap::new()
            }
            Err(_) => HashMap::new(),
        }
    }

    /// One poll of a running checkpoint — the play loop's per-poll detect
    /// callback. `play_macro` eager-spawns the sidecar before playback, so the
    /// client is already live here; this only calls, never spawns. A dead sidecar
    /// (`Closed`) drops the client and yields no detections (the checkpoint then
    /// times out) — respawning mid-play would emit a second `Capture ready` the
    /// in-process Python detector never produced. Unlike `detect_guards_faithful`
    /// this does not refresh the cached FPS: `_run_checkpoint` never touched it.
    pub fn detect_checkpoint(&self, cfg: &Value) -> Vec<Detection> {
        let result = {
            let slot = self.client.lock().unwrap();
            match slot.as_ref() {
                Some(client) => client.detect_checkpoint(cfg),
                None => return Vec::new(),
            }
        };
        match result {
            Ok(dets) => dets,
            Err(VisionError::Closed) => {
                *self.client.lock().unwrap() = None;
                Vec::new()
            }
            Err(_) => Vec::new(),
        }
    }

    /// One AI-step detection against a fresh frame — the executor's `find_click`/
    /// `wait_for` detect callback, live-client-only like [`detect_checkpoint`](
    /// Self::detect_checkpoint) (`steps_run` spawns the sidecar before the run
    /// starts). Returns the matches and the sidecar's message. A dead sidecar
    /// (`Closed`) drops the client and yields no matches; any other error yields
    /// `(none, "")`, degrading a `find_click` to "nothing found" — Python instead
    /// crashed the run thread on a `None` frame, leaking the mode, so this is the
    /// cleaner resolution of the same capture-failure case.
    pub fn ai_detect(&self, step: &Value) -> (Vec<crate::engine::ai::Match>, String) {
        let result = {
            let slot = self.client.lock().unwrap();
            match slot.as_ref() {
                Some(client) => client.ai_detect(step),
                None => return (Vec::new(), String::new()),
            }
        };
        match result {
            Ok(v) => parse_ai_detect(&v),
            Err(VisionError::Closed) => {
                *self.client.lock().unwrap() = None;
                (Vec::new(), String::new())
            }
            Err(_) => (Vec::new(), String::new()),
        }
    }

    /// Bring the sidecar up (spawn + `init`, logging `Capture ready` once), then
    /// run one call against the live client, dropping the client on `Closed` so the
    /// next use respawns. The shared shape behind the editor's Test button and its
    /// pickers: Python reached the capture/detector through the lazy
    /// `_get_capture`/`_get_detector`, so any of these cold spawns and `init`s
    /// exactly as it did. `ensure_spawned` just left a live client; a `None` here
    /// means it was torn down between the two locks — treat that as `Closed` so we
    /// respawn. Other errors surface for the command to report.
    fn call_spawned(
        &self,
        screen_w: i64,
        screen_h: i64,
        backend: &str,
        call: impl FnOnce(&VisionClient) -> Result<Value, VisionError>,
    ) -> Result<Value, VisionError> {
        self.ensure_spawned(screen_w, screen_h, backend)?;
        let result = {
            let slot = self.client.lock().unwrap();
            match slot.as_ref() {
                Some(client) => call(client),
                None => Err(VisionError::Closed),
            }
        };
        if matches!(result, Err(VisionError::Closed)) {
            *self.client.lock().unwrap() = None;
        }
        result
    }

    /// Dry-run one guard for the editor's Test button. Returns the sidecar's raw
    /// result dict.
    pub fn guard_test(
        &self,
        screen_w: i64,
        screen_h: i64,
        backend: &str,
        guard: &Guard,
    ) -> Result<Value, VisionError> {
        self.call_spawned(screen_w, screen_h, backend, |c| c.guard_test(guard))
    }

    /// Dry-run one AI step for the step editor's Test button — the cold-spawn seat
    /// of `Api.ai_test_step`. Returns the sidecar's raw result dict.
    pub fn ai_test_step(
        &self,
        screen_w: i64,
        screen_h: i64,
        backend: &str,
        step: &Value,
    ) -> Result<Value, VisionError> {
        self.call_spawned(screen_w, screen_h, backend, |c| c.ai_test_step(step))
    }

    /// Open the guard editor's colour-sampler overlay. Blocks in the sidecar until
    /// the user clicks a pixel or cancels; returns the picker dict verbatim.
    pub fn guard_pick_color(
        &self,
        screen_w: i64,
        screen_h: i64,
        backend: &str,
    ) -> Result<Value, VisionError> {
        self.call_spawned(screen_w, screen_h, backend, |c| c.guard_pick_color())
    }

    /// Open the guard editor's region-drag overlay. Returns `{ok, x, y, w, h}` in
    /// screen pixels, or the cancellation dict.
    pub fn guard_pick_region(
        &self,
        screen_w: i64,
        screen_h: i64,
        backend: &str,
    ) -> Result<Value, VisionError> {
        self.call_spawned(screen_w, screen_h, backend, |c| c.guard_pick_region())
    }

    /// Capture a button crop into `templates_dir` as a template PNG. The directory
    /// is the Rust side's `paths::templates_dir()`, passed by value.
    pub fn capture_template(
        &self,
        screen_w: i64,
        screen_h: i64,
        backend: &str,
        templates_dir: &str,
    ) -> Result<Value, VisionError> {
        self.call_spawned(screen_w, screen_h, backend, |c| c.capture_template(templates_dir))
    }

    /// Import an image file into `templates_dir` as a template PNG.
    pub fn add_template_image(
        &self,
        screen_w: i64,
        screen_h: i64,
        backend: &str,
        templates_dir: &str,
    ) -> Result<Value, VisionError> {
        self.call_spawned(screen_w, screen_h, backend, |c| c.add_template_image(templates_dir))
    }

    /// Two-phase surgical capture into `templates_dir`. Same spawn/backend seam as
    /// [`capture_template`](Self::capture_template).
    pub fn surgical_capture(
        &self,
        screen_w: i64,
        screen_h: i64,
        backend: &str,
        templates_dir: &str,
    ) -> Result<Value, VisionError> {
        self.call_spawned(screen_w, screen_h, backend, |c| c.surgical_capture(templates_dir))
    }

    /// The backend the sidecar actually opened (`None` until first spawn).
    /// Mirrors `_capture.backend if _capture else None`.
    pub fn resolved_backend(&self) -> Option<String> {
        self.resolved.lock().unwrap().clone()
    }

    /// The last cached capture FPS — a pure read, safe from the heartbeat.
    pub fn capture_fps_cached(&self) -> f64 {
        *self.fps.lock().unwrap()
    }
}

/// Resolve and launch the sidecar command: the repo `.venv` interpreter running
/// the module in dev, the frozen `clawmation_vision.exe` beside the app in a
/// bundled build.
fn spawn_sidecar() -> Result<VisionClient, VisionError> {
    if cfg!(debug_assertions) {
        let root = paths::root();
        let python = root
            .parent()
            .unwrap_or_else(|| root.as_path())
            .join(".venv")
            .join("Scripts")
            .join("python.exe");
        let cwd = root.join("sidecar");
        VisionClient::spawn(
            python.to_str().expect("python path is valid UTF-8"),
            &["-m", "clawmation_vision.server"],
            Some(&cwd),
        )
        .map_err(VisionError::Io)
    } else {
        let exe = std::env::current_exe().map_err(VisionError::Io)?;
        let dir = exe
            .parent()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let bin = dir.join("clawmation_vision.exe");
        VisionClient::spawn(bin.to_str().expect("sidecar path is valid UTF-8"), &[], Some(&dir))
            .map_err(VisionError::Io)
    }
}

/// The wiring hub. Clone is cheap — every field is an `Arc`.
#[derive(Clone)]
pub struct Core {
    pub config: Arc<Mutex<MacroConfig>>,
    pub log: Arc<Mutex<LogBuffer>>,
    pub runtime: Arc<Mutex<Runtime>>,
    /// Per-macro play counts and execution history (`config/stats.json`).
    pub play_stats: Arc<PlayStats>,
    /// Reused across plays (Python recreates a `MacroPlayer` each time; one
    /// persistent player is equivalent and lets guards hold a stable handle).
    pub player: Arc<MacroPlayer>,
    pub controller: Arc<InputController>,
    /// `Some` only while recording.
    pub recorder: Arc<Mutex<Option<MacroRecorder>>>,
    pub vision: Arc<Vision>,
    /// One permanent guard engine (Python recreates it per `_start_guards`).
    pub guard_engine: Arc<GuardEngine>,
    /// Tray toasts (playback-complete / scheduled runs); its app handle is bound
    /// in `setup()`, so toasts fired before the UI is up harmlessly no-op.
    pub notifier: Arc<Notifier>,
    /// The transparent pixel-cat recording overlay; its app handle is bound in
    /// `setup()` too, so a `sync` before the window exists no-ops. Driven by
    /// `set_mode`, mirroring Python's `_sync_indicator`.
    pub indicator: Arc<crate::shell::indicator::Indicator>,
}

impl Core {
    pub fn new(config: MacroConfig) -> Self {
        let config = Arc::new(Mutex::new(config));
        let log = Arc::new(Mutex::new(LogBuffer::default()));
        let runtime = Arc::new(Mutex::new(Runtime::default()));
        let play_stats = Arc::new(PlayStats::new(paths::config_dir().join("stats.json")));
        let player = Arc::new(MacroPlayer::new());
        let controller = Arc::new(InputController::new());
        let recorder = Arc::new(Mutex::new(None));
        let vision = Arc::new(Vision::new(log.clone()));
        let notifier = Arc::new(Notifier::new());
        let indicator = Arc::new(crate::shell::indicator::Indicator::new());

        // Guard-engine callbacks capture leaf Arcs only (never `Core`) so a guard
        // pause/resume/click drives the same player and controller a macro plays
        // through, with no reference cycle.
        let detect: Detect = {
            let vision = vision.clone();
            Box::new(move |guards: &[Guard]| vision.detect_guards_faithful(guards))
        };
        let player_state: PlayerState = {
            let player = player.clone();
            Box::new(move || (player.is_playing(), player.is_paused()))
        };
        let actuate: Actuate = {
            let player = player.clone();
            let controller = controller.clone();
            Box::new(move |action| execute_action(&player, &controller, action))
        };
        let on_fire: OnFire = {
            let log = log.clone();
            Box::new(move |g: &Guard, _x: i64, _y: i64| {
                if let Ok(mut log) = log.lock() {
                    log.push(
                        "warn",
                        format!("Guard '{}' fired \u{2014} handling, then resuming", g.name),
                    );
                }
            })
        };
        let guard_engine = Arc::new(GuardEngine::new(detect, player_state, actuate, Some(on_fire)));

        Self {
            config,
            log,
            runtime,
            play_stats,
            player,
            controller,
            recorder,
            vision,
            guard_engine,
            notifier,
            indicator,
        }
    }

    /// Append a log entry (mirrors `Api._emit`; the UI pulls the tail via
    /// `get_status`).
    pub fn emit(&self, level: &str, msg: impl Into<String>) {
        if let Ok(mut log) = self.log.lock() {
            log.push(level, msg);
        }
    }

    /// Record the mode and the instant it began, sync the recording-indicator
    /// overlay, then fire the completion toast — `Api._set_mode`. Every mode
    /// transition funnels through here, so any playing→idle edge (natural finish,
    /// manual stop, emergency stop, or a step run ending) shows "Playback
    /// Complete" when `notify_on_complete` is set, exactly as the source does, and
    /// the pixel-cat shows/hides on the same edges via `_sync_indicator`.
    pub fn set_mode(&self, mode: &str) {
        let (prev_mode, last_macro) = {
            let mut rt = self.runtime.lock().unwrap();
            let prev = rt.mode.clone();
            rt.mode = mode.to_string();
            rt.mode_since = Some(Instant::now());
            (prev, rt.last_macro.clone())
        };
        // Show/hide the pixel-cat overlay — `_set_mode`'s `_sync_indicator()`, right
        // after the mode flips and before the completion toast.
        self.indicator.sync(mode);
        if prev_mode == "playing"
            && mode == "idle"
            && self.config.lock().unwrap().notify_on_complete
        {
            let name = if last_macro.is_empty() { "macro" } else { &last_macro };
            self.notifier.notify(
                "Clawmation \u{2014} Playback Complete",
                &format!("'{name}' finished"),
            );
        }
    }

    // ── Global hotkeys (TinyTask-style) ──────────────────────────────────────

    /// Toggle recording from the global hotkey — `Api.hotkey_record`. (Python
    /// wraps this in a `try/except`, but that only guards the `keyboard` hook
    /// against a thrown exception; `start_record`/`stop_record` return values
    /// here rather than raising, so the error branch is unreachable and unported.)
    pub fn hotkey_record(&self) {
        let mode = self.runtime.lock().unwrap().mode.clone();
        if mode == "recording" {
            self.stop_record();
        } else if mode == "idle" {
            self.start_record();
        } else {
            self.emit("warn", format!("Can't record while {mode}"));
        }
    }

    /// Play the most recent macro from the global hotkey — `Api.hotkey_play`.
    /// Uses the last-played macro, falling back to the newest on disk, and plays
    /// it with its saved loop settings (`repeat=None`).
    pub fn hotkey_play(&self) {
        let mode = self.runtime.lock().unwrap().mode.clone();
        if mode != "idle" {
            self.emit("warn", format!("Can't play while {mode}"));
            return;
        }
        let last = self.runtime.lock().unwrap().last_macro.clone();
        let name = if last.is_empty() {
            self.most_recent_macro_name()
        } else {
            last
        };
        if name.is_empty() {
            self.emit("warn", "No macro to play - record one first");
            return;
        }
        self.play_macro(&name, None, 1.0);
    }

    /// Stem of the newest `*.json` in the macros dir — `Api._most_recent_macro_name`.
    fn most_recent_macro_name(&self) -> String {
        let mut files: Vec<(PathBuf, SystemTime)> = match std::fs::read_dir(paths::macros_dir()) {
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
            .first()
            .and_then(|(p, _)| p.file_stem())
            .and_then(|s| s.to_str())
            .map(String::from)
            .unwrap_or_default()
    }

    /// Primary screen size, falling back to the configured resolution when Win32
    /// reports nothing — `Api._get_screen_resolution`.
    pub fn resolve_screen(&self) -> (u32, u32) {
        let (w, h) = crate::hardware::screen_size();
        if w > 0 && h > 0 {
            (w, h)
        } else {
            self.config.lock().unwrap().resolution
        }
    }

    /// Keys held out of the recording — the record/play/stop hotkeys, lowercased
    /// (`Api._hotkey_ignore_set`), so pressing a hotkey to stop isn't captured.
    fn hotkey_ignore_set(&self) -> HashSet<String> {
        let c = self.config.lock().unwrap();
        [
            c.hotkey_record.to_lowercase(),
            c.hotkey_play.to_lowercase(),
            c.hotkey_stop.to_lowercase(),
        ]
        .into_iter()
        .collect()
    }

    /// Begin recording — `Api.start_record`. Refuses unless idle, then arms a
    /// fresh recorder at the live screen resolution.
    pub fn start_record(&self) -> Value {
        {
            let mode = self.runtime.lock().unwrap().mode.clone();
            if mode != "idle" {
                return json!({ "ok": false, "error": format!("Busy ({mode})") });
            }
        }
        let resolution = self.resolve_screen();
        let ignore = self.hotkey_ignore_set();
        {
            let mut slot = self.recorder.lock().unwrap();
            let mut recorder = MacroRecorder::new(resolution, ignore);
            recorder.start();
            *slot = Some(recorder);
        }
        self.set_mode("recording");
        let hk = self.config.lock().unwrap().hotkey_record.to_uppercase();
        self.emit("rec", format!("Recording started \u{2014} press {hk} or Stop to finish"));
        json!({ "ok": true })
    }

    /// Stop recording, save the macro as `macro_<unix>`, and report it —
    /// `Api.stop_record`.
    pub fn stop_record(&self) -> Value {
        let mode = self.runtime.lock().unwrap().mode.clone();
        let mut slot = self.recorder.lock().unwrap();
        if !matches!(mode.as_str(), "recording" | "paused") || slot.is_none() {
            return json!({ "ok": false, "error": "Not recording" });
        }
        let mut macro_def = slot.as_mut().unwrap().stop();
        drop(slot);

        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        macro_def.name = format!("macro_{stamp}");
        let path = paths::macros_dir().join(format!("{}.json", macro_def.name));
        if let Err(e) = macro_def.save_to(&path) {
            // `Api.stop_record` calls `macro.save()` unguarded: a failure raises
            // before the mode resets, so we surface it and stay in `recording`.
            return json!({ "ok": false, "error": e.to_string() });
        }

        let events = macro_def.events.len();
        {
            let mut rt = self.runtime.lock().unwrap();
            rt.last_macro = macro_def.name.clone();
            rt.recorded_count = events as i64;
        }
        self.set_mode("idle");
        let (w, h) = macro_def.record_resolution;
        self.emit(
            "ok",
            format!("Saved {} ({events} events, {:.1}s)", macro_def.name, macro_def.duration()),
        );
        json!({
            "ok": true,
            "name": macro_def.name,
            "events": events,
            "duration": round1(macro_def.duration()),
            "resolution": format!("{w}x{h}"),
        })
    }

    /// Toggle pause during recording — `Api.pause_record`. Paused time is excluded
    /// from event timestamps by the recorder, so playback stays seamless.
    pub fn pause_record(&self) -> Value {
        let mode = self.runtime.lock().unwrap().mode.clone();
        let mut slot = self.recorder.lock().unwrap();
        if !matches!(mode.as_str(), "recording" | "paused") || slot.is_none() {
            return json!({ "ok": false, "error": "Not recording" });
        }
        let recorder = slot.as_mut().unwrap();
        recorder.toggle_pause();
        let paused = recorder.is_paused();
        drop(slot);
        self.set_mode(if paused { "paused" } else { "recording" });
        self.emit("info", if paused { "Recording paused" } else { "Recording resumed" });
        json!({ "ok": true, "paused": paused })
    }

    /// Play a macro — `Api.play_macro`. `repeat=None` uses the macro's saved loop
    /// settings; `0`/`""`/`∞` loop forever; `N` plays N times. Returns the same
    /// `{ok, name, events}` / `{ok:false, error}` shapes as the source.
    pub fn play_macro(&self, name: &str, repeat: Option<Value>, speed: f64) -> Value {
        {
            let mode = self.runtime.lock().unwrap().mode.clone();
            if mode != "idle" {
                return json!({ "ok": false, "error": format!("Busy ({mode})") });
            }
        }
        let stem = name.strip_suffix(".json").unwrap_or(name);
        let path = paths::macros_dir().join(format!("{stem}.json"));
        if !path.exists() {
            return json!({ "ok": false, "error": format!("Not found: {stem}") });
        }
        let mut macro_def = match Macro::load(&path) {
            Ok(m) => m,
            Err(e) => {
                self.emit("err", format!("Failed to load {stem}: {e}"));
                return json!({ "ok": false, "error": e.to_string() });
            }
        };
        if macro_def.events.is_empty() {
            return json!({ "ok": false, "error": "Macro has no events" });
        }

        let repeat = resolve_repeat(repeat.as_ref(), &macro_def);
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

        let macro_name = macro_def.name.clone();
        let events = macro_def.events.len();

        self.runtime.lock().unwrap().last_macro = macro_name.clone();
        // Recorded at play-start so a stopped macro still counts; the duration and
        // final status are filled in on completion by `watch_playback`.
        self.play_stats.record_play(&macro_name, 0.0, "running");

        // Eager capture spawn — Python's `cap = self._get_capture()` runs before
        // the mode flip and the "Playing" log, warming the sidecar (and logging
        // "Capture ready") so a live detector backs any checkpoints. Best-effort:
        // the sidecar is a subprocess with a spawn-failure mode Python's in-process
        // capture never had, so a failure here degrades checkpoints to their
        // timeout rather than aborting playback — a plain macro must still play.
        // start_guards' own ensure_spawned then no-ops against the live client.
        let target = self.resolve_screen();
        let backend = self.config.lock().unwrap().capture_backend.clone();
        let _ = self.vision.ensure_spawned(target.0 as i64, target.1 as i64, &backend);

        self.set_mode("playing");
        let repeat_msg = if repeat == 0 { "inf".to_string() } else { repeat.to_string() };
        let speed_msg = if speed != 1.0 { format!("{}x", py_float(speed)) } else { "1x".to_string() };
        self.emit(
            "play",
            format!(
                "Playing {macro_name} ({events} events, repeat: {repeat_msg}, speed: {speed_msg}, target: {}x{})",
                target.0, target.1
            ),
        );

        // play_macro wires a live detector — Python builds `MacroPlayer` with a
        // detector + frame_provider here, so checkpoints RUN. The vision-agent
        // runner builds a bare player and passes None, skipping them.
        let checkpoint: CheckpointDetect = {
            let vision = self.vision.clone();
            Box::new(move |cfg: &Value| vision.detect_checkpoint(cfg))
        };
        self.player.play(macro_def, target, speed, Some(checkpoint));
        self.start_guards(&macro_name);

        let core = self.clone();
        thread::spawn(move || core.watch_playback());

        json!({ "ok": true, "name": macro_name, "events": events })
    }

    /// Attach and start the guard engine for a macro that has enabled guards —
    /// `Api._start_guards`. Spawns the sidecar first (logging `Capture ready`),
    /// then `N guard(s) active during playback`.
    fn start_guards(&self, macro_name: &str) {
        let path = paths::guards_dir().join(format!("{macro_name}.json"));
        let enabled: Vec<Guard> = GuardFile::load(&path)
            .guards
            .into_iter()
            .filter(|g| g.enabled)
            .collect();
        if enabled.is_empty() {
            return;
        }
        let (w, h) = self.resolve_screen();
        let backend = self.config.lock().unwrap().capture_backend.clone();
        if let Err(e) = self.vision.ensure_spawned(w as i64, h as i64, &backend) {
            self.emit("err", format!("Guards failed to start: {e}"));
            return;
        }
        let humanize = self.config.lock().unwrap().humanize_clicks;
        let count = enabled.len();
        self.guard_engine.set_humanize(humanize);
        self.guard_engine.start(enabled);
        self.emit("ok", format!("{count} guard(s) active during playback"));
    }

    fn stop_guards(&self) {
        self.guard_engine.stop();
    }

    /// Wait out the playback thread, then reset to idle and record completion —
    /// `Api._watch_playback`. The `mode == "playing"` guard means a concurrent
    /// `stop_playback` (which sets idle first) wins, so exactly one of
    /// `Playback finished` / `Playback stopped` is logged.
    fn watch_playback(&self) {
        self.player.wait();
        self.stop_guards();
        let (playing, duration, last) = {
            let rt = self.runtime.lock().unwrap();
            let playing = rt.mode == "playing";
            let duration = rt.mode_since.map(|s| s.elapsed().as_secs_f64()).unwrap_or(0.0);
            let last = if rt.last_macro.is_empty() {
                "unknown".to_string()
            } else {
                rt.last_macro.clone()
            };
            (playing, duration, last)
        };
        if playing {
            self.play_stats.update_last_run(&last, duration, "completed");
            self.set_mode("idle");
            self.emit("ok", "Playback finished");
        }
    }

    /// Stop the current playback — `Api.stop_playback`.
    pub fn stop_playback(&self) -> Value {
        let playing = self.runtime.lock().unwrap().mode == "playing";
        if playing {
            self.stop_guards();
            self.player.stop();
            self.set_mode("idle");
            self.emit("warn", "Playback stopped");
            json!({ "ok": true })
        } else {
            json!({ "ok": false, "error": "Not playing" })
        }
    }

    /// Run an edited step list in a background thread — `Api.steps_run`. The
    /// frontend's Run button passes only the steps (never loop settings), so this
    /// always plays them once. Refuses unless idle, then flips to `playing` and
    /// drives the executor off-thread. Returns `{ok:true}` immediately (the
    /// pass/fail summary arrives later as a log line), or the busy / bad-steps
    /// error.
    pub fn steps_run(&self, steps: Vec<Value>) -> Value {
        {
            let mode = self.runtime.lock().unwrap().mode.clone();
            if mode != "idle" {
                return json!({ "ok": false, "error": format!("Busy ({mode})") });
            }
        }
        let parsed: Result<Vec<Step>, _> = steps.into_iter().map(serde_json::from_value).collect();
        let step_objs = match parsed {
            Ok(objs) => objs,
            Err(e) => return json!({ "ok": false, "error": format!("Bad steps: {e}") }),
        };

        self.set_mode("playing");
        let count = step_objs.len();
        self.emit("play", format!("Running {count} steps"));

        let core = self.clone();
        thread::spawn(move || {
            // Python spawns the capture *inside* this thread (after the "Running N
            // steps" emit), unlike play_macro's eager pre-flip spawn — so any
            // "Capture ready" log lands after "Running N steps". Best-effort: a
            // spawn failure leaves `ai_detect` returning no matches, degrading each
            // find_click/wait_for to a clean failure rather than aborting, so the
            // run still completes and never leaks the mode.
            let (w, h) = core.resolve_screen();
            let backend = core.config.lock().unwrap().capture_backend.clone();
            let _ = core.vision.ensure_spawned(w as i64, h as i64, &backend);

            let detect: crate::engine::ai::Detect = {
                let vision = core.vision.clone();
                Box::new(move |step: &Step| {
                    let payload = serde_json::to_value(step).unwrap_or(Value::Null);
                    vision.ai_detect(&payload)
                })
            };
            let actuate: crate::engine::ai::Actuate = {
                let controller = core.controller.clone();
                Box::new(move |action| execute_ai_action(&controller, action))
            };

            let summary = crate::engine::ai::run(&step_objs, false, 1, &detect, &actuate);
            core.set_mode("idle");
            let ok = summary["ok"].as_bool().unwrap_or(false);
            let status = if ok { "finished" } else { "stopped (step failed)" };
            let passed = summary["steps_passed"].as_i64().unwrap_or(0);
            let run = summary["steps_run"].as_i64().unwrap_or(0);
            core.emit(
                if ok { "ok" } else { "warn" },
                format!("Steps {status}: {passed}/{run} passed"),
            );
        });

        json!({ "ok": true })
    }

    /// Dry-run one step against a fresh frame for the editor's Test button —
    /// `Api.steps_test` (which delegates to `ai_test_step`). Returns the sidecar's
    /// result dict, or `{ok:false, error}` if the sidecar can't be reached.
    pub fn steps_test(&self, step: Value) -> Value {
        let (w, h) = self.resolve_screen();
        let backend = self.config.lock().unwrap().capture_backend.clone();
        match self.vision.ai_test_step(w as i64, h as i64, &backend, &step) {
            Ok(v) => v,
            Err(e) => json!({ "ok": false, "error": e.to_string() }),
        }
    }
}

/// Apply one guard [`Action`] to the shared player/controller. Each arm is a
/// single atomic hardware call, matching `GuardEngine._execute_action`.
fn execute_action(player: &MacroPlayer, controller: &InputController, action: Action) {
    match action {
        Action::Pause => player.pause(),
        Action::Resume => player.resume(),
        Action::KeyPress(key) => controller.key_press(&key),
        Action::Click(x, y) => controller.click(x as i32, y as i32, "left"),
        Action::BezierMoveTo(x, y) => controller.bezier_move_to(x as i32, y as i32, 0.0),
        Action::MoveTo(x, y) => controller.move_to(x as i32, y as i32),
        Action::MouseDown(button) => controller.mouse_down(None, &button),
        Action::MouseUp(button) => controller.mouse_up(None, &button),
    }
}

/// Apply one AI-step [`Action`](crate::engine::ai::Action) to the input
/// controller — the executor's actuate callback, mirroring `AIExecutor`'s direct
/// `controller.*` calls. `Sleep` clamps negative/NaN to zero so
/// `Duration::from_secs_f64` can't panic in the run thread (which would leak the
/// mode); Python's `time.sleep` raised on a negative delay instead.
fn execute_ai_action(controller: &InputController, action: crate::engine::ai::Action) {
    use crate::engine::ai::Action;
    match action {
        Action::Click(x, y) => controller.click(x as i32, y as i32, "left"),
        Action::KeyPress(key) => controller.key_press(&key),
        Action::TypeText(text) => controller.type_text(&text),
        Action::Scroll(amount, pos) => {
            let pos = pos.map(|(x, y)| (x as i32, y as i32));
            controller.scroll(amount as i32, pos);
        }
        Action::Sleep(secs) => {
            std::thread::sleep(std::time::Duration::from_secs_f64(secs.max(0.0)));
        }
    }
}

/// Parse the sidecar's `ai_detect` reply — `{matches:[{x,y,confidence}], message}`
/// — into the executor's shape. Missing fields default (empty matches, empty
/// message), so a malformed reply degrades to "nothing found" rather than erroring.
fn parse_ai_detect(v: &Value) -> (Vec<crate::engine::ai::Match>, String) {
    let matches = v
        .get("matches")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .map(|m| crate::engine::ai::Match {
                    x: m.get("x").and_then(Value::as_i64).unwrap_or(0),
                    y: m.get("y").and_then(Value::as_i64).unwrap_or(0),
                    confidence: m.get("confidence").and_then(Value::as_f64).unwrap_or(0.0),
                })
                .collect()
        })
        .unwrap_or_default();
    let message = v.get("message").and_then(Value::as_str).unwrap_or("").to_string();
    (matches, message)
}

/// Resolve `play_macro`'s `repeat` argument. `None` reads the macro's saved loop
/// settings; a value coerces through Python's `int()` rules, with `""`/`∞`
/// meaning infinite (0).
fn resolve_repeat(repeat: Option<&Value>, m: &Macro) -> i64 {
    match repeat {
        None => {
            if m.loop_enabled && m.loop_count == 0 {
                0
            } else if !m.loop_enabled {
                1
            } else {
                m.loop_count
            }
        }
        Some(v) => {
            if let Some(s) = v.as_str() {
                if s.is_empty() || s == "\u{221e}" {
                    return 0;
                }
            }
            py_int(v).unwrap_or(1)
        }
    }
}

/// Python `int(value)` for the shapes `repeat` can arrive as: an int/float
/// (truncated toward zero), a base-10 string, or a bool. Anything else is `None`.
fn py_int(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Value::String(s) => s.trim().parse::<i64>().ok(),
        Value::Bool(b) => Some(*b as i64),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{py_int, resolve_repeat};
    use crate::models::macro_def::Macro;
    use serde_json::json;

    #[test]
    fn py_int_matches_python_int() {
        // int() truncates toward zero and strips surrounding whitespace.
        assert_eq!(py_int(&json!(3)), Some(3));
        assert_eq!(py_int(&json!(2.9)), Some(2));
        assert_eq!(py_int(&json!(-2.7)), Some(-2));
        assert_eq!(py_int(&json!("5")), Some(5));
        assert_eq!(py_int(&json!("  7 ")), Some(7));
        assert_eq!(py_int(&json!(true)), Some(1));
        assert_eq!(py_int(&json!("abc")), None);
        assert_eq!(py_int(&json!(null)), None);
    }

    fn macro_with(loop_enabled: bool, loop_count: i64) -> Macro {
        Macro { loop_enabled, loop_count, ..Default::default() }
    }

    #[test]
    fn resolve_repeat_none_reads_saved_loop_settings() {
        assert_eq!(resolve_repeat(None, &macro_with(true, 0)), 0); // loop enabled, count 0 → forever
        assert_eq!(resolve_repeat(None, &macro_with(false, 1)), 1); // no loop → once
        assert_eq!(resolve_repeat(None, &macro_with(true, 5)), 5); // fixed count
    }

    #[test]
    fn resolve_repeat_value_coerces_like_python() {
        let m = macro_with(false, 1);
        assert_eq!(resolve_repeat(Some(&json!("")), &m), 0); // empty string → infinite
        assert_eq!(resolve_repeat(Some(&json!("\u{221e}")), &m), 0); // ∞ → infinite
        assert_eq!(resolve_repeat(Some(&json!("3")), &m), 3);
        assert_eq!(resolve_repeat(Some(&json!(3)), &m), 3);
        assert_eq!(resolve_repeat(Some(&json!(2.9)), &m), 2); // int() truncates
        assert_eq!(resolve_repeat(Some(&json!("abc")), &m), 1); // int() raises → fallback 1
    }
}
