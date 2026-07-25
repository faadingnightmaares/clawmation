//! GuardEngine — the self-healing overlay that clears interrupt conditions
//! mid-playback (disconnect dialogs, reward popups, AFK checks, …) so an
//! unattended macro keeps running.
//!
//! Mirrors `anime_macro/guards.py::GuardEngine`. One background thread polls at
//! 5Hz; while a macro is actively playing it detects every cooldown-eligible
//! guard in a single batch (one screen grab per cycle),
//! and for each match it pauses the macro, performs the guard's action, waits
//! the configured settle delay, then resumes. Like the other engines it is
//! hardware-free: detection, player state, actuation, and the fire callback are
//! injected, so it runs and tests without real capture, input, or playback.
//!
//! The distinct input/player operations are funnelled through one [`Action`]
//! enum behind a single `actuate` closure. That keeps the engine to the same
//! handful of injected closures as `scheduler`/`chains` (rather than a dozen),
//! and gives tests a literal action log to assert the pause→act→resume bracket
//! and drag sequences against.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use super::sleep_interruptible;
use crate::hardware::vision::Detection;
use crate::models::guard::Guard;

/// 5Hz — fast enough to catch an interrupt within 200ms, slow enough to leave
/// the game and the detector headroom (matches Python's `POLL_INTERVAL`).
const POLL_INTERVAL: Duration = Duration::from_millis(200);

/// Dwell between the moves of a drag sweep, so a game registers it as a
/// continuous drag rather than a teleport (Python sleeps 8ms per step).
const STROKE_STEP: Duration = Duration::from_millis(8);

/// One primitive the engine asks the host to perform. Collapsing every
/// input/player operation into a single enum is what keeps GuardEngine to four
/// injected closures; see the module docs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Pause,
    Resume,
    KeyPress(String),
    Click(i64, i64),
    BezierMoveTo(i64, i64),
    MoveTo(i64, i64),
    MouseDown(String),
    MouseUp(String),
}

/// `detect(eligible_guards) -> {guard_id: [detections]}` — one batched grab.
pub type Detect = Box<dyn Fn(&[Guard]) -> HashMap<String, Vec<Detection>> + Send + Sync>;
/// `player() -> (is_playing, is_paused)`.
pub type PlayerState = Box<dyn Fn() -> (bool, bool) + Send + Sync>;
/// `actuate(action)` — perform one input/player primitive.
pub type Actuate = Box<dyn Fn(Action) + Send + Sync>;
/// `on_fire(guard, x, y)` — UI feedback the instant a guard triggers.
pub type OnFire = Box<dyn Fn(&Guard, i64, i64) + Send + Sync>;

/// The geometric intent of a firing guard, independent of *how* it executes.
/// Whether the click humanizes (Bézier-then-click) and whether the action is a
/// key press instead are execution choices applied in `handle`, not encoded
/// here — exactly as Python decides them inside `_handle`/`_handle_lines`,
/// after the branch that picks the target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// Click a single point (the match centre, or a surgical top-left + offset).
    Click(i64, i64),
    /// Drag along one or more drawn strokes, offset from the match top-left.
    Drag {
        tlx: i64,
        tly: i64,
        strokes: Vec<Vec<i64>>,
    },
}

/// Pick the target for a matched guard, mirroring `_check_guards`: drawn strokes
/// first, then a surgical click-offset from the match's top-left, else the match
/// centre. Pure, so the branch logic tests without a running engine.
pub fn plan_action(guard: &Guard, best: &Detection) -> Plan {
    let tlx = best.x - best.w / 2;
    let tly = best.y - best.h / 2;
    let strokes: Vec<Vec<i64>> = if !guard.click_lines.is_empty() {
        guard.click_lines.clone()
    } else if !guard.click_line.is_empty() {
        vec![guard.click_line.clone()]
    } else {
        Vec::new()
    };
    if !strokes.is_empty() {
        Plan::Drag { tlx, tly, strokes }
    } else if guard.click_offset.len() == 2 {
        Plan::Click(tlx + guard.click_offset[0], tly + guard.click_offset[1])
    } else {
        Plan::Click(best.x, best.y)
    }
}

/// The ordered cursor points for one drag stroke `[sx, sy, ex, ey]` (offsets from
/// the match top-left `tlx, tly`): the start, then one point per integer pixel of
/// travel through the end. A stroke that isn't a 4-tuple produces no points and
/// is skipped, matching Python's `if not ln or len(ln) != 4: continue`. The
/// per-step truncation is deliberately `f64 as i64` (toward zero), reproducing
/// Python's `int(...)` — including its floating-point rounding on fractions.
pub fn stroke_points(stroke: &[i64], tlx: i64, tly: i64) -> Vec<(i64, i64)> {
    if stroke.len() != 4 {
        return Vec::new();
    }
    let (sx, sy, ex, ey) = (stroke[0], stroke[1], stroke[2], stroke[3]);
    let (dx, dy) = (ex - sx, ey - sy);
    let dist = ((dx as f64).hypot(dy as f64) as i64).max(1);
    let screen = |t: f64| -> (i64, i64) {
        (
            ((tlx + sx) as f64 + dx as f64 * t) as i64,
            ((tly + sy) as f64 + dy as f64 * t) as i64,
        )
    };
    let mut pts = Vec::with_capacity(dist as usize + 1);
    pts.push(screen(0.0));
    for i in 1..=dist {
        pts.push(screen(i as f64 / dist as f64));
    }
    pts
}

/// The coordinates `on_fire` reports for a plan: the click point, or a drag's
/// first stroke start. A malformed first stroke yields `None` — Python reads
/// `lines[0][0..2]` there and swallows the resulting `IndexError`, so no
/// callback fires.
fn fire_point(plan: &Plan) -> Option<(i64, i64)> {
    match plan {
        Plan::Click(x, y) => Some((*x, *y)),
        Plan::Drag { tlx, tly, strokes } => {
            let first = strokes.first()?;
            if first.len() >= 2 {
                Some((tlx + first[0], tly + first[1]))
            } else {
                None
            }
        }
    }
}

/// Whether a guard is past its cooldown: a never-fired guard always is; else the
/// elapsed time since its last fire must reach `cooldown` seconds.
fn cooled_down(last: Option<&Instant>, now: Instant, cooldown: f64) -> bool {
    match last {
        Some(t) => now.duration_since(*t).as_secs_f64() >= cooldown,
        None => true,
    }
}

struct Inner {
    detect: Detect,
    player: PlayerState,
    actuate: Actuate,
    on_fire: Option<OnFire>,
    humanize: AtomicBool,
    guards: Mutex<Vec<Guard>>,
    /// guard id → when it last fired, for the cooldown gate.
    last_fired: Mutex<HashMap<String, Instant>>,
    running: AtomicBool,
}

pub struct GuardEngine {
    inner: Arc<Inner>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl GuardEngine {
    pub fn new(
        detect: Detect,
        player: PlayerState,
        actuate: Actuate,
        on_fire: Option<OnFire>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                detect,
                player,
                actuate,
                on_fire,
                humanize: AtomicBool::new(false),
                guards: Mutex::new(Vec::new()),
                last_fired: Mutex::new(HashMap::new()),
                running: AtomicBool::new(false),
            }),
            thread: Mutex::new(None),
        }
    }

    /// Route autonomous clicks along a human-like Bézier curve (the UI's
    /// "humanize clicks" toggle). Set before `start`.
    pub fn set_humanize(&self, on: bool) {
        self.inner.humanize.store(on, Ordering::SeqCst);
    }

    /// Start watching the given guards. Filters to enabled ones and is a no-op
    /// if none are enabled or the engine is already running (matches Python: the
    /// engine simply stays idle rather than spinning a thread).
    pub fn start(&self, guards: Vec<Guard>) {
        let enabled: Vec<Guard> = guards.into_iter().filter(|g| g.enabled).collect();
        if enabled.is_empty() {
            return;
        }
        let mut thread = self.thread.lock().unwrap();
        if thread.is_some() {
            return;
        }
        *self.inner.guards.lock().unwrap() = enabled;
        self.inner.last_fired.lock().unwrap().clear();
        self.inner.running.store(true, Ordering::SeqCst);
        let inner = Arc::clone(&self.inner);
        *thread = Some(thread::spawn(move || inner.run_loop()));
    }

    pub fn stop(&self) {
        self.inner.running.store(false, Ordering::SeqCst);
        if let Some(handle) = self.thread.lock().unwrap().take() {
            let _ = handle.join();
        }
    }

    pub fn is_running(&self) -> bool {
        self.inner.running.load(Ordering::SeqCst)
    }
}

impl Inner {
    fn run_loop(self: Arc<Self>) {
        while self.running.load(Ordering::SeqCst) {
            self.tick();
            sleep_interruptible(&self.running, POLL_INTERVAL);
        }
    }

    /// One poll cycle: gate on active playback, detect the cooldown-eligible
    /// guards in a single batch, then handle every match in order.
    fn tick(&self) {
        // Only act while a macro is actively playing (not paused) — never fight
        // the user or a paused run.
        let (is_playing, is_paused) = (self.player)();
        if !is_playing || is_paused {
            return;
        }
        // One `now` for the whole cycle: every guard's cooldown is measured
        // against the same instant, so detecting exactly the eligible set is
        // faithful to Python's per-guard skip (and keeps it to one grab/cycle).
        let now = Instant::now();
        let eligible: Vec<Guard> = {
            let guards = self.guards.lock().unwrap();
            let last = self.last_fired.lock().unwrap();
            guards
                .iter()
                .filter(|g| g.enabled && cooled_down(last.get(&g.id), now, g.cooldown))
                .cloned()
                .collect()
        };
        if eligible.is_empty() {
            return;
        }

        let detections = (self.detect)(&eligible);
        for guard in &eligible {
            // Don't begin a new handle after stop() was requested; an in-flight
            // one still finishes (releasing the mouse, resuming) inside `handle`.
            if !self.running.load(Ordering::SeqCst) {
                break;
            }
            let matched = detections.get(&guard.id).map(Vec::as_slice).unwrap_or(&[]);
            let Some(best) = matched.first() else {
                continue;
            };
            let plan = plan_action(guard, best);
            self.handle(guard, &plan);
            // Stamp after handling returns (post settle-delay), like Python.
            self.last_fired
                .lock()
                .unwrap()
                .insert(guard.id.clone(), Instant::now());
        }
    }

    /// Pause the macro, perform the guard's action, wait its settle delay, then
    /// resume — the self-healing bracket from `_handle`/`_handle_lines`.
    fn handle(&self, guard: &Guard, plan: &Plan) {
        if let Some(on_fire) = &self.on_fire {
            if let Some((fx, fy)) = fire_point(plan) {
                on_fire(guard, fx, fy);
            }
        }

        let (was_playing, _) = (self.player)();
        (self.actuate)(Action::Pause);

        if guard.action == "key" && !guard.key.is_empty() {
            // A keyed guard presses its key regardless of the geometric plan,
            // matching Python's key branch inside both handlers.
            (self.actuate)(Action::KeyPress(guard.key.clone()));
        } else {
            match plan {
                Plan::Click(x, y) => {
                    if self.humanize.load(Ordering::SeqCst) {
                        // Travel to the target along a human-like curve, then
                        // click in place (no teleport).
                        (self.actuate)(Action::BezierMoveTo(*x, *y));
                    }
                    (self.actuate)(Action::Click(*x, *y));
                }
                Plan::Drag { tlx, tly, strokes } => self.sweep(*tlx, *tly, strokes),
            }
        }

        // Give the game time to react (load screen, reconnect, …).
        if guard.resume_delay > 0.0 {
            sleep_interruptible(&self.running, Duration::from_secs_f64(guard.resume_delay));
        }

        // Resume only if the macro was and still is playing — if it finished or
        // was stopped during our handle, leave it stopped (Python's guard).
        let (still_playing, _) = (self.player)();
        if was_playing && still_playing {
            (self.actuate)(Action::Resume);
        }
    }

    /// Drag along each stroke: press at the start, sweep pixel-by-pixel, release.
    /// The release always runs — even if `stop()` interrupts the sweep — so a
    /// held button is never stranded.
    fn sweep(&self, tlx: i64, tly: i64, strokes: &[Vec<i64>]) {
        for stroke in strokes {
            let pts = stroke_points(stroke, tlx, tly);
            if pts.is_empty() {
                continue; // malformed stroke (len != 4) — skipped, like Python
            }
            let (startx, starty) = pts[0];
            let (endx, endy) = *pts.last().unwrap();
            (self.actuate)(Action::MoveTo(startx, starty));
            (self.actuate)(Action::MouseDown("left".to_string()));
            for &(px, py) in &pts[1..] {
                if !self.running.load(Ordering::SeqCst) {
                    break;
                }
                (self.actuate)(Action::MoveTo(px, py));
                thread::sleep(STROKE_STEP);
            }
            // Always land on the end and release (Python's `finally`).
            (self.actuate)(Action::MoveTo(endx, endy));
            (self.actuate)(Action::MouseUp("left".to_string()));
        }
    }
}

#[cfg(test)]
impl GuardEngine {
    /// Load guards and arm the engine without spawning the poll thread, so a
    /// test can drive [`Inner::tick`] deterministically.
    fn test_prime(&self, guards: Vec<Guard>) {
        *self.inner.guards.lock().unwrap() = guards;
        self.inner.running.store(true, Ordering::SeqCst);
    }

    fn test_tick(&self) {
        self.inner.tick();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detection(x: i64, y: i64, w: i64, h: i64) -> Detection {
        Detection {
            label: "m".into(),
            x,
            y,
            w,
            h,
            confidence: 1.0,
            roi_offset: [0, 0],
        }
    }

    type ActionLog = Arc<Mutex<Vec<Action>>>;
    type FireLog = Arc<Mutex<Vec<(String, i64, i64)>>>;

    /// An engine whose detect always returns `dets` for guard `id`, a player
    /// that's playing-and-not-paused, and an actuate/on_fire that append to
    /// shared logs.
    fn harness(id: &str, dets: Vec<Detection>) -> (GuardEngine, ActionLog, FireLog) {
        let log: ActionLog = Arc::new(Mutex::new(Vec::new()));
        let fires: FireLog = Arc::new(Mutex::new(Vec::new()));
        let (alog, flog, id) = (Arc::clone(&log), Arc::clone(&fires), id.to_string());
        let engine = GuardEngine::new(
            Box::new(move |_guards: &[Guard]| {
                let mut m = HashMap::new();
                m.insert(id.clone(), dets.clone());
                m
            }),
            Box::new(|| (true, false)),
            Box::new(move |a: Action| alog.lock().unwrap().push(a)),
            Some(Box::new(move |g: &Guard, x, y| {
                flog.lock().unwrap().push((g.name.clone(), x, y))
            })),
        );
        (engine, log, fires)
    }

    fn base_guard(id: &str) -> Guard {
        Guard {
            id: id.into(),
            name: "G".into(),
            resume_delay: 0.0,
            cooldown: 0.0,
            ..Default::default()
        }
    }

    #[test]
    fn plain_click_brackets_with_pause_resume() {
        let (engine, log, fires) = harness("g", vec![detection(100, 200, 20, 10)]);
        engine.test_prime(vec![base_guard("g")]);
        engine.test_tick();
        assert_eq!(
            *log.lock().unwrap(),
            vec![Action::Pause, Action::Click(100, 200), Action::Resume]
        );
        assert_eq!(*fires.lock().unwrap(), vec![("G".into(), 100, 200)]);
    }

    #[test]
    fn humanize_bezier_moves_before_clicking() {
        let (engine, log, _) = harness("g", vec![detection(50, 60, 10, 10)]);
        engine.set_humanize(true);
        engine.test_prime(vec![base_guard("g")]);
        engine.test_tick();
        assert_eq!(
            *log.lock().unwrap(),
            vec![
                Action::Pause,
                Action::BezierMoveTo(50, 60),
                Action::Click(50, 60),
                Action::Resume,
            ]
        );
    }

    #[test]
    fn keyed_guard_presses_key_not_click() {
        let (engine, log, _) = harness("g", vec![detection(0, 0, 4, 4)]);
        let mut g = base_guard("g");
        g.action = "key".into();
        g.key = "space".into();
        engine.test_prime(vec![g]);
        engine.test_tick();
        assert_eq!(
            *log.lock().unwrap(),
            vec![
                Action::Pause,
                Action::KeyPress("space".into()),
                Action::Resume,
            ]
        );
    }

    #[test]
    fn click_offset_targets_top_left_plus_offset() {
        // Centre (100,100), w=h=20 → top-left (90,90); +offset(5,7) → (95,97).
        let (engine, log, fires) = harness("g", vec![detection(100, 100, 20, 20)]);
        let mut g = base_guard("g");
        g.click_offset = vec![5, 7];
        engine.test_prime(vec![g]);
        engine.test_tick();
        assert_eq!(
            *log.lock().unwrap(),
            vec![Action::Pause, Action::Click(95, 97), Action::Resume]
        );
        assert_eq!(*fires.lock().unwrap(), vec![("G".into(), 95, 97)]);
    }

    #[test]
    fn drag_sweeps_hold_move_release() {
        // Centre (10,10), w=h=0 → top-left (10,10). Stroke [0,0,2,0] sweeps from
        // (10,10) to (12,10): points (10,10),(11,10),(12,10).
        let (engine, log, fires) = harness("g", vec![detection(10, 10, 0, 0)]);
        let mut g = base_guard("g");
        g.click_lines = vec![vec![0, 0, 2, 0]];
        engine.test_prime(vec![g]);
        engine.test_tick();
        assert_eq!(
            *log.lock().unwrap(),
            vec![
                Action::Pause,
                Action::MoveTo(10, 10),
                Action::MouseDown("left".into()),
                Action::MoveTo(11, 10),
                Action::MoveTo(12, 10),
                Action::MoveTo(12, 10), // final re-land (Python's `finally`)
                Action::MouseUp("left".into()),
                Action::Resume,
            ]
        );
        // on_fire reports the first stroke's start (top-left + [0,0]).
        assert_eq!(*fires.lock().unwrap(), vec![("G".into(), 10, 10)]);
    }

    #[test]
    fn cooldown_skips_a_guard_still_cooling() {
        let (engine, log, _) = harness("g", vec![detection(1, 2, 0, 0)]);
        let mut g = base_guard("g");
        g.cooldown = 100.0; // long — the immediate second tick is within it
        engine.test_prime(vec![g]);
        engine.test_tick();
        engine.test_tick();
        // Only the first tick fired; the second was gated out.
        assert_eq!(
            *log.lock().unwrap(),
            vec![Action::Pause, Action::Click(1, 2), Action::Resume]
        );
    }

    #[test]
    fn zero_cooldown_fires_every_tick() {
        let (engine, log, _) = harness("g", vec![detection(1, 2, 0, 0)]);
        engine.test_prime(vec![base_guard("g")]); // cooldown 0.0
        engine.test_tick();
        engine.test_tick();
        assert_eq!(log.lock().unwrap().len(), 6); // two full brackets
    }

    #[test]
    fn idle_or_paused_player_never_acts() {
        // detect would match, but the player gate short-circuits first.
        for state in [(false, false), (true, true)] {
            let log: ActionLog = Arc::new(Mutex::new(Vec::new()));
            let alog = Arc::clone(&log);
            let engine = GuardEngine::new(
                Box::new(|_: &[Guard]| {
                    let mut m = HashMap::new();
                    m.insert("g".to_string(), vec![detection(1, 1, 0, 0)]);
                    m
                }),
                Box::new(move || state),
                Box::new(move |a: Action| alog.lock().unwrap().push(a)),
                None,
            );
            engine.test_prime(vec![base_guard("g")]);
            engine.test_tick();
            assert!(log.lock().unwrap().is_empty(), "state {state:?} acted");
        }
    }

    #[test]
    fn plan_action_picks_the_right_branch() {
        let best = detection(100, 100, 20, 20); // top-left (90, 90)
        // Plain → match centre.
        assert_eq!(plan_action(&base_guard("g"), &best), Plan::Click(100, 100));
        // Offset → top-left + offset.
        let mut g = base_guard("g");
        g.click_offset = vec![3, 4];
        assert_eq!(plan_action(&g, &best), Plan::Click(93, 94));
        // Strokes → drag, and click_lines wins over click_line.
        let mut g = base_guard("g");
        g.click_line = vec![1, 1, 2, 2];
        g.click_lines = vec![vec![5, 5, 6, 6]];
        assert_eq!(
            plan_action(&g, &best),
            Plan::Drag {
                tlx: 90,
                tly: 90,
                strokes: vec![vec![5, 5, 6, 6]],
            }
        );
    }

    #[test]
    fn stroke_points_walks_pixel_by_pixel_and_skips_malformed() {
        assert_eq!(
            stroke_points(&[0, 0, 0, 2], 0, 0),
            vec![(0, 0), (0, 1), (0, 2)]
        );
        // Offsets add the top-left; a zero-length stroke still yields start+end.
        assert_eq!(stroke_points(&[1, 1, 1, 1], 10, 20), vec![(11, 21), (11, 21)]);
        // Not a 4-tuple → no points (Python's `len(ln) != 4` skip).
        assert!(stroke_points(&[1, 2, 3], 0, 0).is_empty());
        assert!(stroke_points(&[], 0, 0).is_empty());
    }

    #[test]
    fn stop_is_prompt_even_during_resume_delay() {
        // A long settle delay: a naive join would block for its full duration.
        // The interruptible sleep must let stop() return promptly instead.
        let (engine, _log, _fires) = harness("g", vec![detection(1, 1, 0, 0)]);
        let mut g = base_guard("g");
        g.resume_delay = 30.0;
        engine.start(vec![g]);
        thread::sleep(Duration::from_millis(300)); // let it enter the settle sleep
        let t0 = Instant::now();
        engine.stop();
        assert!(
            t0.elapsed() < Duration::from_secs(5),
            "stop() blocked on resume_delay"
        );
        assert!(!engine.is_running());
    }
}
