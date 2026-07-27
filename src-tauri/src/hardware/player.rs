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

use std::collections::BTreeSet;
use std::panic::{catch_unwind, AssertUnwindSafe};
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaybackOutcome {
    Completed { iterations: u64 },
    Stopped,
    Failed(String),
}

impl PlaybackOutcome {
    pub fn status(&self) -> &'static str {
        match self {
            Self::Completed { .. } => "completed",
            Self::Stopped => "stopped",
            Self::Failed(_) => "failed",
        }
    }
}

const MAX_CRITICAL_LATENESS: Duration = Duration::from_millis(250);

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

fn valid_button(button: &str) -> bool {
    matches!(
        button.to_ascii_lowercase().as_str(),
        "left" | "primary" | "right" | "secondary" | "middle" | "center" | "x1" | "x2"
    )
}

fn validate_inputs(macro_def: &Macro, target_resolution: (u32, u32), speed: f64) -> Result<(), String> {
    macro_def.validate_for_playback()?;
    if target_resolution.0 == 0 || target_resolution.1 == 0 {
        return Err("target resolution must be non-zero".to_string());
    }
    if !speed.is_finite() || speed <= 0.0 || speed > 100.0 {
        return Err("playback speed must be finite and between 0 and 100".to_string());
    }
    if has_button_edges(&macro_def.events)
        && macro_def
            .events
            .iter()
            .any(|event| event.event_type == InputEventType::MouseClick)
    {
        return Err(
            "macro mixes legacy MOUSE_CLICK events with explicit MOUSE_DOWN/MOUSE_UP edges; \
             repair or re-record it to avoid duplicate or missing clicks"
                .to_string(),
        );
    }
    for (index, event) in macro_def.events.iter().enumerate() {
        if matches!(
            event.event_type,
            InputEventType::MouseClick | InputEventType::MouseDown | InputEventType::MouseUp
        ) && !valid_button(&event.button)
        {
            return Err(format!(
                "event {index} uses unsupported mouse button '{}'",
                event.button
            ));
        }
        if matches!(
            event.event_type,
            InputEventType::KeyPress | InputEventType::KeyDown | InputEventType::KeyUp
        ) && crate::hardware::input::resolve_vk(&event.key).is_none()
        {
            return Err(format!("event {index} uses unsupported key '{}'", event.key));
        }
    }
    Ok(())
}

fn is_critical(event_type: InputEventType) -> bool {
    matches!(
        event_type,
        InputEventType::MouseClick
            | InputEventType::MouseDown
            | InputEventType::MouseUp
            | InputEventType::KeyPress
            | InputEventType::KeyDown
            | InputEventType::KeyUp
            | InputEventType::Scroll
    )
}

fn checkpoint_stops_on_timeout(cfg: &Value) -> bool {
    cfg.get("on_timeout")
        .and_then(Value::as_str)
        .map(|policy| policy != "continue")
        .unwrap_or(true)
}

struct HeldInputs<'a> {
    controller: &'a InputController,
    buttons: BTreeSet<String>,
    keys: BTreeSet<String>,
}

impl<'a> HeldInputs<'a> {
    fn new(controller: &'a InputController) -> Self {
        Self {
            controller,
            buttons: BTreeSet::new(),
            keys: BTreeSet::new(),
        }
    }

    fn release_all(&mut self) -> Vec<String> {
        let mut failures = Vec::new();
        for key in std::mem::take(&mut self.keys) {
            if let Err(error) = self.controller.try_key_up(&key) {
                failures.push(error.to_string());
            }
        }
        for button in std::mem::take(&mut self.buttons) {
            if let Err(error) = self.controller.try_mouse_up(None, &button) {
                failures.push(error.to_string());
            }
        }
        failures
    }
}

impl Drop for HeldInputs<'_> {
    fn drop(&mut self) {
        let _ = self.release_all();
    }
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
    /// Terminal result written by the playback thread and consumed by `wait`.
    outcome: Mutex<Option<PlaybackOutcome>>,
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
            outcome: Mutex::new(None),
        }
    }

    /// Block while paused, mirroring Python's `self._pause_event.wait()`.
    ///
    /// The stop flag is checked *under the pause lock*, which (together with
    /// `stop()` setting the flag under the same lock before notifying) closes
    /// the lost-wakeup race: a stop that lands between the predicate check and
    /// the `wait` can't be missed.
    fn wait_while_paused(&self) -> Duration {
        let started = Instant::now();
        let mut paused = self.paused.lock().unwrap();
        let was_paused = *paused;
        while *paused && !self.stop_flag.load(Ordering::SeqCst) {
            paused = self.pause_cv.wait(paused).unwrap();
        }
        if was_paused {
            started.elapsed()
        } else {
            Duration::ZERO
        }
    }

    /// Wait for at most `duration`, waking immediately when `stop()` notifies
    /// the shared condition variable. Taking the pause lock around the predicate
    /// closes the same lost-wakeup window as `wait_while_paused`.
    fn wait_interruptibly(&self, duration: Duration) -> bool {
        let guard = self.paused.lock().unwrap();
        if self.stop_flag.load(Ordering::SeqCst) {
            return false;
        }
        let _ = self
            .pause_cv
            .wait_timeout_while(guard, duration, |_| !self.stop_flag.load(Ordering::SeqCst))
            .unwrap();
        !self.stop_flag.load(Ordering::SeqCst)
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
                if !self.wait_interruptibly(remaining - Duration::from_micros(1000)) {
                    return;
                }
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
) -> PlaybackOutcome {
    // Replays in the physical pixels the recorder captured: hold
    // Per-Monitor-V2 for the whole playback thread, the same idiom as the
    // recorder's hook thread (see `hardware::dpi`).
    let _aware = crate::hardware::dpi::PerMonitorAware::new();
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
    let mut held = HeldInputs::new(&controller);
    let mut outcome = PlaybackOutcome::Completed { iterations: 0 };

    'iterations: loop {
        iteration += 1;
        if !should_run_iteration(loop_count, loop_enabled, iteration) {
            break;
        }
        shared.iteration.store(iteration, Ordering::SeqCst);

        // 1ms timer + no mouse acceleration for the duration of this iteration;
        // both restore on drop at the end of the loop body (matching Python's
        // `timeEndPeriod` finally + `_NoAcceleration` context manager). The player
        // still reconciles the visible cursor after relative movement because
        // display scaling can affect the pointer path independently.
        let _timer = HiResTimer::begin();
        let _no_accel = NoAcceleration::new();

        // Fresh absolute timeline per iteration. Pauses, explicit waits and
        // checkpoints extend `timeline_shift`; resuming never releases a burst
        // of overdue clicks.
        let t0 = Instant::now();
        let mut timeline_shift = Duration::ZERO;
        let mut prev: Option<(i32, i32)> = None;

        for (event_index, event) in events.iter().enumerate() {
            if shared.stop_flag.load(Ordering::SeqCst) {
                outcome = PlaybackOutcome::Stopped;
                break 'iterations;
            }
            timeline_shift += shared.wait_while_paused();
            if shared.stop_flag.load(Ordering::SeqCst) {
                outcome = PlaybackOutcome::Stopped;
                break 'iterations;
            }

            let target = t0 + timeline_shift + Duration::from_secs_f64(event.timestamp / speed);
            shared.wait_until(target);
            if shared.stop_flag.load(Ordering::SeqCst) {
                outcome = PlaybackOutcome::Stopped;
                break 'iterations;
            }

            if button_edges && event.event_type == InputEventType::MouseClick {
                continue;
            }
            let lateness = Instant::now().saturating_duration_since(target);
            if is_critical(event.event_type) && lateness > MAX_CRITICAL_LATENESS {
                outcome = PlaybackOutcome::Failed(format!(
                    "event {event_index} ({:?}) was {} ms late; stopped before sending a catch-up input",
                    event.event_type,
                    lateness.as_millis()
                ));
                break 'iterations;
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
                    let started = Instant::now();
                    match run_checkpoint(&shared, &controller, detect, cfg, x_scale, y_scale) {
                        Ok(CheckpointOutcome::Reached) => {}
                        Ok(CheckpointOutcome::Stopped) => {
                            outcome = PlaybackOutcome::Stopped;
                            break 'iterations;
                        }
                        Ok(CheckpointOutcome::TimedOut) if checkpoint_stops_on_timeout(cfg) => {
                            outcome = PlaybackOutcome::Failed(format!(
                                "checkpoint at event {event_index} timed out; no further repetitions were sent"
                            ));
                            break 'iterations;
                        }
                        Ok(CheckpointOutcome::TimedOut) => {}
                        Err(error) => {
                            outcome =
                                PlaybackOutcome::Failed(format!("checkpoint at event {event_index} failed: {error}"));
                            break 'iterations;
                        }
                    }
                    timeline_shift += started.elapsed();
                }
                continue;
            }

            let delivered = (|| {
                match event.event_type {
                    // Mouse moves replay as relative deltas (after the first, which
                    // seeds the absolute position) so camera-drag Raw Input tracks.
                    InputEventType::MouseMove => {
                        let (x, y) = scale_point(event.x, event.y, x_scale, y_scale);
                        match prev {
                            Some((px, py)) => {
                                let (dx, dy) = (x - px, y - py);
                                if dx != 0 || dy != 0 {
                                    controller.try_move_relative(dx, dy)?;
                                }
                                // Relative input preserves Raw Input for game camera
                                // movement, but Windows may scale the visible cursor
                                // path at non-100% display scaling. Reconcile it to
                                // the recorded physical point after every segment so
                                // drift can never accumulate into a missed click.
                                controller.try_sync_cursor_to(x, y)?;
                            }
                            None => controller.try_move_to(x, y)?,
                        }
                        prev = Some((x, y));
                        Ok(())
                    }
                    // Reconcile with SetCursorPos immediately before each button
                    // edge, then send ONLY the button. SetCursorPos does not inject
                    // a relative Raw Input delta, so right-click-drag camera tracking
                    // keeps the recorded movement while UI clicks land exactly.
                    InputEventType::MouseDown => {
                        let (x, y) = scale_point(event.x, event.y, x_scale, y_scale);
                        controller.try_sync_cursor_to(x, y)?;
                        controller.try_mouse_down(None, &event.button)?;
                        held.buttons.insert(event.button.to_ascii_lowercase());
                        prev = Some((x, y));
                        Ok(())
                    }
                    InputEventType::MouseUp => {
                        let (x, y) = scale_point(event.x, event.y, x_scale, y_scale);
                        controller.try_sync_cursor_to(x, y)?;
                        controller.try_mouse_up(None, &event.button)?;
                        held.buttons.remove(&event.button.to_ascii_lowercase());
                        prev = Some((x, y));
                        Ok(())
                    }
                    InputEventType::MouseClick => {
                        let (x, y) = scale_point(event.x, event.y, x_scale, y_scale);
                        controller.try_sync_cursor_to(x, y)?;
                        controller.try_mouse_down(None, &event.button)?;
                        held.buttons.insert(event.button.to_ascii_lowercase());
                        controller.try_mouse_up(None, &event.button)?;
                        held.buttons.remove(&event.button.to_ascii_lowercase());
                        prev = Some((x, y));
                        Ok(())
                    }
                    InputEventType::KeyPress => {
                        controller.try_key_down(&event.key)?;
                        held.keys.insert(event.key.to_ascii_lowercase());
                        controller.try_key_up(&event.key)?;
                        held.keys.remove(&event.key.to_ascii_lowercase());
                        Ok(())
                    }
                    InputEventType::KeyDown => {
                        controller.try_key_down(&event.key)?;
                        held.keys.insert(event.key.to_ascii_lowercase());
                        Ok(())
                    }
                    InputEventType::KeyUp => {
                        controller.try_key_up(&event.key)?;
                        held.keys.remove(&event.key.to_ascii_lowercase());
                        Ok(())
                    }
                    InputEventType::Scroll => {
                        let (x, y) = scale_point(event.x, event.y, x_scale, y_scale);
                        controller.try_scroll(event.delta as i32, Some((x, y)))
                    }
                    InputEventType::Wait => {
                        if event.duration > 0.0 {
                            let wait = Duration::from_secs_f64(event.duration / speed);
                            shared.wait_until(Instant::now() + wait);
                            timeline_shift += wait;
                        }
                        Ok(())
                    }
                    InputEventType::Checkpoint => Ok(()),
                }
            })();
            if let Err(error) = delivered {
                outcome = PlaybackOutcome::Failed(format!(
                    "input delivery failed at event {event_index} ({:?}): {error}",
                    event.event_type
                ));
                break 'iterations;
            }
        }

        if shared.stop_flag.load(Ordering::SeqCst) {
            outcome = PlaybackOutcome::Stopped;
            break;
        }
        let release_failures = held.release_all();
        if !release_failures.is_empty() {
            outcome = PlaybackOutcome::Failed(format!(
                "could not release held input(s) at repetition {iteration}: {}",
                release_failures.join("; ")
            ));
            break;
        }
        outcome = PlaybackOutcome::Completed { iterations: iteration };
    }

    let release_failures = held.release_all();
    if !release_failures.is_empty() {
        let release_message = format!(
            "could not release held input(s) while ending playback: {}",
            release_failures.join("; ")
        );
        outcome = match outcome {
            PlaybackOutcome::Failed(message) => PlaybackOutcome::Failed(format!("{message}; {release_message}")),
            _ => PlaybackOutcome::Failed(release_message),
        };
    }
    outcome
}

/// Execute one vision checkpoint: the orchestration half of Python's
/// `MacroPlayer._run_checkpoint`. Detection (frame grab, region collapse, method
/// route) lives behind the injected `detect`; this drives the poll/timeout loop
/// and the resulting click/drag/key. The Python player logs checkpoint state
/// (skips, timeouts, hold) only to stderr, never the UI, so those lines are
/// dropped here; a timed-out checkpoint just returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CheckpointOutcome {
    Reached,
    TimedOut,
    Stopped,
}

fn detect_checked(detect: &CheckpointDetect, cfg: &Value) -> Result<Vec<Detection>, String> {
    catch_unwind(AssertUnwindSafe(|| detect(cfg))).map_err(|payload| {
        payload
            .downcast_ref::<&str>()
            .map(|s| format!("screen detector panicked: {s}"))
            .or_else(|| {
                payload
                    .downcast_ref::<String>()
                    .map(|s| format!("screen detector panicked: {s}"))
            })
            .unwrap_or_else(|| "screen detector panicked".to_string())
    })
}

fn run_checkpoint(
    shared: &PlayerShared,
    controller: &InputController,
    detect: &CheckpointDetect,
    cfg: &Value,
    x_scale: f64,
    y_scale: f64,
) -> Result<CheckpointOutcome, String> {
    let mode = cfg.get("mode").and_then(Value::as_str).unwrap_or("wait_for");
    let timeout = cfg.get("timeout").and_then(Value::as_f64).unwrap_or(10.0);

    if mode == "hold_follow" {
        return hold_follow(shared, controller, cfg, detect, x_scale, y_scale, timeout);
    }

    // wait_for: poll until the target appears or the timeout elapses.
    let deadline = Instant::now() + Duration::from_secs_f64(timeout);
    let poll = cfg.get("poll").and_then(Value::as_f64).unwrap_or(0.05);
    let found = loop {
        if Instant::now() >= deadline || shared.stop_flag.load(Ordering::SeqCst) {
            break None;
        }
        let _ = shared.wait_while_paused();
        if shared.stop_flag.load(Ordering::SeqCst) {
            return Ok(CheckpointOutcome::Stopped);
        }
        let matches = detect_checked(detect, cfg)?;
        if let Some(m) = matches.into_iter().next() {
            break Some(m);
        }
        let _ = shared.wait_interruptibly(Duration::from_secs_f64(poll));
    };
    let found = match found {
        Some(f) => f,
        None if shared.stop_flag.load(Ordering::SeqCst) => return Ok(CheckpointOutcome::Stopped),
        None => return Ok(CheckpointOutcome::TimedOut),
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
                    )?;
                }
            }
        }
        return if shared.stop_flag.load(Ordering::SeqCst) {
            Ok(CheckpointOutcome::Stopped)
        } else {
            Ok(CheckpointOutcome::Reached)
        };
    }

    // Single surgical point: click the exact pixel offset from the match top-left,
    // else the match centre.
    let (click_x, click_y) = match cfg.get("click_offset").and_then(Value::as_array) {
        Some(o) if o.len() == 2 => (
            ((top_left_x as f64 + o[0].as_f64().unwrap_or(0.0)) * x_scale) as i32,
            ((top_left_y as f64 + o[1].as_f64().unwrap_or(0.0)) * y_scale) as i32,
        ),
        _ => ((found.x as f64 * x_scale) as i32, (found.y as f64 * y_scale) as i32),
    };
    do_action(controller, cfg, click_x, click_y)?;
    Ok(CheckpointOutcome::Reached)
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
) -> Result<(), String> {
    let button = cfg.get("button").and_then(Value::as_str).unwrap_or("left");

    // Key action can't sweep, so press the key and return.
    if cfg.get("action").and_then(Value::as_str) == Some("key") {
        if let Some(key) = cfg.get("key").and_then(Value::as_str).filter(|k| !k.is_empty()) {
            controller.try_key_press(key).map_err(|e| e.to_string())?;
            return Ok(());
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
    controller.try_move_to(start_x, start_y).map_err(|e| e.to_string())?;
    controller.try_mouse_down(None, button).map_err(|e| e.to_string())?;
    let trace_result = (|| -> Result<(), String> {
        for i in 1..=dist {
            if shared.stop_flag.load(Ordering::SeqCst) {
                break;
            }
            let (px, py) = screen(i as f64 / dist as f64);
            controller.try_move_to(px, py).map_err(|e| e.to_string())?;
            std::thread::sleep(Duration::from_secs_f64(0.008));
        }
        let (end_x, end_y) = screen(1.0);
        controller.try_move_to(end_x, end_y).map_err(|e| e.to_string())
    })();
    let release_result = controller.try_mouse_up(None, button).map_err(|e| e.to_string());
    trace_result?;
    release_result
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
) -> Result<CheckpointOutcome, String> {
    let hold_button = cfg.get("hold_button").and_then(Value::as_str).unwrap_or("left");
    let release_when = cfg.get("release_when").and_then(Value::as_str).unwrap_or("lost");
    let poll = cfg.get("poll").and_then(Value::as_f64).unwrap_or(0.05);
    let deadline = Instant::now() + Duration::from_secs_f64(timeout);

    controller
        .try_mouse_down(None, hold_button)
        .map_err(|e| e.to_string())?;
    let mut last_seen = false;
    let mut condition_met = false;
    let follow_result = (|| -> Result<(), String> {
        while Instant::now() < deadline && !shared.stop_flag.load(Ordering::SeqCst) {
            let _ = shared.wait_while_paused();
            if shared.stop_flag.load(Ordering::SeqCst) {
                break;
            }
            let matches = detect_checked(detect, cfg)?;
            if let Some(m) = matches.first() {
                let tx = (m.x as f64 * x_scale) as i32;
                let ty = (m.y as f64 * y_scale) as i32;
                controller.try_move_to(tx, ty).map_err(|e| e.to_string())?;
                last_seen = true;
                // Release when the target is present.
                if release_when == "found" {
                    condition_met = true;
                    break;
                }
            } else if release_when == "lost" && last_seen {
                // Release when the target disappears.
                condition_met = true;
                break;
            }
            let _ = shared.wait_interruptibly(Duration::from_secs_f64(poll));
        }
        Ok(())
    })();
    let release_result = controller.try_mouse_up(None, hold_button).map_err(|e| e.to_string());
    follow_result?;
    release_result?;
    if shared.stop_flag.load(Ordering::SeqCst) {
        Ok(CheckpointOutcome::Stopped)
    } else if condition_met {
        Ok(CheckpointOutcome::Reached)
    } else {
        Ok(CheckpointOutcome::TimedOut)
    }
}

/// The wait_for action once the target is found: click the pixel, press a key, or
/// (action `none`) do nothing. A `key` action with no key set is a no-op, exactly
/// as Python's `if action == "key" and cfg.get("key")` guarded.
fn do_action(controller: &InputController, cfg: &Value, x: i32, y: i32) -> Result<(), String> {
    let action = cfg.get("action").and_then(Value::as_str).unwrap_or("click");
    if action == "key" {
        if let Some(key) = cfg.get("key").and_then(Value::as_str).filter(|k| !k.is_empty()) {
            controller.try_key_press(key).map_err(|e| e.to_string())?;
        }
    } else if action == "click" {
        let button = cfg.get("button").and_then(Value::as_str).unwrap_or("left");
        controller.try_click(x, y, button).map_err(|e| e.to_string())?;
    }
    Ok(())
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

    pub fn validate(&self, macro_def: &Macro, target_resolution: (u32, u32), speed: f64) -> Result<(), String> {
        validate_inputs(macro_def, target_resolution, speed)
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
    ) -> Result<(), String> {
        if self.is_playing() {
            return Err("a macro is already playing".to_string());
        }
        validate_inputs(&macro_def, target_resolution, speed)?;
        // Reap the previous (finished or stopped) thread before starting a new
        // one, so at most one play thread ever touches `shared`. `is_playing`
        // being false here means the old thread has stopped or is exiting, so
        // this join is bounded.
        if let Some(handle) = self.thread.lock().unwrap().take() {
            let _ = handle.join();
        }

        *self.shared.outcome.lock().unwrap() = None;
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
            let result = catch_unwind(AssertUnwindSafe(|| {
                play_loop(Arc::clone(&shared), macro_def, target_resolution, speed, checkpoint)
            }));
            let outcome = match result {
                Ok(outcome) => outcome,
                Err(payload) => {
                    let reason = payload
                        .downcast_ref::<&str>()
                        .map(|s| (*s).to_string())
                        .or_else(|| payload.downcast_ref::<String>().cloned())
                        .unwrap_or_else(|| "unknown panic".to_string());
                    PlaybackOutcome::Failed(format!("playback thread panicked: {reason}"))
                }
            };
            shared.playing.store(false, Ordering::SeqCst);
            *shared.outcome.lock().unwrap() = Some(outcome);
        });
        *self.thread.lock().unwrap() = Some(handle);
        Ok(())
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
    pub fn wait(&self) -> PlaybackOutcome {
        let handle = self.thread.lock().unwrap().take();
        if let Some(handle) = handle {
            if handle.join().is_err() {
                self.shared.playing.store(false, Ordering::SeqCst);
                return PlaybackOutcome::Failed("playback thread terminated unexpectedly".to_string());
            }
        }
        self.shared.outcome.lock().unwrap().take().unwrap_or_else(|| {
            if self.shared.stop_flag.load(Ordering::SeqCst) {
                PlaybackOutcome::Stopped
            } else {
                PlaybackOutcome::Failed("playback ended without an outcome".to_string())
            }
        })
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
    fn checkpoint_timeout_is_fail_closed_unless_explicitly_overridden() {
        assert!(checkpoint_stops_on_timeout(&serde_json::json!({})));
        assert!(checkpoint_stops_on_timeout(
            &serde_json::json!({ "on_timeout": "stop" })
        ));
        assert!(!checkpoint_stops_on_timeout(
            &serde_json::json!({ "on_timeout": "continue" })
        ));
    }

    #[test]
    fn preflight_rejects_unknown_inputs_and_unsafe_speed() {
        let mut key = move_event(0, 0, 0.0);
        key.event_type = InputEventType::KeyPress;
        key.key = "definitely_not_a_key".to_string();
        let bad_key = Macro {
            name: "bad-key".to_string(),
            events: vec![key],
            ..Default::default()
        };
        assert!(validate_inputs(&bad_key, (1920, 1080), 1.0)
            .unwrap_err()
            .contains("unsupported key"));

        let mut click = move_event(0, 0, 0.0);
        click.event_type = InputEventType::MouseClick;
        click.button = "mystery".to_string();
        let bad_button = Macro {
            name: "bad-button".to_string(),
            events: vec![click],
            ..Default::default()
        };
        assert!(validate_inputs(&bad_button, (1920, 1080), 1.0)
            .unwrap_err()
            .contains("unsupported mouse button"));

        let valid = Macro {
            name: "valid".to_string(),
            events: vec![move_event(0, 0, 0.0)],
            ..Default::default()
        };
        assert!(validate_inputs(&valid, (1920, 1080), f64::NAN).is_err());
        assert!(validate_inputs(&valid, (0, 1080), 1.0).is_err());
    }

    #[test]
    fn preflight_rejects_ambiguous_mixed_click_encodings() {
        let mut click = move_event(10, 10, 0.0);
        click.event_type = InputEventType::MouseClick;
        click.button = "left".to_string();
        let mut down = move_event(10, 10, 0.1);
        down.event_type = InputEventType::MouseDown;
        down.button = "left".to_string();
        let mixed = Macro {
            name: "mixed-click-encoding".to_string(),
            events: vec![click, down],
            ..Default::default()
        };

        assert!(validate_inputs(&mixed, (1920, 1080), 1.0)
            .unwrap_err()
            .contains("mixes legacy MOUSE_CLICK"));
    }

    #[test]
    fn pause_gate_reports_time_to_shift_out_of_the_timeline() {
        let shared = Arc::new(PlayerShared::new());
        *shared.paused.lock().unwrap() = true;
        let resumer = Arc::clone(&shared);
        let thread = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            *resumer.paused.lock().unwrap() = false;
            resumer.pause_cv.notify_all();
        });
        let paused_for = shared.wait_while_paused();
        thread.join().unwrap();
        assert!(
            paused_for >= Duration::from_millis(15),
            "pause duration is rebased instead of causing catch-up: {paused_for:?}"
        );
    }

    #[test]
    fn long_timeline_wait_wakes_immediately_on_stop() {
        let shared = Arc::new(PlayerShared::new());
        let waiter = Arc::clone(&shared);
        let started = Instant::now();
        let thread = std::thread::spawn(move || {
            waiter.wait_until(Instant::now() + Duration::from_secs(5));
        });

        std::thread::sleep(Duration::from_millis(20));
        {
            let _guard = shared.paused.lock().unwrap();
            shared.stop_flag.store(true, Ordering::SeqCst);
        }
        shared.pause_cv.notify_all();
        thread.join().unwrap();

        assert!(started.elapsed() < Duration::from_millis(300));
    }

    #[test]
    fn playback_outcomes_map_to_honest_history_statuses() {
        assert_eq!(PlaybackOutcome::Completed { iterations: 1 }.status(), "completed");
        assert_eq!(PlaybackOutcome::Stopped.status(), "stopped");
        assert_eq!(PlaybackOutcome::Failed("delivery".to_string()).status(), "failed");
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
        player.play(macro_def, (1000, 1000), 1.0, None).unwrap();
        assert!(player.is_playing(), "playing immediately after play()");
        assert_eq!(player.wait(), PlaybackOutcome::Completed { iterations: 2 });
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

    /// The whole promise of the DPI fix in one test: a point RECORDED through
    /// the real hook must REPLAY onto that same physical pixel through the
    /// real playback thread, at whatever scaling the display runs at. The
    /// field bug — clicks landing a scale factor short at 125% — is exactly
    /// record-space and replay-space disagreeing, which this asserts they
    /// cannot. Ignored by default: it moves the real cursor, but never clicks.
    #[test]
    #[ignore = "moves the real mouse cursor"]
    fn replay_lands_on_the_point_the_recorder_saw() {
        use crate::hardware::input::InputController;
        use crate::hardware::recorder::MacroRecorder;
        use std::collections::HashSet;

        // Mirror `run()`: the process raise happens before any thread exists.
        crate::hardware::dpi::raise_process_to_per_monitor_v2();

        let controller = InputController::new();
        let physical = crate::hardware::screen_size();
        let mut original = windows_sys::Win32::Foundation::POINT { x: 0, y: 0 };
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut original);
        }

        // Record a short path ending at a known physical point. No buttons are
        // injected, so this hardware regression test cannot activate anything
        // on the user's desktop.
        let mut rec = MacroRecorder::new(physical, HashSet::new());
        rec.start();
        std::thread::sleep(Duration::from_millis(100)); // hook thread arms
        controller.move_to(620, 460); // approach from somewhere else
        controller.move_to(820, 640);
        std::thread::sleep(Duration::from_millis(150)); // drain the queue
        let mut macro_def = rec.stop();
        macro_def.loop_count = 1; // stop() leaves 0, which play() reads as "infinite"

        // Park the cursor FAR from the target: the replay must travel there,
        // not merely already be there.
        controller.move_to(80, 80);

        // The real playback path — the same call core makes, with the same
        // physical target `resolve_screen` supplies. 8x speed collapses the
        // recorded inter-event delays; pacing is the other test's job.
        let player = MacroPlayer::new();
        let events = macro_def.events.clone();
        player.play(macro_def, physical, 8.0, None).unwrap();
        assert!(matches!(player.wait(), PlaybackOutcome::Completed { iterations: 1 }));
        assert!(!player.is_playing(), "playback finished");

        // Deliberately UNGUARDED readback: the process-wide raise must make this
        // physical too. Before cursor reconciliation, this exact test landed at
        // (870,685) on 125% instead of (820,640).
        let mut pt = windows_sys::Win32::Foundation::POINT { x: 0, y: 0 };
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut pt);
        }
        controller.move_to(original.x, original.y);
        assert!(
            (pt.x - 820).abs() <= 3 && (pt.y - 640).abs() <= 3,
            "replay landed at ({},{}) instead of the recorded physical (820,640) on a {}x{} screen; recorded events: {:?}",
            pt.x,
            pt.y,
            physical.0,
            physical.1,
            events
                .iter()
                .map(|e| (format!("{:?}", e.event_type), e.x, e.y))
                .collect::<Vec<_>>()
        );
    }
}
