//! Macro playback: the threaded replayer that drives `InputController`.
//!
//! Faithful port of `recorder.py::MacroPlayer`. Playback runs on a background
//! thread; the public handle methods (`stop`/`pause`/`resume`/`is_playing`) are
//! called from the command thread and, later, the guard engine, so every piece
//! of shared state lives in an `Arc<PlayerShared>` of atomics plus a `Condvar`
//! pause gate.
//!
//! Two Python conveniences are intentionally dropped as dead code (see
//! MIGRATION-NOTES "MacroPlayer"): `_densify_moves` (no caller) and the
//! `on_event` playback callback (never passed at any of the four `.play()` call
//! sites). Vision `CHECKPOINT` events run when `play()` is handed a
//! `CheckpointDetect`: the `play_macro` path wires one over `core::Vision`, while
//! the vision-agent runner passes `None`, mirroring Python's detector-vs-bare
//! `MacroPlayer` split. Detection lives behind that closure; this file owns only
//! the poll/timeout/action orchestration.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use windows_sys::Win32::Media::{timeBeginPeriod, timeEndPeriod};

use serde_json::Value;

use crate::hardware::input::{InputController, NoAcceleration};
use crate::hardware::vision::Detection;
use crate::models::macro_def::{InputEventType, Macro, MacroEvent};

/// A checkpoint detector: one poll returns the current matches for a checkpoint
/// config. Boxed and `Send + Sync` so the play thread can own it; `play_macro`
/// builds one over `core::Vision`, the vision-agent runner passes `None`.
pub type CheckpointDetect = Box<dyn Fn(&Value) -> Vec<Detection> + Send + Sync>;

/// Whether repetition `iteration` (1-based) should run, given the loop settings.
///
/// Mirrors the break logic in `_play_loop`: a positive `loop_count` plays
/// exactly that many times and *wins* over `loop_enabled` (the `elif not loop`
/// branch is unreachable while `loop_count > 0`); otherwise the macro plays once,
/// or forever when `loop_enabled` is set.
fn should_run_iteration(loop_count: i64, loop_enabled: bool, iteration: u64) -> bool {
    if loop_count > 0 {
        iteration <= loop_count as u64
    } else {
        loop_enabled || iteration <= 1
    }
}

/// Record→target axis scale factors. A zero recorded dimension falls back to 1.0
/// (Python's `tgt / rec if rec else 1.0`), avoiding a divide-by-zero on a
/// malformed macro.
fn compute_scales(record_resolution: (u32, u32), target_resolution: (u32, u32)) -> (f64, f64) {
    let (rec_w, rec_h) = record_resolution;
    let (tgt_w, tgt_h) = target_resolution;
    let x = if rec_w != 0 { tgt_w as f64 / rec_w as f64 } else { 1.0 };
    let y = if rec_h != 0 { tgt_h as f64 / rec_h as f64 } else { 1.0 };
    (x, y)
}

/// True when the macro carries explicit press/release edges. When it does, the
/// legacy synthetic `MOUSE_CLICK` events are skipped during replay so a click
/// isn't fired twice.
fn has_button_edges(events: &[MacroEvent]) -> bool {
    events
        .iter()
        .any(|e| matches!(e.event_type, InputEventType::MouseDown | InputEventType::MouseUp))
}

/// Scale a recorded point to the target resolution, truncating toward zero like
/// Python's `int(v * scale)` (and `InputController::replay_event`).
fn scale_point(x: i64, y: i64, x_scale: f64, y_scale: f64) -> (i32, i32) {
    ((x as f64 * x_scale) as i32, (y as f64 * y_scale) as i32)
}

/// The resolution to scale FROM, correcting a stored `record_resolution` that is
/// smaller than the coordinates it recorded. Recorded coordinates come from the
/// low-level mouse hook, which always reports PHYSICAL pixels; the stored
/// resolution came from `GetSystemMetrics`, which a DPI-unaware recorder (the old
/// Python app, or a build from before per-monitor awareness) read as the
/// *logical*, scaled-down size. On a 2560-wide panel at 125% that stores 2048
/// while the coordinates still run to ~2560, so replaying against the physical
/// target scales every relative delta up by the 1.25 DPI factor and the path
/// visibly "extends" outward. A resolution that does not bound its own
/// coordinates is provably wrong, and there the coordinates are physical and so
/// is the target, so the correct transform is 1:1 — substituting the target makes
/// the scale 1.0 and replays the path exactly. A correctly-recorded resolution
/// always bounds its coordinates, so genuine cross-resolution macros (recorded at
/// 1080p, played at 1440p) are left untouched.
fn effective_record_resolution(
    record_resolution: (u32, u32),
    target_resolution: (u32, u32),
    events: &[MacroEvent],
) -> (u32, u32) {
    let (mut rw, mut rh) = record_resolution;
    let (mut max_x, mut max_y) = (0i64, 0i64);
    for e in events {
        if e.x > max_x {
            max_x = e.x;
        }
        if e.y > max_y {
            max_y = e.y;
        }
    }
    // Valid coordinates in an `rw`-wide space are 0..=rw-1, so a coordinate that
    // reaches `rw` already falls outside the stored space.
    if max_x >= rw as i64 {
        rw = target_resolution.0;
    }
    if max_y >= rh as i64 {
        rh = target_resolution.1;
    }
    (rw, rh)
}

/// State shared between the public `MacroPlayer` handle and its playback thread.
struct PlayerShared {
    /// Set to request the play loop stop at the next event boundary.
    stop_flag: AtomicBool,
    /// True while a playback thread is live.
    playing: AtomicBool,
    /// Current 1-based repetition (0 before the first starts). Read by the UI.
    iteration: AtomicU64,
    /// Total repetitions the UI should show (0 = infinite).
    total_reps: AtomicU64,
    /// `true` while paused. Paired with `pause_cv`.
    paused: Mutex<bool>,
    /// Signaled on resume/stop to wake a paused loop.
    pause_cv: Condvar,
}

impl PlayerShared {
    fn new() -> Self {
        Self {
            stop_flag: AtomicBool::new(false),
            playing: AtomicBool::new(false),
            iteration: AtomicU64::new(0),
            // Python's `__init__` default; `play()` overwrites before each run.
            total_reps: AtomicU64::new(1),
            paused: Mutex::new(false),
            pause_cv: Condvar::new(),
        }
    }

    /// Block while paused, mirroring Python's `self._pause_event.wait()`.
    ///
    /// The stop flag is checked *under the pause lock*, which (together with
    /// `stop()` setting the flag under the same lock before notifying) closes
    /// the lost-wakeup race: a stop that lands between the predicate check and
    /// the `wait` can't be missed.
    fn wait_while_paused(&self) {
        let mut paused = self.paused.lock().unwrap();
        while *paused && !self.stop_flag.load(Ordering::SeqCst) {
            paused = self.pause_cv.wait(paused).unwrap();
        }
    }

    /// Wait until `target`. Sleep the long part of the gap and busy-wait the
    /// final ~1-2ms so event timing stays sub-millisecond on Windows' coarse
    /// sleep grid. Mirrors `_wait_until`; returns early on a stop request.
    fn wait_until(&self, target: Instant) {
        loop {
            if self.stop_flag.load(Ordering::SeqCst) {
                return;
            }
            let now = Instant::now();
            if now >= target {
                return;
            }
            let remaining = target - now;
            if remaining > Duration::from_micros(2000) {
                // Sleep most of the gap; leave ~1ms for the final spin.
                std::thread::sleep(remaining - Duration::from_micros(1000));
            } else {
                // Busy-wait the last ~1-2ms for sub-ms accuracy.
                while Instant::now() < target {
                    if self.stop_flag.load(Ordering::SeqCst) {
                        return;
                    }
                }
                return;
            }
        }
    }
}

/// Raises the system timer resolution to 1ms for the lifetime of the guard, so
/// `thread::sleep` isn't stuck on the default ~15ms grid during playback. Mirrors
/// the `timeBeginPeriod(1)` / `timeEndPeriod(1)` pair around each iteration.
struct HiResTimer;

impl HiResTimer {
    fn begin() -> Self {
        // SAFETY: timeBeginPeriod is a simple FFI call with no invariants; the
        // paired timeEndPeriod runs in Drop.
        unsafe { timeBeginPeriod(1) };
        HiResTimer
    }
}

impl Drop for HiResTimer {
    fn drop(&mut self) {
        // SAFETY: pairs the timeBeginPeriod(1) from `begin`.
        unsafe { timeEndPeriod(1) };
    }
}

/// The playback thread body. Replays `macro_def`'s events at their exact
/// recorded timestamps, scaled to `target_resolution`, `speed`× faster.
fn play_loop(
    shared: Arc<PlayerShared>,
    macro_def: Macro,
    target_resolution: (u32, u32),
    speed: f64,
    checkpoint: Option<CheckpointDetect>,
) {
    let controller = InputController::new();
    let (x_scale, y_scale) = compute_scales(
        effective_record_resolution(macro_def.record_resolution, target_resolution, &macro_def.events),
        target_resolution,
    );
    let speed = if speed > 0.0 { speed } else { 1.0 };
    // Prefer exact down/up for modern macros; skip legacy synthetic clicks when
    // real edges exist so a click isn't fired twice.
    let button_edges = has_button_edges(&macro_def.events);
    let loop_count = macro_def.loop_count;
    let loop_enabled = macro_def.loop_enabled;
    let events = &macro_def.events;

    let mut iteration: u64 = 0;
    loop {
        iteration += 1;
        if !should_run_iteration(loop_count, loop_enabled, iteration) {
            break;
        }
        shared.iteration.store(iteration, Ordering::SeqCst);

        // 1ms timer + no mouse acceleration for the duration of this iteration;
        // both restore on drop at the end of the loop body (matching Python's
        // `timeEndPeriod` finally + `_NoAcceleration` context manager). Mouse
        // acceleration is disabled so relative deltas map 1:1 to cursor motion.
        let _timer = HiResTimer::begin();
        let _no_accel = NoAcceleration::new();

        // Fresh absolute timeline per iteration: each event fires at
        // t0 + timestamp/speed. t0 is deliberately NOT rebased on pause/resume,
        // so events queued during a pause fire in a catch-up burst on resume,
        // a preserved quirk the guard engine depends on (see MIGRATION-NOTES).
        let t0 = Instant::now();
        let mut prev: Option<(i32, i32)> = None;

        for event in events {
            if shared.stop_flag.load(Ordering::SeqCst) {
                break;
            }
            shared.wait_while_paused();
            if shared.stop_flag.load(Ordering::SeqCst) {
                break;
            }

            shared.wait_until(t0 + Duration::from_secs_f64(event.timestamp / speed));

            if button_edges && event.event_type == InputEventType::MouseClick {
                continue;
            }
            // Vision checkpoint: run only when a detector is wired AND the event
            // carries a non-empty config, mirroring Python's
            // `event.type == CHECKPOINT and event.checkpoint`. The vision-agent
            // runner passes no detector, so its checkpoints are silent no-ops,
            // faithful to its bare `MacroPlayer(controller)`. Either way the event
            // is consumed (Python's bare player falls through to a no-op replay).
            if event.event_type == InputEventType::Checkpoint {
                if let (Some(detect), Some(cfg)) = (
                    checkpoint.as_ref(),
                    event
                        .checkpoint
                        .as_ref()
                        .filter(|c| matches!(c, Value::Object(m) if !m.is_empty())),
                ) {
                    run_checkpoint(&shared, &controller, detect, cfg, x_scale, y_scale);
                }
                continue;
            }

            match event.event_type {
                // Mouse moves replay as relative deltas (after the first, which
                // seeds the absolute position) so camera-drag Raw Input tracks.
                InputEventType::MouseMove => {
                    let (x, y) = scale_point(event.x, event.y, x_scale, y_scale);
                    match prev {
                        Some((px, py)) => {
                            let (dx, dy) = (x - px, y - py);
                            if dx != 0 || dy != 0 {
                                controller.move_relative(dx, dy);
                            }
                        }
                        None => controller.move_to(x, y),
                    }
                    prev = Some((x, y));
                }
                // Button edges send ONLY the button, no cursor move: the
                // preceding moves already placed the cursor, and an absolute move
                // here would emit a WM_INPUT that corrupts Raw Input delta
                // tracking (breaks right-click-drag camera rotation).
                InputEventType::MouseDown => {
                    controller.mouse_down(None, &event.button);
                    prev = Some(scale_point(event.x, event.y, x_scale, y_scale));
                }
                InputEventType::MouseUp => {
                    controller.mouse_up(None, &event.button);
                    prev = Some(scale_point(event.x, event.y, x_scale, y_scale));
                }
                // Keys, scroll, wait, and legacy clicks (when no edges exist).
                _ => controller.replay_event(event, 0, 0, x_scale, y_scale),
            }
        }

        if shared.stop_flag.load(Ordering::SeqCst) {
            break;
        }
    }

    shared.playing.store(false, Ordering::SeqCst);
}

/// Execute one vision checkpoint: the orchestration half of Python's
/// `MacroPlayer._run_checkpoint`. Detection (frame grab, region collapse, method
/// route) lives behind the injected `detect`; this drives the poll/timeout loop
/// and the resulting click/drag/key. The Python player logs checkpoint state
/// (skips, timeouts, hold) only to stderr, never the UI, so those lines are
/// dropped here; a timed-out checkpoint just returns.
fn run_checkpoint(
    shared: &PlayerShared,
    controller: &InputController,
    detect: &CheckpointDetect,
    cfg: &Value,
    x_scale: f64,
    y_scale: f64,
) {
    let mode = cfg.get("mode").and_then(Value::as_str).unwrap_or("wait_for");
    let timeout = cfg.get("timeout").and_then(Value::as_f64).unwrap_or(10.0);

    if mode == "hold_follow" {
        hold_follow(shared, controller, cfg, detect, x_scale, y_scale, timeout);
        return;
    }

    // wait_for: poll until the target appears or the timeout elapses.
    let deadline = Instant::now() + Duration::from_secs_f64(timeout);
    let poll = cfg.get("poll").and_then(Value::as_f64).unwrap_or(0.05);
    let found = loop {
        if Instant::now() >= deadline || shared.stop_flag.load(Ordering::SeqCst) {
            break None;
        }
        shared.wait_while_paused();
        if shared.stop_flag.load(Ordering::SeqCst) {
            return;
        }
        let matches = detect(cfg);
        if let Some(m) = matches.into_iter().next() {
            break Some(m);
        }
        std::thread::sleep(Duration::from_secs_f64(poll));
    };
    let found = match found {
        Some(f) => f,
        None => return,
    };

    let top_left_x = found.x - found.w / 2;
    let top_left_y = found.y - found.h / 2;

    // Surgical drawn strokes: drag along every drawn path, relative to the match
    // top-left. `click_lines` wins; an absent-or-empty one falls back to a lone
    // truthy `click_line` (Python's `if not lines and cfg.get("click_line")`).
    // A non-empty `lines` always returns here, even if every entry is malformed.
    let lines: Vec<Value> = match cfg.get("click_lines").and_then(Value::as_array) {
        Some(a) if !a.is_empty() => a.clone(),
        _ => match cfg.get("click_line") {
            Some(cl) if matches!(cl, Value::Array(a) if !a.is_empty()) => vec![cl.clone()],
            _ => Vec::new(),
        },
    };
    if !lines.is_empty() {
        for ln in &lines {
            if shared.stop_flag.load(Ordering::SeqCst) {
                break;
            }
            if let Some(arr) = ln.as_array() {
                if arr.len() == 4 {
                    trace_line(
                        shared,
                        controller,
                        cfg,
                        top_left_x,
                        top_left_y,
                        arr[0].as_f64().unwrap_or(0.0),
                        arr[1].as_f64().unwrap_or(0.0),
                        arr[2].as_f64().unwrap_or(0.0),
                        arr[3].as_f64().unwrap_or(0.0),
                        x_scale,
                        y_scale,
                    );
                }
            }
        }
        return;
    }

    // Single surgical point: click the exact pixel offset from the match top-left,
    // else the match centre.
    let (click_x, click_y) = match cfg.get("click_offset").and_then(Value::as_array) {
        Some(o) if o.len() == 2 => (
            ((top_left_x as f64 + o[0].as_f64().unwrap_or(0.0)) * x_scale) as i32,
            ((top_left_y as f64 + o[1].as_f64().unwrap_or(0.0)) * y_scale) as i32,
        ),
        _ => (
            (found.x as f64 * x_scale) as i32,
            (found.y as f64 * y_scale) as i32,
        ),
    };
    do_action(controller, cfg, click_x, click_y);
}

/// Drag along a drawn line from (sx,sy) to (ex,ey), in crop offsets from the
/// match top-left: hold the button at the start, sweep ~1px steps, release at the
/// end, giving the smooth sweep sliders/charge-bars need. A key action can't
/// sweep, so it falls back to pressing the key at the start.
#[allow(clippy::too_many_arguments)]
fn trace_line(
    shared: &PlayerShared,
    controller: &InputController,
    cfg: &Value,
    tlx: i64,
    tly: i64,
    sx: f64,
    sy: f64,
    ex: f64,
    ey: f64,
    x_scale: f64,
    y_scale: f64,
) {
    let button = cfg.get("button").and_then(Value::as_str).unwrap_or("left");

    // Key action can't sweep, so press the key and return.
    if cfg.get("action").and_then(Value::as_str) == Some("key") {
        if let Some(key) = cfg.get("key").and_then(Value::as_str).filter(|k| !k.is_empty()) {
            controller.key_press(key);
            return;
        }
    }

    let (dx, dy) = (ex - sx, ey - sy);
    let dist = (dx.hypot(dy) as i64).max(1);
    let screen = |t: f64| -> (i32, i32) {
        let px = tlx as f64 + sx + dx * t;
        let py = tly as f64 + sy + dy * t;
        ((px * x_scale) as i32, (py * y_scale) as i32)
    };

    let (start_x, start_y) = screen(0.0);
    controller.move_to(start_x, start_y);
    controller.mouse_down(None, button);
    for i in 1..=dist {
        if shared.stop_flag.load(Ordering::SeqCst) {
            break;
        }
        let (px, py) = screen(i as f64 / dist as f64);
        controller.move_to(px, py);
        std::thread::sleep(Duration::from_secs_f64(0.008)); // smooth cadence so the game tracks the drag
    }
    let (end_x, end_y) = screen(1.0);
    controller.move_to(end_x, end_y);
    controller.mouse_up(None, button);
}

/// Hold a button and continuously move the cursor to the detected target,
/// releasing when the release condition is met. Automates timing minigames
/// (e.g. a charge bar) where you track a moving indicator until it aligns.
#[allow(clippy::too_many_arguments)]
fn hold_follow(
    shared: &PlayerShared,
    controller: &InputController,
    cfg: &Value,
    detect: &CheckpointDetect,
    x_scale: f64,
    y_scale: f64,
    timeout: f64,
) {
    let hold_button = cfg.get("hold_button").and_then(Value::as_str).unwrap_or("left");
    let release_when = cfg.get("release_when").and_then(Value::as_str).unwrap_or("lost");
    let poll = cfg.get("poll").and_then(Value::as_f64).unwrap_or(0.05);
    let deadline = Instant::now() + Duration::from_secs_f64(timeout);

    controller.mouse_down(None, hold_button);
    let mut last_seen = false;
    while Instant::now() < deadline && !shared.stop_flag.load(Ordering::SeqCst) {
        shared.wait_while_paused();
        if shared.stop_flag.load(Ordering::SeqCst) {
            break;
        }
        let matches = detect(cfg);
        if let Some(m) = matches.first() {
            let tx = (m.x as f64 * x_scale) as i32;
            let ty = (m.y as f64 * y_scale) as i32;
            controller.move_to(tx, ty);
            last_seen = true;
            // Release when the target is present.
            if release_when == "found" {
                break;
            }
        } else if release_when == "lost" && last_seen {
            // Release when the target disappears.
            break;
        }
        std::thread::sleep(Duration::from_secs_f64(poll));
    }
    controller.mouse_up(None, hold_button);
}

/// The wait_for action once the target is found: click the pixel, press a key, or
/// (action `none`) do nothing. A `key` action with no key set is a no-op, exactly
/// as Python's `if action == "key" and cfg.get("key")` guarded.
fn do_action(controller: &InputController, cfg: &Value, x: i32, y: i32) {
    let action = cfg.get("action").and_then(Value::as_str).unwrap_or("click");
    if action == "key" {
        if let Some(key) = cfg.get("key").and_then(Value::as_str).filter(|k| !k.is_empty()) {
            controller.key_press(key);
        }
    } else if action == "click" {
        let button = cfg.get("button").and_then(Value::as_str).unwrap_or("left");
        controller.click(x, y, button);
    }
}

/// Plays back recorded macros with coordinate scaling, on a background thread.
pub struct MacroPlayer {
    shared: Arc<PlayerShared>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl MacroPlayer {
    pub fn new() -> Self {
        Self {
            shared: Arc::new(PlayerShared::new()),
            thread: Mutex::new(None),
        }
    }

    pub fn is_playing(&self) -> bool {
        self.shared.playing.load(Ordering::SeqCst)
    }

    pub fn is_paused(&self) -> bool {
        *self.shared.paused.lock().unwrap()
    }

    /// Current 1-based repetition (0 = not yet started / stopped).
    pub fn iteration(&self) -> u64 {
        self.shared.iteration.load(Ordering::SeqCst)
    }

    /// Total repetitions for display (0 = infinite).
    pub fn total_reps(&self) -> u64 {
        self.shared.total_reps.load(Ordering::SeqCst)
    }

    /// Start playback on a background thread. A no-op if a macro is already
    /// playing (Python logs "Already playing, stop first" and returns).
    ///
    /// `checkpoint` is the per-call vision detector for macro checkpoint steps:
    /// `Some` from the main play path (Python's `MacroPlayer(..., detector=...)`),
    /// `None` from the vision-agent runner (Python's detector-less
    /// `MacroPlayer(controller)`), which skips checkpoint steps entirely.
    pub fn play(
        &self,
        macro_def: Macro,
        target_resolution: (u32, u32),
        speed: f64,
        checkpoint: Option<CheckpointDetect>,
    ) {
        if self.is_playing() {
            return;
        }
        // Reap the previous (finished or stopped) thread before starting a new
        // one, so at most one play thread ever touches `shared`. `is_playing`
        // being false here means the old thread has stopped or is exiting, so
        // this join is bounded.
        if let Some(handle) = self.thread.lock().unwrap().take() {
            let _ = handle.join();
        }

        self.shared.stop_flag.store(false, Ordering::SeqCst);
        *self.shared.paused.lock().unwrap() = false;
        self.shared.playing.store(true, Ordering::SeqCst);
        self.shared.iteration.store(0, Ordering::SeqCst);
        let total = if macro_def.loop_count > 0 {
            macro_def.loop_count as u64
        } else {
            0
        };
        self.shared.total_reps.store(total, Ordering::SeqCst);

        let shared = Arc::clone(&self.shared);
        let handle = std::thread::spawn(move || {
            play_loop(shared, macro_def, target_resolution, speed, checkpoint)
        });
        *self.thread.lock().unwrap() = Some(handle);
    }

    pub fn stop(&self) {
        // Set the stop flag under the pause lock, then notify: this ordering is
        // what makes a stop during a pause impossible to miss (see
        // `wait_while_paused`).
        {
            let _guard = self.shared.paused.lock().unwrap();
            self.shared.stop_flag.store(true, Ordering::SeqCst);
        }
        self.shared.pause_cv.notify_all();
        self.shared.playing.store(false, Ordering::SeqCst);
    }

    pub fn pause(&self) {
        if self.is_playing() {
            *self.shared.paused.lock().unwrap() = true;
        }
    }

    pub fn resume(&self) {
        if self.is_playing() {
            let mut paused = self.shared.paused.lock().unwrap();
            if *paused {
                *paused = false;
                self.shared.pause_cv.notify_all();
            }
        }
    }

    pub fn toggle_pause(&self) {
        if self.is_paused() {
            self.resume();
        } else {
            self.pause();
        }
    }

    /// Block until playback finishes. Python's `wait()` takes an optional
    /// timeout, but every call site in this app uses the no-arg form (verified),
    /// so this is a plain join.
    pub fn wait(&self) {
        let handle = self.thread.lock().unwrap().take();
        if let Some(handle) = handle {
            let _ = handle.join();
        }
    }
}

impl Default for MacroPlayer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn move_event(x: i64, y: i64, t: f64) -> MacroEvent {
        MacroEvent {
            event_type: InputEventType::MouseMove,
            timestamp: t,
            x,
            y,
            button: "left".to_string(),
            key: String::new(),
            delta: 0,
            duration: 0.0,
            checkpoint: None,
        }
    }

    #[test]
    fn should_run_iteration_exact_count() {
        // loop_count = N plays exactly N times.
        for it in 1..=3 {
            assert!(should_run_iteration(3, false, it));
        }
        assert!(!should_run_iteration(3, false, 4));
    }

    #[test]
    fn should_run_iteration_loop_count_wins_over_loop_flag() {
        // Precedence: a positive loop_count caps the run even when the loop flag
        // is set (the `elif not loop` branch is unreachable while loop_count > 0).
        for it in 1..=3 {
            assert!(should_run_iteration(3, true, it));
        }
        assert!(!should_run_iteration(3, true, 4));
    }

    #[test]
    fn should_run_iteration_single_shot() {
        // loop_count 0 + loop disabled = play once.
        assert!(should_run_iteration(0, false, 1));
        assert!(!should_run_iteration(0, false, 2));
    }

    #[test]
    fn should_run_iteration_infinite() {
        // loop_count 0 + loop enabled = never stops on its own.
        assert!(should_run_iteration(0, true, 1));
        assert!(should_run_iteration(0, true, 1_000_000));
    }

    #[test]
    fn compute_scales_ratio_and_zero_fallback() {
        assert_eq!(compute_scales((1280, 720), (2560, 1440)), (2.0, 2.0));
        // A zero recorded dimension falls back to 1.0 (no divide-by-zero).
        assert_eq!(compute_scales((0, 0), (2560, 1440)), (1.0, 1.0));
    }

    #[test]
    fn has_button_edges_detects_down_up() {
        let moves = vec![move_event(0, 0, 0.0)];
        assert!(!has_button_edges(&moves));

        let with_down = vec![
            move_event(0, 0, 0.0),
            MacroEvent {
                event_type: InputEventType::MouseDown,
                ..move_event(0, 0, 0.1)
            },
        ];
        assert!(has_button_edges(&with_down));
    }

    #[test]
    fn scale_point_truncates_toward_zero() {
        // int(v * scale) truncation, matching Python + replay_event.
        assert_eq!(scale_point(100, 50, 1.5, 1.5), (150, 75));
        assert_eq!(scale_point(3, 3, 0.9, 0.9), (2, 2)); // 2.7 -> 2
    }

    #[test]
    fn effective_record_resolution_corrects_an_understated_space() {
        let ev = |x, y| MacroEvent {
            event_type: InputEventType::MouseMove,
            timestamp: 0.0,
            x,
            y,
            button: "left".to_string(),
            key: String::new(),
            delta: 0,
            duration: 0.0,
            checkpoint: None,
        };

        // A macro recorded under a LOGICAL 2048x1152 query whose coordinates are
        // the PHYSICAL 2211x1364 (a 2560x1440 panel at 125% scaling): the stored
        // resolution does not bound its own coordinates, so it is replaced by the
        // physical target, making the replay scale 1.0 instead of stretching every
        // delta out by the DPI factor (the "always extending" path).
        let logical = vec![ev(0, 0), ev(2211, 1364)];
        let corrected = effective_record_resolution((2048, 1152), (2560, 1440), &logical);
        assert_eq!(corrected, (2560, 1440));
        assert_eq!(compute_scales(corrected, (2560, 1440)), (1.0, 1.0));

        // A correctly-recorded macro (resolution bounds its coordinates) is left
        // alone, so a genuine cross-resolution rescale still applies.
        let small = vec![ev(0, 0), ev(1919, 1079)];
        let untouched = effective_record_resolution((1920, 1080), (2560, 1440), &small);
        assert_eq!(untouched, (1920, 1080));
        assert_eq!(
            compute_scales(untouched, (2560, 1440)),
            (2560.0 / 1920.0, 1440.0 / 1080.0)
        );
    }

    /// End-to-end timing + loop-control check on the real playback thread.
    /// Ignored by default: it installs the 1ms timer, toggles mouse
    /// acceleration, and moves the real cursor. Run with:
    ///   cargo test --lib -- --ignored player_plays_two_reps_on_a_real_timeline
    #[test]
    #[ignore = "installs the 1ms timer, toggles mouse acceleration, and moves the real cursor"]
    fn player_plays_two_reps_on_a_real_timeline() {
        // Two moves 0.25s apart, played twice => ~0.5s of real pacing. The
        // moves target the same point, so only the timeline (not the cursor)
        // is what the assertion exercises.
        let macro_def = Macro {
            name: "__test__timeline".to_string(),
            record_resolution: (1000, 1000),
            loop_count: 2,
            events: vec![move_event(500, 500, 0.0), move_event(500, 500, 0.25)],
            ..Default::default()
        };

        let player = MacroPlayer::new();
        let start = Instant::now();
        player.play(macro_def, (1000, 1000), 1.0, None);
        assert!(player.is_playing(), "playing immediately after play()");
        player.wait();
        let elapsed = start.elapsed();

        assert!(!player.is_playing(), "finished after wait()");
        assert_eq!(player.iteration(), 2, "ran exactly 2 reps");
        // Lower bound only: proves wait_until actually paced the timeline
        // instead of firing everything instantly. 0.9 * (2 * 0.25s) = 0.45s.
        assert!(
            elapsed >= Duration::from_millis(450),
            "paced timeline took >= 0.45s, got {elapsed:?}",
        );
    }
}
