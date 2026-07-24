//! Chain execution engine — run several macros back-to-back.
//!
//! Mirrors `anime_macro/chains.py::ChainManager`. A chain runs on a dedicated
//! background thread that plays each macro in order, waiting for the app to
//! return to idle before and after each one, repeating the whole sequence
//! `repeat` times (0 = infinite). Only one chain runs at a time; a second `run`
//! while busy is refused. Chains persist to `config/chains.json`.
//!
//! The Python class injects `play_macro` and `is_idle` callbacks so it can be
//! tested without real playback; we keep that shape with boxed closures.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::json;

use crate::models::chain::{self, Chain};
use crate::util::now_millis;

/// `play_macro(macro_name, repeat) -> started_ok`.
pub type PlayMacro = Box<dyn Fn(&str, i64) -> bool + Send + Sync>;
/// `is_idle() -> can_start` (true when the app is neither recording nor playing).
pub type IsIdle = Box<dyn Fn() -> bool + Send + Sync>;

/// Poll interval while waiting for the app to become idle, matching Python's
/// `time.sleep(0.5)`.
const IDLE_POLL: Duration = Duration::from_millis(500);
/// Give up waiting for idle after 5 minutes, as in the source.
const IDLE_TIMEOUT: Duration = Duration::from_secs(300);

struct State {
    chains: Vec<Chain>,
    running_chain: Option<String>,
    current_index: usize,
    current_iteration: i64,
}

struct Inner {
    store_path: PathBuf,
    play_macro: PlayMacro,
    is_idle: IsIdle,
    state: Mutex<State>,
    stop_flag: AtomicBool,
}

/// Outcome of [`ChainManager::duplicate`]: the new chain's `id` and `name`, plus
/// the source chain's name — the left side of the `Api.duplicate_chain` emit.
pub struct Duplicated {
    pub id: String,
    pub name: String,
    pub source_name: String,
}

pub struct ChainManager {
    inner: Arc<Inner>,
}

impl ChainManager {
    pub fn new(store_path: impl Into<PathBuf>, play_macro: PlayMacro, is_idle: IsIdle) -> Self {
        let store_path = store_path.into();
        let chains = chain::load(&store_path)
            .into_iter()
            .filter(|c| !c.id.is_empty())
            .collect();
        Self {
            inner: Arc::new(Inner {
                store_path,
                play_macro,
                is_idle,
                state: Mutex::new(State {
                    chains,
                    running_chain: None,
                    current_index: 0,
                    current_iteration: 1,
                }),
                stop_flag: AtomicBool::new(false),
            }),
        }
    }

    // ── CRUD ────────────────────────────────────────────────────────────────

    pub fn list(&self) -> Vec<Chain> {
        self.inner.state.lock().unwrap().chains.clone()
    }

    pub fn add(
        &self,
        name: &str,
        macro_names: Vec<String>,
        delay_between: f64,
        repeat: i64,
    ) -> Chain {
        let mut state = self.inner.state.lock().unwrap();
        let c = Chain {
            id: format!("chain_{}", now_millis()),
            name: name.to_string(),
            macro_names,
            delay_between,
            repeat,
        };
        state.chains.push(c.clone());
        self.inner.save(&state);
        c
    }

    pub fn remove(&self, cid: &str) -> bool {
        let mut state = self.inner.state.lock().unwrap();
        let before = state.chains.len();
        state.chains.retain(|c| c.id != cid);
        if state.chains.len() != before {
            self.inner.save(&state);
            true
        } else {
            false
        }
    }

    pub fn update(
        &self,
        cid: &str,
        name: Option<String>,
        macro_names: Option<Vec<String>>,
        delay_between: Option<f64>,
        repeat: Option<i64>,
    ) -> bool {
        let mut state = self.inner.state.lock().unwrap();
        let Some(c) = state.chains.iter_mut().find(|c| c.id == cid) else {
            return false;
        };
        if let Some(name) = name {
            c.name = name;
        }
        if let Some(macro_names) = macro_names {
            c.macro_names = macro_names;
        }
        if let Some(delay_between) = delay_between {
            c.delay_between = delay_between;
        }
        if let Some(repeat) = repeat {
            c.repeat = repeat;
        }
        self.inner.save(&state);
        true
    }

    /// `Api.duplicate_chain`: copy a chain under a fresh id and a unique
    /// `{name}_copy` / `{name}_copyN` name (N counting from 2). Find, rename,
    /// insert, and save happen under one held lock, matching Python's
    /// `with self._lock:` block. Returns `None` if the source id is unknown.
    pub fn duplicate(&self, cid: &str) -> Option<Duplicated> {
        let mut state = self.inner.state.lock().unwrap();
        let source = state.chains.iter().find(|c| c.id == cid)?.clone();
        let base = format!("{}_copy", source.name);
        let existing: HashSet<String> = state.chains.iter().map(|c| c.name.clone()).collect();
        let mut new_name = base.clone();
        let mut counter = 2;
        while existing.contains(&new_name) {
            new_name = format!("{base}{counter}");
            counter += 1;
        }
        let new_id = format!("chain_{}", now_millis());
        state.chains.push(Chain {
            id: new_id.clone(),
            name: new_name.clone(),
            macro_names: source.macro_names.clone(),
            delay_between: source.delay_between,
            repeat: source.repeat,
        });
        self.inner.save(&state);
        Some(Duplicated {
            id: new_id,
            name: new_name,
            source_name: source.name,
        })
    }

    // ── Execution ───────────────────────────────────────────────────────────

    /// Start running a chain on a background thread. Returns false if a chain is
    /// already running, the id is unknown, or the chain has no macros.
    pub fn run(&self, cid: &str) -> bool {
        let chain = {
            let mut state = self.inner.state.lock().unwrap();
            if state.running_chain.is_some() {
                return false;
            }
            let Some(chain) = state.chains.iter().find(|c| c.id == cid).cloned() else {
                return false;
            };
            if chain.macro_names.is_empty() {
                return false;
            }
            state.running_chain = Some(cid.to_string());
            self.inner.stop_flag.store(false, Ordering::SeqCst);
            chain
        };

        let inner = Arc::clone(&self.inner);
        thread::spawn(move || inner.run_chain(chain));
        true
    }

    /// Signal the running chain to stop after the current macro.
    pub fn stop(&self) {
        self.inner.stop_flag.store(true, Ordering::SeqCst);
    }

    /// The id of the running chain, or `None` when idle.
    pub fn running_chain(&self) -> Option<String> {
        self.inner.state.lock().unwrap().running_chain.clone()
    }

    /// Progress of the running chain, in the same shape as Python's
    /// `get_progress` (`{"running": false}` when nothing is running).
    pub fn get_progress(&self) -> serde_json::Value {
        let state = self.inner.state.lock().unwrap();
        let Some(cid) = &state.running_chain else {
            return json!({ "running": false });
        };
        let Some(chain) = state.chains.iter().find(|c| &c.id == cid) else {
            return json!({ "running": false });
        };
        let total = chain.macro_names.len();
        let idx = state.current_index;
        json!({
            "running": true,
            "chain_id": cid,
            "chain_name": chain.name,
            "current_index": idx,
            "current_macro": chain.macro_names.get(idx).cloned().unwrap_or_default(),
            "total_macros": total,
            "iteration": state.current_iteration,
            "total_iterations": chain.repeat,
        })
    }
}

impl Inner {
    fn save(&self, state: &State) {
        let _ = chain::save_to(&self.store_path, &state.chains);
    }

    fn stopped(&self) -> bool {
        self.stop_flag.load(Ordering::SeqCst)
    }

    /// Wait until the app is idle. Returns false if stopped or timed out.
    fn wait_for_idle(&self) -> bool {
        let start = Instant::now();
        while !(self.is_idle)() {
            if self.stopped() {
                return false;
            }
            if start.elapsed() > IDLE_TIMEOUT {
                return false;
            }
            thread::sleep(IDLE_POLL);
        }
        true
    }

    fn run_chain(self: Arc<Self>, chain: Chain) {
        let mut iteration = 0i64;
        loop {
            iteration += 1;
            if chain.repeat > 0 && iteration > chain.repeat {
                break;
            }
            {
                let mut state = self.state.lock().unwrap();
                state.current_iteration = iteration;
            }
            let last = chain.macro_names.len().saturating_sub(1);
            for (i, macro_name) in chain.macro_names.iter().enumerate() {
                if self.stopped() {
                    self.finish();
                    return;
                }
                {
                    let mut state = self.state.lock().unwrap();
                    state.current_index = i;
                }
                // Wait until idle before starting the next macro.
                if !self.wait_for_idle() {
                    self.finish();
                    return;
                }
                (self.play_macro)(macro_name, 1);
                // Wait for the macro to finish (back to idle).
                if !self.wait_for_idle() {
                    self.finish();
                    return;
                }
                // Delay between macros, but not after the last one.
                if i < last && chain.delay_between > 0.0 {
                    thread::sleep(Duration::from_secs_f64(chain.delay_between));
                }
            }
        }
        self.finish();
    }

    fn finish(&self) {
        let mut state = self.state.lock().unwrap();
        state.running_chain = None;
        state.current_index = 0;
        state.current_iteration = 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_dir;

    /// Block until the chain clears its running flag, or the deadline passes.
    fn wait_idle(cm: &ChainManager, secs: u64) {
        let deadline = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < deadline && cm.running_chain().is_some() {
            thread::sleep(Duration::from_millis(50));
        }
    }

    #[test]
    fn chains_run_in_order_and_persist() {
        let store = temp_dir("chains_run").join("chains.json");
        let played = Arc::new(Mutex::new(Vec::<String>::new()));
        let played_cb = Arc::clone(&played);

        let cm = ChainManager::new(
            &store,
            Box::new(move |name: &str, _rep| {
                played_cb.lock().unwrap().push(name.to_string());
                true
            }),
            Box::new(|| true),
        );
        let c = cm.add(
            "__test___chain",
            vec![
                "__test___a".into(),
                "__test___b".into(),
                "__test___c".into(),
            ],
            0.05,
            1,
        );
        assert!(cm.run(&c.id), "chain started");
        wait_idle(&cm, 10);
        assert_eq!(
            *played.lock().unwrap(),
            vec!["__test___a", "__test___b", "__test___c"],
            "chain ran all 3 in order"
        );
        assert!(cm.running_chain().is_none(), "chain cleared running flag");

        // Persistence across restart.
        let cm2 = ChainManager::new(&store, Box::new(|_, _| true), Box::new(|| true));
        assert_eq!(cm2.list().len(), 1, "chain persisted");
    }

    #[test]
    fn chain_repeat_runs_twice() {
        let store = temp_dir("chain_repeat").join("chains.json");
        let played = Arc::new(Mutex::new(Vec::<String>::new()));
        let played_cb = Arc::clone(&played);

        let cm = ChainManager::new(
            &store,
            Box::new(move |name: &str, _rep| {
                played_cb.lock().unwrap().push(name.to_string());
                true
            }),
            Box::new(|| true),
        );
        let c = cm.add(
            "__test___chain_rep",
            vec!["__test___x".into(), "__test___y".into()],
            0.02,
            2,
        );
        cm.run(&c.id);
        wait_idle(&cm, 10);
        assert_eq!(
            *played.lock().unwrap(),
            vec!["__test___x", "__test___y", "__test___x", "__test___y"],
            "chain repeated twice in order"
        );
    }

    #[test]
    fn chain_refuses_second_while_busy() {
        let store = temp_dir("chain_busy").join("chains.json");
        let idle = Arc::new(AtomicBool::new(false));
        let idle_cb = Arc::clone(&idle);

        let cm = ChainManager::new(
            &store,
            Box::new(|_, _| true),
            Box::new(move || idle_cb.load(Ordering::SeqCst)),
        );
        let c = cm.add("__test___busy", vec!["__test___m1".into()], 0.02, 1);
        assert!(cm.run(&c.id), "first chain starts");
        thread::sleep(Duration::from_millis(100));
        assert!(!cm.run(&c.id), "second chain refused while busy");
        idle.store(true, Ordering::SeqCst); // let the first finish
        wait_idle(&cm, 10);
        cm.stop();
    }
}
