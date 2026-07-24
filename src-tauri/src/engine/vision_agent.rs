//! VisionAgent — the standalone autonomous vision loop behind the Vision panel.
//!
//! Mirrors `anime_macro/guards.py::VisionAgent`. Unlike [`super::guards`] (which
//! pauses and resumes a *playing* macro), this runs on its own: it continuously
//! scans for its triggers and, the moment one appears, performs its action and
//! optionally runs a sequence of macros — a fully vision-driven bot.
//!
//! Like every engine here it is hardware-free. Detection, actuation, the UI
//! event feed, and the macro runner are injected closures, so the loop runs and
//! tests without real capture, input, or playback. There is deliberately no
//! cooldown: a trigger fires every cycle it stays on screen (Python's comment,
//! preserved), so the count reflects raw fire cycles.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::hardware::vision::Detection;
use crate::models::guard::Guard;

/// 20Hz — the capture backend tops out near 60fps, so polling faster just
/// re-detects the same frame; 50ms catches any trigger imperceptibly fast
/// (matches Python's `POLL_INTERVAL`).
const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// The action a fired trigger performs on screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VisionAction {
    KeyPress(String),
    Click(i64, i64),
}

/// `detect(enabled_triggers) -> {trigger_id: [detections]}` — one batched grab.
pub type Detect = Box<dyn Fn(&[Guard]) -> HashMap<String, Vec<Detection>> + Send + Sync>;
/// `act(action)` — perform the trigger's on-screen action.
pub type Act = Box<dyn Fn(VisionAction) + Send + Sync>;
/// `on_event(kind, message)` — feed a line to the Vision panel (`kind` is one of
/// `"start"`, `"act"`, `"stop"`).
pub type OnEvent = Box<dyn Fn(&str, &str) + Send + Sync>;
/// `run_macro(name, repeat) -> Result<(), reason>` — run one orchestrated macro.
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
        self.inner.fired.store(0, Ordering::SeqCst);
        self.inner.running.store(true, Ordering::SeqCst);
        let inner = Arc::clone(&self.inner);
        *thread = Some(thread::spawn(move || inner.run_loop()));
        (self.inner.on_event)(
            "start",
            &format!("Vision running — watching for {count} trigger(s)"),
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

impl Inner {
    fn run_loop(self: Arc<Self>) {
        while self.running.load(Ordering::SeqCst) {
            self.tick();
            thread::sleep(POLL_INTERVAL);
        }
    }

    /// One poll cycle: detect every enabled trigger in a single batch, then act
    /// on each match in order.
    fn tick(&self) {
        let enabled: Vec<Guard> = self
            .triggers
            .lock()
            .unwrap()
            .iter()
            .filter(|t| t.enabled)
            .cloned()
            .collect();
        if enabled.is_empty() {
            return;
        }
        let detections = (self.detect)(&enabled);
        for trigger in &enabled {
            // Don't begin a new action after stop() was requested.
            if !self.running.load(Ordering::SeqCst) {
                break;
            }
            let matched = detections
                .get(&trigger.id)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            if let Some(best) = matched.first() {
                self.act_on(trigger, best.x, best.y);
            }
        }
    }

    /// Perform the trigger's action, then run its macro sequence (Python's
    /// `_act`). Each fire increments the count; a keyed trigger presses its key,
    /// otherwise it clicks the match. Blank-named steps are skipped and a failing
    /// macro reports its reason to the feed without aborting the rest.
    fn act_on(&self, trigger: &Guard, x: i64, y: i64) {
        self.fired.fetch_add(1, Ordering::SeqCst);

        if trigger.action == "key" && !trigger.key.is_empty() {
            (self.act)(VisionAction::KeyPress(trigger.key.clone()));
            (self.on_event)("act", &format!("'{}' -> pressed {}", trigger.name, trigger.key));
        } else {
            (self.act)(VisionAction::Click(x, y));
            (self.on_event)("act", &format!("'{}' -> clicked ({x}, {y})", trigger.name));
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
}

#[cfg(test)]
impl VisionAgent {
    /// Load triggers and arm the agent without spawning the poll thread, so a
    /// test can drive [`Inner::tick`] deterministically.
    fn test_prime(&self, triggers: Vec<Guard>) {
        *self.inner.triggers.lock().unwrap() = triggers;
        self.inner.running.store(true, Ordering::SeqCst);
    }

    fn test_tick(&self) {
        self.inner.tick();
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

    fn base_trigger(id: &str) -> Guard {
        Guard {
            id: id.into(),
            name: "Watcher".into(),
            ..Default::default()
        }
    }

    #[test]
    fn click_trigger_acts_and_counts() {
        let (agent, actions, events) = harness("t", vec![detection(7, 9)]);
        agent.test_prime(vec![base_trigger("t")]);
        agent.test_tick();
        assert_eq!(*actions.lock().unwrap(), vec![VisionAction::Click(7, 9)]);
        assert_eq!(agent.fired_count(), 1);
        assert_eq!(
            *events.lock().unwrap(),
            vec![("act".into(), "'Watcher' -> clicked (7, 9)".into())]
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
            vec![VisionAction::KeyPress("e".into())]
        );
        assert_eq!(
            *events.lock().unwrap(),
            vec![("act".into(), "'Watcher' -> pressed e".into())]
        );
    }

    #[test]
    fn no_cooldown_fires_every_tick() {
        let (agent, actions, _) = harness("t", vec![detection(1, 1)]);
        agent.test_prime(vec![base_trigger("t")]);
        agent.test_tick();
        agent.test_tick();
        assert_eq!(
            *actions.lock().unwrap(),
            vec![VisionAction::Click(1, 1), VisionAction::Click(1, 1)]
        );
        assert_eq!(agent.fired_count(), 2);
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
            MacroSeqItem { name: "grind".into(), repeat: 3 }, // finite → "x3"
            MacroSeqItem { name: "".into(), repeat: 1 },      // blank → skipped
            MacroSeqItem { name: "idle".into(), repeat: 0 },  // 0 → "xinf"
            MacroSeqItem { name: "boom".into(), repeat: 5 },  // fails
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
                ("start".into(), "Vision running — watching for 1 trigger(s)".into()),
                ("stop".into(), "Vision stopped".into()),
            ]
        );
    }
}
