//! Macro scheduler engine: fire macros/chains on an interval or at a set time.
//!
//! Mirrors `anime_macro/scheduler.py::MacroScheduler`. One background thread ticks
//! once a second; when a schedule is due and the app is idle it fires the target
//! via an injected callback (`fire` for macros, `fire_chain` for chains). Due
//! schedules are skipped, not queued, while the app is busy. Schedules persist to
//! `config/schedules.json`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::models::schedule::{self, Schedule};
use crate::util::{now_epoch, now_millis};

/// `fire(macro_name, repeat) -> started_ok`.
pub type Fire = Box<dyn Fn(&str, i64) -> bool + Send + Sync>;
/// `fire_chain(chain_id) -> started_ok`.
pub type FireChain = Box<dyn Fn(&str) -> bool + Send + Sync>;
/// `is_idle() -> can_start`.
pub type IsIdle = Box<dyn Fn() -> bool + Send + Sync>;

/// Fields needed to fire one due schedule, snapshotted under the lock so the
/// callback runs without holding it.
struct Due {
    id: String,
    kind: String,
    target_type: String,
    target_id: String,
    macro_name: String,
    repeat: i64,
}

struct Inner {
    store_path: PathBuf,
    is_idle: IsIdle,
    fire: Fire,
    fire_chain: Option<FireChain>,
    schedules: Mutex<Vec<Schedule>>,
    running: AtomicBool,
}

pub struct MacroScheduler {
    inner: Arc<Inner>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl MacroScheduler {
    pub fn new(
        store_path: impl Into<PathBuf>,
        is_idle: IsIdle,
        fire: Fire,
        fire_chain: Option<FireChain>,
    ) -> Self {
        let store_path = store_path.into();
        let schedules = schedule::load(&store_path)
            .into_iter()
            .filter(|s| !s.id.is_empty())
            .collect();
        Self {
            inner: Arc::new(Inner {
                store_path,
                is_idle,
                fire,
                fire_chain,
                schedules: Mutex::new(schedules),
                running: AtomicBool::new(false),
            }),
            thread: Mutex::new(None),
        }
    }

    // ── CRUD ────────────────────────────────────────────────────────────────

    pub fn list(&self) -> Vec<Schedule> {
        self.inner.schedules.lock().unwrap().clone()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn add(
        &self,
        macro_name: &str,
        kind: &str,
        interval_min: f64,
        at_time: &str,
        repeat: i64,
        enabled: bool,
        target_type: &str,
        target_id: &str,
    ) -> Schedule {
        let mut schedules = self.inner.schedules.lock().unwrap();
        let s = Schedule {
            id: format!("sched_{}", now_millis()),
            macro_name: macro_name.to_string(),
            kind: kind.to_string(),
            interval_min: interval_min.max(0.1),
            at_time: at_time.to_string(),
            enabled,
            last_run: 0.0,
            repeat,
            target_type: target_type.to_string(),
            target_id: target_id.to_string(),
        };
        schedules.push(s.clone());
        self.inner.save(&schedules);
        s
    }

    pub fn remove(&self, sid: &str) -> bool {
        let mut schedules = self.inner.schedules.lock().unwrap();
        let before = schedules.len();
        schedules.retain(|s| s.id != sid);
        if schedules.len() != before {
            self.inner.save(&schedules);
            true
        } else {
            false
        }
    }

    pub fn set_enabled(&self, sid: &str, enabled: bool) -> bool {
        let mut schedules = self.inner.schedules.lock().unwrap();
        let Some(s) = schedules.iter_mut().find(|s| s.id == sid) else {
            return false;
        };
        s.enabled = enabled;
        self.inner.save(&schedules);
        true
    }

    // ── Engine ──────────────────────────────────────────────────────────────

    pub fn start(&self) {
        let mut handle = self.thread.lock().unwrap();
        if handle.is_some() {
            return;
        }
        self.inner.running.store(true, Ordering::SeqCst);
        let inner = Arc::clone(&self.inner);
        *handle = Some(thread::spawn(move || inner.run_loop()));
    }

    pub fn stop(&self) {
        self.inner.running.store(false, Ordering::SeqCst);
        if let Some(handle) = self.thread.lock().unwrap().take() {
            let _ = handle.join();
        }
    }
}

impl Inner {
    fn save(&self, schedules: &[Schedule]) {
        let _ = schedule::save_to(&self.store_path, schedules);
    }

    fn run_loop(self: Arc<Self>) {
        // Which "once"/"daily" schedules already fired today, so a matching
        // minute does not re-fire on every one-second tick.
        let mut fired_today: HashMap<String, String> = HashMap::new();

        while self.running.load(Ordering::SeqCst) {
            let now = now_epoch();
            let local = chrono::Local::now();
            let today = local.format("%Y-%m-%d").to_string();
            let hhmm = local.format("%H:%M").to_string();

            let due: Vec<Due> = {
                let schedules = self.schedules.lock().unwrap();
                schedules
                    .iter()
                    .filter(|s| s.enabled)
                    .filter(|s| match s.kind.as_str() {
                        "interval" => now - s.last_run >= s.interval_min * 60.0,
                        "once" | "daily" => {
                            s.at_time == hhmm && fired_today.get(&s.id) != Some(&today)
                        }
                        _ => false,
                    })
                    .map(|s| Due {
                        id: s.id.clone(),
                        kind: s.kind.clone(),
                        target_type: s.target_type.clone(),
                        target_id: s.target_id.clone(),
                        macro_name: s.macro_name.clone(),
                        repeat: s.repeat,
                    })
                    .collect()
            };

            for d in due {
                // Only fire when idle; never interrupt an active session.
                if !(self.is_idle)() {
                    continue;
                }
                match (&self.fire_chain, d.target_type.as_str()) {
                    (Some(fire_chain), "chain") => {
                        fire_chain(&d.target_id);
                    }
                    _ => {
                        (self.fire)(&d.macro_name, d.repeat);
                    }
                }
                let mut schedules = self.schedules.lock().unwrap();
                if let Some(s) = schedules.iter_mut().find(|s| s.id == d.id) {
                    s.last_run = now;
                    if d.kind == "once" || d.kind == "daily" {
                        fired_today.insert(d.id.clone(), today.clone());
                    }
                    if d.kind == "once" {
                        s.enabled = false;
                    }
                }
                self.save(&schedules);
            }

            thread::sleep(Duration::from_secs(1));
        }
    }
}

#[cfg(test)]
impl MacroScheduler {
    /// Inject a pre-built schedule without minting an id or clamping the
    /// interval, the analogue of the Python test's `sched._schedules[id] = s`.
    fn insert_raw(&self, s: Schedule) {
        self.inner.schedules.lock().unwrap().push(s);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_dir;
    use std::time::Instant;

    #[test]
    fn scheduler_fires_chain_and_macro() {
        let store = temp_dir("sched_fire").join("schedules.json");
        let fired_macros = Arc::new(Mutex::new(Vec::<String>::new()));
        let fired_chains = Arc::new(Mutex::new(Vec::<String>::new()));
        let fm = Arc::clone(&fired_macros);
        let fc = Arc::clone(&fired_chains);

        let sched = MacroScheduler::new(
            &store,
            Box::new(|| true),
            Box::new(move |name: &str, _rep| {
                fm.lock().unwrap().push(name.to_string());
                true
            }),
            Some(Box::new(move |cid: &str| {
                fc.lock().unwrap().push(cid.to_string());
                true
            })),
        );

        // last_run = 0 makes both immediately due regardless of interval.
        sched.insert_raw(Schedule {
            id: "c1".into(),
            target_type: "chain".into(),
            target_id: "__test___sched_chain".into(),
            kind: "interval".into(),
            interval_min: 0.1,
            last_run: 0.0,
            ..Default::default()
        });
        sched.insert_raw(Schedule {
            id: "m1".into(),
            macro_name: "__test___sched_macro".into(),
            kind: "interval".into(),
            interval_min: 0.1,
            last_run: 0.0,
            ..Default::default()
        });

        sched.start();
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if !fired_chains.lock().unwrap().is_empty() && !fired_macros.lock().unwrap().is_empty()
            {
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
        sched.stop();

        let chains = fired_chains.lock().unwrap();
        let macros = fired_macros.lock().unwrap();
        assert!(
            chains.first().map(String::as_str) == Some("__test___sched_chain"),
            "scheduler fired chain: {chains:?}"
        );
        assert!(
            macros.first().map(String::as_str) == Some("__test___sched_macro"),
            "scheduler fired macro: {macros:?}"
        );
    }
}
