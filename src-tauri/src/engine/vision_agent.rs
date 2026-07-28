//! VisionAgent: the standalone autonomous vision loop behind the Vision panel.
//!
//! Mirrors `anime_macro/guards.py::VisionAgent`. Unlike [`super::guards`] (which
//! pauses and resumes a *playing* macro), this runs on its own: it continuously
//! scans for its triggers and, the moment one appears, performs its action and
//! optionally runs a sequence of macros, a fully vision-driven bot.
//!
//! Like every engine here it is hardware-free. Detection, actuation, the UI
//! event feed, and the macro runner are injected closures, so the loop runs and
//! tests without real capture, input, or playback.
//!
//! Each trigger is scheduled on its own clock rather than the whole set sharing
//! one fixed cadence. Python polled everything at a flat 20Hz, which is fine for
//! a colour test and ruinous for a text one: reading the screen runs the OS
//! recognizer, and asking for that twenty times a second pins a CPU. Two rules
//! replace the fixed interval:
//!
//! * **Idle triggers pay for themselves.** After a pass that found nothing, a
//!   trigger waits in proportion to what its own detection just cost, so the
//!   watcher never spends more than [`IDLE_DUTY`] of a core hunting. Cheap
//!   checks stay effectively as fast as before; expensive ones back off on
//!   their own, on whatever machine they land on, with no tuning to guess at.
//! * **A trigger that just fired runs at its own `cooldown`,** with no duty
//!   floor at all. At the default of zero that means straight back round on the
//!   next frame: collecting a thing that keeps reappearing goes as fast as the
//!   hardware allows, which is the whole point of the watcher.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::hardware::vision::Detection;
use crate::models::guard::Guard;

use super::guards::{plan_action, stroke_points, Plan};
use super::sleep_interruptible;

/// The share of one core the watcher may spend while nothing is on screen.
/// A pass costing `t` is followed by `t * (1/IDLE_DUTY - 1)` of quiet.
const IDLE_DUTY: f64 = 0.2;

/// Floor on that quiet time, the old fixed cadence. A detection cheap enough to
/// want more than 20Hz is re-running on stale capture anyway, since the backend
/// tops out near 60fps.
const MIN_IDLE: Duration = Duration::from_millis(50);

/// Ceiling on it, so even the priciest detector is retried inside a second and a
/// half rather than disappearing for as long as the duty rule would like.
const MAX_IDLE: Duration = Duration::from_millis(1500);

/// How long to wait when there is nothing enabled to look at.
const EMPTY_IDLE: Duration = Duration::from_millis(200);

/// Floor on the wait after a trigger fires, roughly one display frame.
///
/// A zero cooldown means "collect it as fast as the hardware allows", and the
/// hardware here is the capture, which tops out near 60fps. Looking again
/// sooner than that re-runs the whole detector over pixels that have not
/// changed, so the burst is paced to the frame rather than to nothing. It costs
/// no fires and it is the difference between a busy core and a pinned one.
const BURST_FLOOR: Duration = Duration::from_millis(16);

/// Dwell between the moves of a drag sweep, so a game registers it as a
/// continuous drag rather than a teleport. Matches [`super::guards`].
const STROKE_STEP: Duration = Duration::from_millis(8);
/// Give a game one frame to observe the cursor at the target before pressing.
/// Roblox can paint the hover state while still dropping a button-down sent in
/// the same input burst as the move.
const CLICK_SETTLE: Duration = Duration::from_millis(32);
/// Keep the button down across multiple game ticks. A zero-duration down/up is
/// legal Win32 input, but some games coalesce it before their UI sees a press.
const CLICK_HOLD: Duration = Duration::from_millis(48);
/// A real key press spans more than one game tick for the same reason as a
/// mouse press. This is deliberately scoped to autonomous Watch actions;
/// recorded macros keep their recorded timing.
const KEY_HOLD: Duration = Duration::from_millis(48);

/// The action a fired trigger performs on screen. The three cursor primitives
/// exist for the same reason they do in [`super::guards`]: a trigger whose
/// picture was marked with drawn strokes is a drag, not a click, and the sweep
/// is driven from here so it stays testable without real input. `Nudge` reuses
/// the same target a click would press, but parks the cursor there and wiggles
/// it, so a "nudge it" trigger keeps the game seeing activity at the object
/// instead of clicking on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VisionAction {
    KeyDown(String),
    KeyUp(String),
    Nudge(i64, i64),
    MoveTo(i64, i64),
    MouseDown(String),
    MouseUp(String),
    Pause(Duration),
}

/// `detect(enabled_triggers) -> {trigger_id: [detections]}`: one batched grab.
pub type Detect = Box<dyn Fn(&[Guard]) -> HashMap<String, Vec<Detection>> + Send + Sync>;
/// `act(action)`: perform the trigger's on-screen action.
pub type Act = Box<dyn Fn(VisionAction) + Send + Sync>;
/// `on_event(kind, message)`: feed a line to the Vision panel (`kind` is one of
/// `"start"`, `"act"`, `"stop"`).
pub type OnEvent = Box<dyn Fn(&str, &str) + Send + Sync>;
/// `run_macro(name, repeat) -> Result<(), reason>`: run one orchestrated macro.
/// `repeat` of 0 means run forever, so a faithful implementation MUST be
/// interruptible; the engine already breaks the step loop on `stop()`, but a
/// single infinite step can only be cut short by the runner itself. `Err`
/// carries the failure reason for the event feed (Python's `except … as e`).
pub type MacroRunner = Box<dyn Fn(&str, i64) -> Result<(), String> + Send + Sync>;

struct Inner {
    detect: Detect,
    act: Act,
    on_event: OnEvent,
    run_macro: MacroRunner,
    triggers: Mutex<Vec<Guard>>,
    running: AtomicBool,
    /// Total fire cycles since `start`; read cross-thread by the UI, hence atomic.
    fired: AtomicI64,
    /// Trigger id → when it is next due to be looked for. Entries for triggers
    /// that have gone away are dropped each pass, so an edited list cannot leak.
    due: Mutex<HashMap<String, Instant>>,
}

pub struct VisionAgent {
    inner: Arc<Inner>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl VisionAgent {
    pub fn new(detect: Detect, act: Act, on_event: OnEvent, run_macro: MacroRunner) -> Self {
        Self {
            inner: Arc::new(Inner {
                detect,
                act,
                on_event,
                run_macro,
                triggers: Mutex::new(Vec::new()),
                running: AtomicBool::new(false),
                fired: AtomicI64::new(0),
                due: Mutex::new(HashMap::new()),
            }),
            thread: Mutex::new(None),
        }
    }

    /// Start watching the given triggers. Filters to enabled ones and is a no-op
    /// if none are enabled or the agent is already running (matches Python).
    pub fn start(&self, triggers: Vec<Guard>) {
        let enabled: Vec<Guard> = triggers.into_iter().filter(|t| t.enabled).collect();
        if enabled.is_empty() {
            return;
        }
        let mut thread = self.thread.lock().unwrap();
        if thread.is_some() {
            return;
        }
        let count = enabled.len();
        *self.inner.triggers.lock().unwrap() = enabled;
        self.inner.due.lock().unwrap().clear();
        self.inner.fired.store(0, Ordering::SeqCst);
        self.inner.running.store(true, Ordering::SeqCst);
        let inner = Arc::clone(&self.inner);
        *thread = Some(thread::spawn(move || inner.run_loop()));
        (self.inner.on_event)(
            "start",
            &format!("Vision running, watching for {count} trigger(s)"),
        );
    }

    pub fn stop(&self) {
        self.inner.running.store(false, Ordering::SeqCst);
        if let Some(handle) = self.thread.lock().unwrap().take() {
            let _ = handle.join();
        }
        (self.inner.on_event)("stop", "Vision stopped");
    }

    pub fn is_running(&self) -> bool {
        self.inner.running.load(Ordering::SeqCst)
    }

    pub fn fired_count(&self) -> i64 {
        self.inner.fired.load(Ordering::SeqCst)
    }
}

/// The quiet time a pass costing `cost` has earned itself, per [`IDLE_DUTY`].
fn idle_after(cost: Duration) -> Duration {
    cost.mul_f64(1.0 / IDLE_DUTY - 1.0)
        .clamp(MIN_IDLE, MAX_IDLE)
}

impl Inner {
    fn run_loop(self: Arc<Self>) {
        while self.running.load(Ordering::SeqCst) {
            let wait = self.tick();
            if !wait.is_zero() {
                sleep_interruptible(&self.running, wait);
            }
        }
    }

    /// One poll cycle: detect the triggers that are due in a single batch, act
    /// on each match in order, reschedule everything that was looked at, and
    /// report how long the caller should wait before asking again.
    ///
    /// A wait of [`BURST_FLOOR`] is the burst case: some trigger fired and its
    /// cooldown is zero, so the next look is a frame away rather than a poll
    /// interval away.
    fn tick(&self) -> Duration {
        let enabled: Vec<Guard> = self
            .triggers
            .lock()
            .unwrap()
            .iter()
            .filter(|t| t.enabled)
            .cloned()
            .collect();
        if enabled.is_empty() {
            self.due.lock().unwrap().clear();
            return EMPTY_IDLE;
        }

        let now = Instant::now();
        let (ready, soonest) = {
            // Forget triggers that have gone away, then split the rest into the
            // ones due now and the soonest of those that are not.
            let mut due = self.due.lock().unwrap();
            due.retain(|id, _| enabled.iter().any(|t| &t.id == id));
            let mut ready = Vec::new();
            let mut soonest: Option<Instant> = None;
            for trigger in &enabled {
                match due.get(&trigger.id) {
                    Some(&at) if at > now => {
                        soonest = Some(soonest.map_or(at, |s: Instant| s.min(at)));
                    }
                    _ => ready.push(trigger.clone()),
                }
            }
            (ready, soonest)
        };
        if ready.is_empty() {
            return soonest.map_or(MIN_IDLE, |at| at.saturating_duration_since(now));
        }

        let started = Instant::now();
        let detections = (self.detect)(&ready);
        let idle = idle_after(started.elapsed());

        let mut next = Vec::with_capacity(ready.len());
        for trigger in &ready {
            // Don't begin a new action after stop() was requested.
            if !self.running.load(Ordering::SeqCst) {
                break;
            }
            let matched = detections
                .get(&trigger.id)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            match matched.first() {
                Some(best) => {
                    self.act_on(trigger, best);
                    // Measured after acting: a trigger that runs a macro should
                    // wait its cooldown from the end of that, not the start.
                    next.push((
                        trigger.id.clone(),
                        Duration::from_secs_f64(trigger.cooldown.max(0.0)).max(BURST_FLOOR),
                    ));
                }
                None => next.push((trigger.id.clone(), idle)),
            }
        }

        // Whoever comes up first sets the wait, including the triggers that
        // were not due this pass and so were never looked at.
        let settled = Instant::now();
        let mut due = self.due.lock().unwrap();
        for (id, gap) in next {
            due.insert(id, settled + gap);
        }
        due.values()
            .map(|&at| at.saturating_duration_since(settled))
            .min()
            .unwrap_or(MIN_IDLE)
    }

    /// Perform the trigger's action, then run its macro sequence (Python's
    /// `_act`). Each fire increments the count; a keyed trigger presses its key,
    /// a nudging one moves onto the match and wiggles the cursor there, and a
    /// clicking one follows the same geometric plan a guard would. Blank-named
    /// steps are skipped and a failing macro reports its reason to the feed
    /// without aborting the rest.
    ///
    /// The plan matters more here than it does for guards, because the Watch
    /// surface is the only one that offers "snap and mark click": the picture is
    /// the whole element and the marked point is where inside it to press. Acting
    /// on the match centre instead lands wherever the middle of that picture
    /// happens to be, which for anything but a plain rectangular button is not
    /// the target at all.
    fn act_on(&self, trigger: &Guard, best: &Detection) {
        self.fired.fetch_add(1, Ordering::SeqCst);

        if trigger.action == "key" {
            let key = trigger.key.trim();
            if key.is_empty() {
                (self.on_event)(
                    "error",
                    &format!("'{}' -> no key is configured", trigger.name),
                );
                return;
            }
            (self.act)(VisionAction::KeyDown(key.to_string()));
            (self.act)(VisionAction::Pause(KEY_HOLD));
            (self.act)(VisionAction::KeyUp(key.to_string()));
            (self.on_event)("act", &format!("'{}' -> pressed {key}", trigger.name));
        } else if trigger.action == "nudge" {
            // Same target a click would press (a marked point or offset applied
            // to the match), but parked-and-wiggled instead of pressed: the game
            // sees activity at the object, not a click on it.
            let (x, y) = nudge_point(trigger, best);
            (self.act)(VisionAction::Nudge(x, y));
            (self.on_event)(
                "act",
                &format!("'{}' -> nudged the mouse at ({x}, {y})", trigger.name),
            );
        } else {
            match plan_action(trigger, best) {
                Plan::Click(x, y) => {
                    // Emit the press as explicit phases. Besides making the
                    // behavior testable, this prevents games from collapsing a
                    // move/down/up burst into a hover with no registered click.
                    (self.act)(VisionAction::MoveTo(x, y));
                    (self.act)(VisionAction::Pause(CLICK_SETTLE));
                    (self.act)(VisionAction::MouseDown("left".to_string()));
                    (self.act)(VisionAction::Pause(CLICK_HOLD));
                    (self.act)(VisionAction::MouseUp("left".to_string()));
                    (self.on_event)("act", &format!("'{}' -> clicked ({x}, {y})", trigger.name));
                }
                Plan::Drag { tlx, tly, strokes } => {
                    self.sweep(tlx, tly, &strokes);
                    (self.on_event)(
                        "act",
                        &format!("'{}' -> dragged {} stroke(s)", trigger.name, strokes.len()),
                    );
                }
            }
        }

        if trigger.macro_sequence.is_empty() {
            return;
        }
        (self.on_event)(
            "act",
            &format!(
                "'{}' -> running {} macro(s)",
                trigger.name,
                trigger.macro_sequence.len()
            ),
        );
        for step in &trigger.macro_sequence {
            if step.name.is_empty() {
                continue;
            }
            // A long-running step (repeat 0 = forever) would otherwise strand
            // stop(); bail before starting another once shutdown is requested.
            if !self.running.load(Ordering::SeqCst) {
                break;
            }
            match (self.run_macro)(&step.name, step.repeat) {
                Ok(()) => {
                    let reps = if step.repeat > 0 {
                        step.repeat.to_string()
                    } else {
                        "inf".to_string()
                    };
                    (self.on_event)("act", &format!("  -> ran '{}' (x{reps})", step.name));
                }
                Err(reason) => {
                    (self.on_event)("act", &format!("  -> '{}' failed: {reason}", step.name));
                }
            }
        }
    }

    /// Drag along each stroke: press at the start, sweep pixel by pixel, release.
    /// The release always runs, even if `stop()` cuts the sweep short, so a held
    /// button is never stranded.
    fn sweep(&self, tlx: i64, tly: i64, strokes: &[Vec<i64>]) {
        for stroke in strokes {
            let pts = stroke_points(stroke, tlx, tly);
            if pts.is_empty() {
                continue;
            }
            let (startx, starty) = pts[0];
            let (endx, endy) = *pts.last().unwrap();
            (self.act)(VisionAction::MoveTo(startx, starty));
            (self.act)(VisionAction::MouseDown("left".to_string()));
            for &(px, py) in &pts[1..] {
                if !self.running.load(Ordering::SeqCst) {
                    break;
                }
                (self.act)(VisionAction::MoveTo(px, py));
                thread::sleep(STROKE_STEP);
            }
            (self.act)(VisionAction::MoveTo(endx, endy));
            (self.act)(VisionAction::MouseUp("left".to_string()));
        }
    }
}

/// The point a nudge parks on: exactly where a click would land (a marked point
/// or offset applied to the match), so "nudge it" means "go to the thing and
/// wiggle there". A drawn stroke has no single press point for a nudge to use,
/// so it falls back to the stroke's start, then to the match's top-left.
fn nudge_point(trigger: &Guard, best: &Detection) -> (i64, i64) {
    match plan_action(trigger, best) {
        Plan::Click(x, y) => (x, y),
        Plan::Drag { tlx, tly, strokes } => strokes
            .first()
            .and_then(|s| stroke_points(s, tlx, tly).first().copied())
            .unwrap_or((tlx, tly)),
    }
}

#[cfg(test)]
impl VisionAgent {
    /// Load triggers and arm the agent without spawning the poll thread, so a
    /// test can drive [`Inner::tick`] deterministically.
    fn test_prime(&self, triggers: Vec<Guard>) {
        *self.inner.triggers.lock().unwrap() = triggers;
        self.inner.running.store(true, Ordering::SeqCst);
    }

    fn test_tick(&self) -> Duration {
        self.inner.tick()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::guard::MacroSeqItem;
    use std::time::Instant;

    fn detection(x: i64, y: i64) -> Detection {
        Detection {
            label: "m".into(),
            x,
            y,
            w: 0,
            h: 0,
            confidence: 1.0,
            roi_offset: [0, 0],
        }
    }

    type ActionLog = Arc<Mutex<Vec<VisionAction>>>;
    type EventLog = Arc<Mutex<Vec<(String, String)>>>;

    /// An agent whose detect always returns `dets` for trigger `id`, with the
    /// given macro runner, logging actions and events to shared vectors.
    fn make(
        id: &str,
        dets: Vec<Detection>,
        run_macro: MacroRunner,
    ) -> (VisionAgent, ActionLog, EventLog) {
        let actions: ActionLog = Arc::new(Mutex::new(Vec::new()));
        let events: EventLog = Arc::new(Mutex::new(Vec::new()));
        let (alog, elog, id) = (Arc::clone(&actions), Arc::clone(&events), id.to_string());
        let agent = VisionAgent::new(
            Box::new(move |_: &[Guard]| {
                let mut m = HashMap::new();
                m.insert(id.clone(), dets.clone());
                m
            }),
            Box::new(move |a: VisionAction| alog.lock().unwrap().push(a)),
            Box::new(move |k: &str, msg: &str| {
                elog.lock().unwrap().push((k.to_string(), msg.to_string()))
            }),
            run_macro,
        );
        (agent, actions, events)
    }

    /// The common case: a runner that succeeds for every macro.
    fn harness(id: &str, dets: Vec<Detection>) -> (VisionAgent, ActionLog, EventLog) {
        make(id, dets, Box::new(|_: &str, _: i64| Ok(())))
    }

    /// A watch trigger at the Watch view's own default: no cooldown, so it fires
    /// every cycle the thing is on screen.
    fn base_trigger(id: &str) -> Guard {
        Guard {
            id: id.into(),
            name: "Watcher".into(),
            cooldown: 0.0,
            ..Default::default()
        }
    }

    fn click_actions(x: i64, y: i64) -> Vec<VisionAction> {
        vec![
            VisionAction::MoveTo(x, y),
            VisionAction::Pause(CLICK_SETTLE),
            VisionAction::MouseDown("left".into()),
            VisionAction::Pause(CLICK_HOLD),
            VisionAction::MouseUp("left".into()),
        ]
    }

    #[test]
    fn click_trigger_acts_and_counts() {
        let (agent, actions, events) = harness("t", vec![detection(7, 9)]);
        agent.test_prime(vec![base_trigger("t")]);
        agent.test_tick();
        assert_eq!(*actions.lock().unwrap(), click_actions(7, 9));
        assert_eq!(agent.fired_count(), 1);
        assert_eq!(
            *events.lock().unwrap(),
            vec![("act".into(), "'Watcher' -> clicked (7, 9)".into())]
        );
    }

    /// A match with real extent, so the top-left the surgical offsets are
    /// measured from is somewhere other than the centre.
    fn sized_detection(x: i64, y: i64, w: i64, h: i64) -> Detection {
        Detection {
            w,
            h,
            ..detection(x, y)
        }
    }

    #[test]
    fn a_marked_click_point_is_where_it_clicks() {
        // "Snap and mark click" only exists on the Watch surface, so this is the
        // one engine that has to honour it. Centre of the 40x20 match is (100,
        // 200), top-left (80, 190), and the mark was 4 px in and 3 px down.
        let (agent, actions, _) = harness("t", vec![sized_detection(100, 200, 40, 20)]);
        let mut t = base_trigger("t");
        t.click_offset = vec![4, 3];
        agent.test_prime(vec![t]);
        agent.test_tick();
        assert_eq!(*actions.lock().unwrap(), click_actions(84, 193));
    }

    #[test]
    fn a_marked_stroke_drags_across_the_match() {
        let (agent, actions, events) = harness("t", vec![sized_detection(100, 200, 40, 20)]);
        let mut t = base_trigger("t");
        t.click_lines = vec![vec![0, 0, 2, 0]];
        agent.test_prime(vec![t]);
        agent.test_tick();
        assert_eq!(
            *actions.lock().unwrap(),
            vec![
                VisionAction::MoveTo(80, 190),
                VisionAction::MouseDown("left".into()),
                VisionAction::MoveTo(81, 190),
                VisionAction::MoveTo(82, 190),
                VisionAction::MoveTo(82, 190),
                VisionAction::MouseUp("left".into()),
            ]
        );
        assert_eq!(
            *events.lock().unwrap(),
            vec![("act".into(), "'Watcher' -> dragged 1 stroke(s)".into())]
        );
    }

    #[test]
    fn key_trigger_presses_key() {
        let (agent, actions, events) = harness("t", vec![detection(0, 0)]);
        let mut t = base_trigger("t");
        t.action = "key".into();
        t.key = "e".into();
        agent.test_prime(vec![t]);
        agent.test_tick();
        assert_eq!(
            *actions.lock().unwrap(),
            vec![
                VisionAction::KeyDown("e".into()),
                VisionAction::Pause(KEY_HOLD),
                VisionAction::KeyUp("e".into()),
            ]
        );
        assert_eq!(
            *events.lock().unwrap(),
            vec![("act".into(), "'Watcher' -> pressed e".into())]
        );
    }

    #[test]
    fn key_trigger_without_a_key_never_falls_back_to_clicking() {
        let (agent, actions, events) = harness("t", vec![detection(50, 60)]);
        let mut t = base_trigger("t");
        t.action = "key".into();
        t.key = "   ".into();
        agent.test_prime(vec![t]);
        agent.test_tick();
        assert!(actions.lock().unwrap().is_empty());
        assert_eq!(
            *events.lock().unwrap(),
            vec![("error".into(), "'Watcher' -> no key is configured".into())]
        );
    }

    #[test]
    fn nudge_trigger_wiggles_without_pressing() {
        // Nudge parks on the match — the same point a click would use — and
        // wiggles there instead of pressing, so the detection at (300, 400)
        // becomes a nudge at (300, 400), never a click.
        let (agent, actions, events) = harness("t", vec![detection(300, 400)]);
        let mut t = base_trigger("t");
        t.action = "nudge".into();
        agent.test_prime(vec![t]);
        agent.test_tick();
        assert_eq!(
            *actions.lock().unwrap(),
            vec![VisionAction::Nudge(300, 400)]
        );
        assert_eq!(
            *events.lock().unwrap(),
            vec![(
                "act".into(),
                "'Watcher' -> nudged the mouse at (300, 400)".into()
            )]
        );
    }

    #[test]
    fn nudge_parks_on_the_marked_point_like_a_click_would() {
        // "Exactly like click but it nudges": a marked offset resolves to the
        // same point the click test gets (84, 193), then wiggles instead of
        // presses — proving nudge shares the click geometry verbatim.
        let (agent, actions, _) = harness("t", vec![sized_detection(100, 200, 40, 20)]);
        let mut t = base_trigger("t");
        t.action = "nudge".into();
        t.click_offset = vec![4, 3];
        agent.test_prime(vec![t]);
        agent.test_tick();
        assert_eq!(*actions.lock().unwrap(), vec![VisionAction::Nudge(84, 193)]);
    }

    #[test]
    fn no_cooldown_fires_every_frame() {
        // And asks only for the frame floor between them: "collect it as fast
        // as the hardware allows" is what a zero cooldown means, and the
        // hardware is the capture. The sleep is what the run loop does with the
        // returned wait, so ticking straight through it would test a loop that
        // does not exist.
        let (agent, actions, _) = harness("t", vec![detection(1, 1)]);
        agent.test_prime(vec![base_trigger("t")]);
        assert_eq!(agent.test_tick(), BURST_FLOOR);
        thread::sleep(BURST_FLOOR);
        agent.test_tick();
        let mut expected = click_actions(1, 1);
        expected.extend(click_actions(1, 1));
        assert_eq!(*actions.lock().unwrap(), expected);
        assert_eq!(agent.fired_count(), 2);
    }

    #[test]
    fn a_burst_trigger_is_not_looked_at_again_inside_the_frame() {
        // The other half of the floor: a second look in the same frame reads
        // pixels that have not changed, which is the whole cost with none of
        // the benefit, so it is skipped rather than merely wasted.
        let (agent, actions, _) = harness("t", vec![detection(1, 1)]);
        agent.test_prime(vec![base_trigger("t")]);
        agent.test_tick();
        agent.test_tick();
        assert_eq!(*actions.lock().unwrap(), click_actions(1, 1));
        assert_eq!(agent.fired_count(), 1);
    }

    #[test]
    fn a_cooldown_holds_a_trigger_back() {
        let (agent, actions, _) = harness("t", vec![detection(1, 1)]);
        let mut t = base_trigger("t");
        t.cooldown = 30.0;
        agent.test_prime(vec![t]);
        let wait = agent.test_tick();
        // Fired once, and the next look is a cooldown away rather than now.
        agent.test_tick();
        assert_eq!(*actions.lock().unwrap(), click_actions(1, 1));
        assert!(wait > Duration::from_secs(25), "asked to wait {wait:?}");
    }

    #[test]
    fn a_trigger_that_finds_nothing_backs_off_by_what_it_cost() {
        // The floor applies to a free detection; a slow one is charged the duty
        // rate, and neither is ever asked to disappear for longer than the cap.
        assert_eq!(idle_after(Duration::ZERO), MIN_IDLE);
        assert_eq!(
            idle_after(Duration::from_millis(100)),
            Duration::from_millis(400)
        );
        assert_eq!(idle_after(Duration::from_secs(9)), MAX_IDLE);

        let (agent, actions, _) = harness("other", vec![detection(1, 1)]);
        agent.test_prime(vec![base_trigger("t")]);
        let wait = agent.test_tick();
        assert!(actions.lock().unwrap().is_empty());
        assert!(wait >= MIN_IDLE, "asked to wait {wait:?}");
    }

    #[test]
    fn macro_sequence_runs_each_step_and_reports_failures() {
        // "boom" is the only unknown macro; every other name runs.
        let runner: MacroRunner = Box::new(|name: &str, _repeat: i64| {
            if name == "boom" {
                Err("no such macro".into())
            } else {
                Ok(())
            }
        });
        let (agent, _actions, events) = make("t", vec![detection(3, 4)], runner);
        let mut t = base_trigger("t");
        t.macro_sequence = vec![
            MacroSeqItem {
                name: "grind".into(),
                repeat: 3,
            }, // finite → "x3"
            MacroSeqItem {
                name: "".into(),
                repeat: 1,
            }, // blank → skipped
            MacroSeqItem {
                name: "idle".into(),
                repeat: 0,
            }, // 0 → "xinf"
            MacroSeqItem {
                name: "boom".into(),
                repeat: 5,
            }, // fails
        ];
        agent.test_prime(vec![t]);
        agent.test_tick();

        assert_eq!(
            *events.lock().unwrap(),
            vec![
                ("act".into(), "'Watcher' -> clicked (3, 4)".into()),
                ("act".into(), "'Watcher' -> running 4 macro(s)".into()),
                ("act".into(), "  -> ran 'grind' (x3)".into()),
                ("act".into(), "  -> ran 'idle' (xinf)".into()),
                ("act".into(), "  -> 'boom' failed: no such macro".into()),
            ]
        );
        assert_eq!(agent.fired_count(), 1);
    }

    #[test]
    fn disabled_trigger_is_ignored() {
        // The per-tick enabled filter drops it even if detect would match.
        let (agent, actions, events) = harness("t", vec![detection(1, 1)]);
        let mut t = base_trigger("t");
        t.enabled = false;
        agent.test_prime(vec![t]);
        agent.test_tick();
        assert!(actions.lock().unwrap().is_empty());
        assert!(events.lock().unwrap().is_empty());
        assert_eq!(agent.fired_count(), 0);
    }

    #[test]
    fn start_then_stop_terminates() {
        // Empty detect → the loop just polls; verify the lifecycle events and a
        // prompt, blocking-free shutdown.
        let events: EventLog = Arc::new(Mutex::new(Vec::new()));
        let elog = Arc::clone(&events);
        let agent = VisionAgent::new(
            Box::new(|_: &[Guard]| HashMap::new()),
            Box::new(|_: VisionAction| {}),
            Box::new(move |k: &str, msg: &str| {
                elog.lock().unwrap().push((k.to_string(), msg.to_string()))
            }),
            Box::new(|_: &str, _: i64| Ok(())),
        );
        agent.start(vec![base_trigger("t")]);
        assert!(agent.is_running());
        thread::sleep(Duration::from_millis(120));
        let t0 = Instant::now();
        agent.stop();
        assert!(t0.elapsed() < Duration::from_secs(1), "stop() blocked");
        assert!(!agent.is_running());
        assert_eq!(agent.fired_count(), 0);
        assert_eq!(
            *events.lock().unwrap(),
            vec![
                (
                    "start".into(),
                    "Vision running, watching for 1 trigger(s)".into()
                ),
                ("stop".into(), "Vision stopped".into()),
            ]
        );
    }
}
