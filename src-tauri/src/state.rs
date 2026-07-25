//! Shared application state, managed by Tauri and injected into commands.
//!
//! `AppState` is the Rust analogue of the Python `Api` object: a cheaply-cloned
//! bundle of the [`Core`] wiring hub plus the managers that orchestrate it
//! (chains, scheduler, and the optional vision agent). `Core` owns the leaf Arcs
//! and the permanent guard engine; the managers here capture `Core` clones in
//! their callbacks, so the object graph is a DAG rooted at `AppState`.

use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use crate::core::Core;
use crate::engine::chains::ChainManager;
use crate::engine::scheduler::MacroScheduler;
use crate::engine::vision_agent::VisionAgent;
use crate::models::config::MacroConfig;
use crate::paths;

#[derive(Clone)]
pub struct AppState {
    pub core: Core,
    pub chains: Arc<ChainManager>,
    pub scheduler: Arc<MacroScheduler>,
    /// Present only while the vision-agent loop is running (`Api.start_vision`).
    pub vision_agent: Arc<Mutex<Option<VisionAgent>>>,
    /// The vision panel's rolling event feed (`Api._vision_log`) — `(kind, msg)`
    /// pairs, newest last, capped at 50. Fed by the agent's `on_event` closure.
    pub vision_log: Arc<Mutex<Vec<(String, String)>>>,
}

impl AppState {
    pub fn new(config: MacroConfig) -> Self {
        let core = Core::new(config);

        // Chains and the scheduler drive playback through `Core`; each callback
        // captures its own `Core` clone (leaving the graph acyclic). A scheduled
        // repeat/chain play always uses speed 1.0, as the source does.
        let play: Box<dyn Fn(&str, i64) -> bool + Send + Sync> = {
            let core = core.clone();
            Box::new(move |name, repeat| {
                core.play_macro(name, Some(json!(repeat)), 1.0)
                    .get("ok")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            })
        };
        let chain_idle: Box<dyn Fn() -> bool + Send + Sync> = {
            let core = core.clone();
            Box::new(move || core.runtime.lock().unwrap().mode == "idle")
        };
        let chains = Arc::new(ChainManager::new(
            paths::config_dir().join("chains.json"),
            play,
            chain_idle,
        ));

        let sched_idle: Box<dyn Fn() -> bool + Send + Sync> = {
            let core = core.clone();
            Box::new(move || core.runtime.lock().unwrap().mode == "idle")
        };
        let fire: Box<dyn Fn(&str, i64) -> bool + Send + Sync> = {
            let core = core.clone();
            Box::new(move |name, repeat| {
                // Scheduled-run toast — `Api._scheduler_fire`.
                if core.config.lock().unwrap().notify_on_schedule {
                    core.notifier.notify(
                        "Clawmation \u{2014} Scheduled Macro",
                        &format!("Running '{name}'"),
                    );
                }
                core.play_macro(name, Some(json!(repeat)), 1.0)
                    .get("ok")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            })
        };
        let fire_chain: Box<dyn Fn(&str) -> bool + Send + Sync> = {
            let core = core.clone();
            let chains = chains.clone();
            Box::new(move |cid| {
                // Scheduled-chain toast — `Api._scheduler_fire_chain`. Resolve the
                // chain's name for a friendlier message, falling back to its id.
                if core.config.lock().unwrap().notify_on_schedule {
                    let chain_name = chains
                        .list()
                        .into_iter()
                        .find(|c| c.id == cid)
                        .map(|c| c.name)
                        .unwrap_or_else(|| cid.to_string());
                    core.notifier.notify(
                        "Clawmation \u{2014} Scheduled Chain",
                        &format!("Running chain '{chain_name}'"),
                    );
                }
                run_chain(&core, &chains, cid)
                    .get("ok")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            })
        };
        let scheduler = Arc::new(MacroScheduler::new(
            paths::config_dir().join("schedules.json"),
            sched_idle,
            fire,
            Some(fire_chain),
        ));
        scheduler.start();

        Self {
            core,
            chains,
            scheduler,
            vision_agent: Arc::new(Mutex::new(None)),
            vision_log: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Append a log entry (delegates to `Core::emit`, the analogue of `Api._emit`).
    pub fn emit(&self, level: &str, msg: impl Into<String>) {
        self.core.emit(level, msg);
    }
}

/// `Api.run_chain`: start a chain and log the outcome. Shared by the `run_chain`
/// command and the scheduler's fire-chain callback.
pub fn run_chain(core: &Core, chains: &ChainManager, chain_id: &str) -> Value {
    let ok = chains.run(chain_id);
    if ok {
        core.emit("ok", "Chain started");
    } else {
        core.emit("warn", "Chain could not start (busy or not found)");
    }
    json!({ "ok": ok })
}
