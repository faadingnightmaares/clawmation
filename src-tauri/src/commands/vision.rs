//! Vision commands: the standalone, autonomous vision surface.
//!
//! Triggers are stored separately from per-macro guards, in
//! `macros/guards/_vision.json` under a `"triggers"` key. `vision_save` and
//! `vision_load` are the persistence half (ports of `Api.vision_save` /
//! `vision_load`); each trigger is a [`Guard`], round-tripped so the stored form
//! is the canonical `to_dict` shape. `vision_start` / `vision_stop` /
//! `vision_status` are the orchestration half: they arm the [`VisionAgent`] over
//! the saved triggers and surface its running state and rolling event feed
//! (ports of `Api.vision_start` / `vision_stop` / `vision_status`).

use std::path::Path;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tauri::State;

use crate::core::Core;
use crate::engine::vision_agent::{Act, Detect, MacroRunner, OnEvent, VisionAction, VisionAgent};
use crate::models::guard::Guard;
use crate::models::macro_def::Macro;
use crate::paths;
use crate::state::AppState;

#[tauri::command(async)]
pub fn vision_save(state: State<AppState>, triggers: Option<Vec<Value>>) -> Value {
    let parsed: Result<Vec<Guard>, _> = triggers
        .unwrap_or_default()
        .into_iter()
        .map(serde_json::from_value)
        .collect();
    let objs = match parsed {
        Ok(objs) => objs,
        Err(e) => {
            state.emit("err", format!("Vision save failed: {e}"));
            return json!({ "ok": false, "error": e.to_string() });
        }
    };
    let count = objs.len();
    let path = paths::guards_dir().join("_vision.json");
    let bytes = serde_json::to_vec_pretty(&json!({ "triggers": objs }))
        .expect("vision triggers always serialize");
    if let Err(e) = crate::util::write_atomic(&path, &bytes) {
        state.emit("err", format!("Vision save failed: {e}"));
        return json!({ "ok": false, "error": e.to_string() });
    }
    state.emit("ok", format!("Saved {count} vision trigger(s)"));
    json!({ "ok": true })
}

#[tauri::command(async)]
pub fn vision_load() -> Value {
    let path = paths::guards_dir().join("_vision.json");
    if !path.exists() {
        return json!({ "ok": true, "triggers": [] });
    }
    match parse_triggers(&path) {
        Ok(triggers) => json!({ "ok": true, "triggers": triggers }),
        Err(e) => json!({ "ok": false, "error": e, "triggers": [] }),
    }
}

/// Read `_vision.json`'s `"triggers"` array into [`Guard`]s, the `from_dict`
/// pass shared by `vision_load` and `vision_start`. A missing `"triggers"` key
/// is an empty list, matching `data.get("triggers", [])`.
fn parse_triggers(path: &Path) -> Result<Vec<Guard>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let data: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let arr = data
        .get("triggers")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    arr.into_iter()
        .map(|t| serde_json::from_value(t).map_err(|e| e.to_string()))
        .collect()
}

/// Build the vision agent's four injected closures over [`Core`] and the shared
/// event log, the Rust seat of the `on_event` / `macro_runner` that
/// `Api.vision_start` defines inline. Detection is a `core::Vision` grab, actuation
/// the input controller, and the runner reuses the one shared player (Python
/// spins up a fresh `MacroPlayer` per run; equivalent, as playback is otherwise
/// idle while vision drives).
fn build_agent(core: &Core, log: &Arc<Mutex<Vec<(String, String)>>>) -> VisionAgent {
    let detect: Detect = {
        let core = core.clone();
        Box::new(move |triggers: &[Guard]| core.vision.detect_guards_faithful("watch", triggers))
    };
    let act: Act = {
        let core = core.clone();
        Box::new(move |action: VisionAction| match action {
            VisionAction::KeyPress(key) => core.controller.key_press(&key),
            VisionAction::Click(x, y) => core.controller.click(x as i32, y as i32, "left"),
            VisionAction::Nudge(x, y) => {
                core.controller.move_to(x as i32, y as i32);
                core.controller.nudge();
            }
            VisionAction::MoveTo(x, y) => core.controller.move_to(x as i32, y as i32),
            VisionAction::MouseDown(button) => core.controller.mouse_down(None, &button),
            VisionAction::MouseUp(button) => core.controller.mouse_up(None, &button),
        })
    };
    let on_event: OnEvent = {
        let core = core.clone();
        let log = log.clone();
        Box::new(move |kind: &str, message: &str| {
            {
                let mut entries = log.lock().unwrap();
                entries.push((kind.to_string(), message.to_string()));
                let overflow = entries.len().saturating_sub(50);
                if overflow > 0 {
                    entries.drain(0..overflow);
                }
            }
            // Only fired actions reach the main log; start/stop stay in the feed.
            if kind == "act" {
                core.emit("warn", format!("Vision: {message}"));
            }
        })
    };
    let run_macro: MacroRunner = {
        let core = core.clone();
        Box::new(move |name: &str, repeat: i64| {
            let stem = name.strip_suffix(".json").unwrap_or(name);
            let path = paths::macros_dir().join(format!("{stem}.json"));
            if !path.exists() {
                return Err(format!("Macro not found: {stem}"));
            }
            let mut macro_def = Macro::load(&path).map_err(|e| e.to_string())?;
            macro_def.loop_enabled = repeat == 0;
            macro_def.loop_count = if repeat > 0 { repeat } else { 0 };
            // Bare player: the vision-agent runner passes no checkpoint detector,
            // so macro checkpoint steps are silent no-ops, faithful to Python's
            // detector-less `MacroPlayer(controller)` at ui_app.py:2415.
            core.player.play(macro_def, core.resolve_screen(), 1.0, None)?;
            match core.player.wait() {
                crate::hardware::player::PlaybackOutcome::Completed { .. } => Ok(()),
                crate::hardware::player::PlaybackOutcome::Stopped => Err("Macro playback was stopped".to_string()),
                crate::hardware::player::PlaybackOutcome::Failed(error) => Err(error),
            }
        })
    };
    VisionAgent::new(detect, act, on_event, run_macro)
}

#[tauri::command(async)]
pub fn vision_start(state: State<AppState>) -> Value {
    // Opening the capture is the slow part of starting (a cold backend can sit
    // there for a second or more), so it happens before the agent slot is
    // locked. Holding that lock across it wedges `vision_stop` and every status
    // poll behind it, which is what a Start button stuck on "loading" looks
    // like. It is idempotent, so a racing second start just no-ops here.
    let (w, h) = state.core.resolve_screen();
    let backend = state.core.config.lock().unwrap().capture_backend.clone();
    state.core.vision.ensure_ready(w as i64, h as i64, &backend);

    // Hold the agent slot across the check and the store so two starts can't
    // race a second poll thread into existence (pywebview serialized these).
    let mut slot = state.vision_agent.lock().unwrap();
    if slot.as_ref().is_some_and(|a| a.is_running()) {
        return json!({ "ok": false, "error": "Already running" });
    }

    let path = paths::guards_dir().join("_vision.json");
    if !path.exists() {
        return json!({ "ok": false, "error": "No triggers saved" });
    }
    let triggers = match parse_triggers(&path) {
        Ok(triggers) => triggers,
        Err(e) => return json!({ "ok": false, "error": e }),
    };

    let enabled: Vec<Guard> = triggers.into_iter().filter(|t| t.enabled).collect();
    if enabled.is_empty() {
        return json!({ "ok": false, "error": "No enabled triggers" });
    }

    let count = enabled.len();
    let agent = build_agent(&state.core, &state.vision_log);
    agent.start(enabled);
    state.core.detections.set("watch", true);
    *slot = Some(agent);
    json!({ "ok": true, "triggers": count })
}

#[tauri::command(async)]
pub fn vision_stop(state: State<AppState>) -> Value {
    // Release the slot lock before stop() joins the poll thread: if that thread is
    // mid-run on an infinite macro the join blocks, and holding the lock would wedge
    // every concurrent vision_status too (Python doesn't lock, so its status keeps
    // answering).
    let agent = state.vision_agent.lock().unwrap().take();
    match agent {
        Some(agent) => {
            // The poll thread may be parked in `player.wait()` on a trigger's
            // macro sequence, and a step with repeat 0 runs forever, so the
            // player is told to stop first. Without this the join below never
            // returns and the Stop button spins for good.
            state.core.player.stop();
            agent.stop();
            state.core.detections.set("watch", false);
            json!({ "ok": true })
        }
        None => json!({ "ok": false, "error": "Not running" }),
    }
}

/// The last detection pass for the on-screen overlay. See
/// [`Vision::sightings`](crate::core::Vision::sightings).
///
/// Deliberately the cheapest command in the app: the overlay polls it several
/// times a second while a macro runs, so it reads one mutex and copies a handful
/// of rectangles. It never captures or detects anything itself; it reports what
/// the detection loop already did.
#[tauri::command(async)]
pub fn get_detections(state: State<AppState>) -> Value {
    state.core.vision.sightings()
}

/// Running state and the rolling event feed.
///
/// Polled once a second by the Watch view for as long as it is open, so it never
/// waits on a lock: a `stop` mid-join holds the agent slot for as long as the
/// poll thread takes to notice, and a status call that queued behind that would
/// leave the view frozen on whatever it last drew. `ok: false` here means only
/// "ask again in a moment", and the view keeps showing what it has.
#[tauri::command(async)]
pub fn vision_status(state: State<AppState>) -> Value {
    let Ok(slot) = state.vision_agent.try_lock() else {
        return json!({ "ok": false, "busy": true });
    };
    let (running, fired) = match slot.as_ref() {
        Some(agent) => (agent.is_running(), agent.fired_count()),
        None => (false, 0),
    };
    drop(slot);
    let entries = state.vision_log.lock().unwrap();
    let start = entries.len().saturating_sub(20);
    let log: Vec<Value> = entries[start..]
        .iter()
        .rev()
        .map(|(kind, msg)| json!({ "kind": kind, "msg": msg }))
        .collect();
    json!({ "ok": true, "running": running, "fired": fired, "log": log })
}
