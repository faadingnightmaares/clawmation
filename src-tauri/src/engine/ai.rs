//! AI step executor — the run loop and single-step execution behind the per-macro
//! step editor. Mirrors `anime_macro/ai_macro.py::AIExecutor`.
//!
//! Detection is injected (in production it grabs a fresh frame and runs the
//! detector) and so is actuation (the input controller), so the
//! orchestration that lives here — the loop, `find_click`'s stop-on-miss,
//! `wait_for`'s poll/deadline, and the summary shape — is exercised by
//! hardware-free tests through mock closures. Only enabled steps run; a failed
//! `find_click` stops the whole run, remaining iterations included.
//!
//! `AIExecutor._on_step` isn't reproduced: the live `Api.steps_run` path never set
//! a step callback, so it would be dead plumbing here.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::models::step::Step;
use crate::util::py_float;

/// One detection hit — the `(x, y, confidence)` of a match, which is all the
/// executor's messages and result fields need (`Detection.{x,y,confidence}`).
#[derive(Debug, Clone)]
pub struct Match {
    pub x: i64,
    pub y: i64,
    pub confidence: f64,
}

/// A hardware action the executor asks its actuator to perform. Collapses the
/// controller calls `_execute_step` made so a test can assert an action log without
/// touching the real mouse/keyboard. `Sleep` is an action (not a bare
/// `thread::sleep`) so the `delay` step's wait is injectable too.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Click(i64, i64),
    KeyPress(String),
    TypeText(String),
    Scroll(i64, Option<(i64, i64)>),
    Sleep(f64),
}

/// Poll a step's detection against a fresh frame → `(matches, message)`. Production
/// calls `core::Vision`; tests supply a canned closure.
pub type Detect = Box<dyn Fn(&Step) -> (Vec<Match>, String) + Send + Sync>;

/// Perform one [`Action`]. Production drives the controller; tests record the call.
pub type Actuate = Box<dyn Fn(Action) + Send + Sync>;

/// The outcome of one executed step — mirrors `ai_macro.StepResult`, spread into
/// each run-summary result entry.
struct StepResult {
    ok: bool,
    message: String,
    found_x: i64,
    found_y: i64,
    matched: i64,
    confidence: f64,
    elapsed: f64,
}

/// Execute one step through the injected seams — Python's `_execute_step`. The
/// action steps (`click`/`key`/`type`/`scroll`/`delay`) actuate and report their
/// intent; `find_click`/`wait_for` detect and act on a hit.
fn execute_step(step: &Step, detect: &Detect, actuate: &Actuate, running: &AtomicBool) -> StepResult {
    let t0 = Instant::now();
    let secs = |t: Instant| t.elapsed().as_secs_f64();

    match step.step_type.as_str() {
        "click" => {
            actuate(Action::Click(step.x, step.y));
            StepResult {
                ok: true,
                message: format!("clicked ({}, {})", step.x, step.y),
                found_x: step.x,
                found_y: step.y,
                matched: 0,
                confidence: 0.0,
                elapsed: secs(t0),
            }
        }
        "key" => {
            actuate(Action::KeyPress(step.key.clone()));
            StepResult {
                ok: true,
                message: format!("pressed '{}'", step.key),
                found_x: -1,
                found_y: -1,
                matched: 0,
                confidence: 0.0,
                elapsed: secs(t0),
            }
        }
        "type" => {
            actuate(Action::TypeText(step.text.clone()));
            StepResult {
                ok: true,
                message: format!("typed '{}'", step.text),
                found_x: -1,
                found_y: -1,
                matched: 0,
                confidence: 0.0,
                elapsed: secs(t0),
            }
        }
        "scroll" => {
            // Python: `controller.scroll(amount, step.x or None, step.y or None)`,
            // and `scroll` only moves when both coordinates are non-None. `0` is
            // falsy, so a move happens only when both are non-zero.
            let pos = if step.x != 0 && step.y != 0 { Some((step.x, step.y)) } else { None };
            actuate(Action::Scroll(step.scroll_amount, pos));
            StepResult {
                ok: true,
                message: format!("scrolled {:+}", step.scroll_amount),
                found_x: -1,
                found_y: -1,
                matched: 0,
                confidence: 0.0,
                elapsed: secs(t0),
            }
        }
        "delay" => {
            // Clamp negative/NaN so `Duration::from_secs_f64` in the actuator can't
            // panic (a panic in the run thread would leak the mode). Python's
            // `time.sleep(-x)` raised instead — an error path either way — and the
            // message still reports the raw value via `str(float)`.
            actuate(Action::Sleep(step.delay));
            StepResult {
                ok: true,
                message: format!("waited {}s", py_float(step.delay)),
                found_x: -1,
                found_y: -1,
                matched: 0,
                confidence: 0.0,
                elapsed: secs(t0),
            }
        }
        "find_click" => {
            let (matches, msg) = detect(step);
            match matches.first() {
                None => StepResult {
                    ok: false,
                    message: format!("find_click: nothing found ({msg})"),
                    found_x: -1,
                    found_y: -1,
                    matched: 0,
                    confidence: 0.0,
                    elapsed: secs(t0),
                },
                Some(best) => {
                    actuate(Action::Click(best.x, best.y));
                    StepResult {
                        ok: true,
                        message: format!("clicked match at ({}, {})", best.x, best.y),
                        found_x: best.x,
                        found_y: best.y,
                        matched: matches.len() as i64,
                        confidence: best.confidence,
                        elapsed: secs(t0),
                    }
                }
            }
        }
        "wait_for" => {
            // Python: `deadline = perf_counter() + step.timeout`. A negative/NaN
            // timeout would panic `Duration::from_secs_f64`, so clamp the span to
            // zero — the deadline is then already past and the loop reports the
            // timeout immediately, matching Python's past-deadline `while`. The
            // message keeps the raw value.
            let deadline = t0 + Duration::from_secs_f64(step.timeout.max(0.0));
            while Instant::now() < deadline && running.load(Ordering::Relaxed) {
                let (matches, _) = detect(step);
                if let Some(best) = matches.first() {
                    return StepResult {
                        ok: true,
                        message: format!("appeared at ({}, {})", best.x, best.y),
                        found_x: best.x,
                        found_y: best.y,
                        matched: matches.len() as i64,
                        confidence: best.confidence,
                        elapsed: secs(t0),
                    };
                }
                std::thread::sleep(Duration::from_millis(150));
            }
            StepResult {
                ok: false,
                message: format!("timed out after {}s", py_float(step.timeout)),
                found_x: -1,
                found_y: -1,
                matched: 0,
                confidence: 0.0,
                elapsed: secs(t0),
            }
        }
        other => StepResult {
            ok: false,
            message: format!("unknown step type '{other}'"),
            found_x: -1,
            found_y: -1,
            matched: 0,
            confidence: 0.0,
            elapsed: secs(t0),
        },
    }
}

/// Execute an AI macro's steps → the summary dict (`ok`, `iterations`,
/// `steps_run`, `steps_passed`, `results`) — Python's `AIExecutor.run`. Only
/// enabled steps run; a failed `find_click` stops the whole run (all remaining
/// iterations); `loop_enabled`/`loop_count` drive the outer repeat.
pub fn run(
    steps: &[Step],
    loop_enabled: bool,
    loop_count: i64,
    detect: &Detect,
    actuate: &Actuate,
) -> Value {
    let running = AtomicBool::new(true);
    let mut iterations: i64 = 0;
    let max_iter = if loop_enabled { loop_count } else { 1 };
    let mut results: Vec<Value> = Vec::new();

    while running.load(Ordering::Relaxed) && iterations < max_iter {
        iterations += 1;
        for (i, step) in steps.iter().enumerate() {
            if !running.load(Ordering::Relaxed) {
                break;
            }
            if !step.enabled {
                continue;
            }
            let result = execute_step(step, detect, actuate, &running);
            let label = if step.label.is_empty() {
                step.step_type.clone()
            } else {
                step.label.clone()
            };
            results.push(json!({
                "index": i,
                "label": label,
                "ok": result.ok,
                "message": result.message,
                "found_x": result.found_x,
                "found_y": result.found_y,
                "matched": result.matched,
                "confidence": result.confidence,
                "elapsed": result.elapsed,
            }));
            if !result.ok && step.step_type == "find_click" {
                // A failed find_click stops the run (target missing).
                running.store(false, Ordering::Relaxed);
                break;
            }
        }
    }

    running.store(false, Ordering::Relaxed);
    let passed = results
        .iter()
        .filter(|r| r["ok"].as_bool().unwrap_or(false))
        .count() as i64;
    let ok = if results.is_empty() {
        true
    } else {
        results.iter().all(|r| r["ok"].as_bool().unwrap_or(false))
    };
    json!({
        "ok": ok,
        "iterations": iterations,
        "steps_run": results.len(),
        "steps_passed": passed,
        "results": results,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::{run, Action, Actuate, Detect, Match};
    use crate::models::step::Step;

    /// An actuator that records every action it is handed.
    fn recording_actuate() -> (Actuate, Arc<Mutex<Vec<Action>>>) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let sink = log.clone();
        let actuate: Actuate = Box::new(move |a| sink.lock().unwrap().push(a));
        (actuate, log)
    }

    /// A detector that always returns the same canned result.
    fn detect_returning(matches: Vec<Match>, message: &str) -> Detect {
        let message = message.to_string();
        Box::new(move |_| (matches.clone(), message.clone()))
    }

    fn step(step_type: &str) -> Step {
        Step { step_type: step_type.to_string(), ..Default::default() }
    }

    #[test]
    fn runs_enabled_steps_and_skips_disabled() {
        let steps = vec![
            Step { step_type: "click".into(), x: 10, y: 20, ..Default::default() },
            Step { step_type: "key".into(), key: "a".into(), enabled: false, ..Default::default() },
            Step { step_type: "type".into(), text: "hi".into(), ..Default::default() },
        ];
        let (actuate, log) = recording_actuate();
        let detect = detect_returning(vec![], "");
        let summary = run(&steps, false, 1, &detect, &actuate);

        assert_eq!(
            *log.lock().unwrap(),
            vec![Action::Click(10, 20), Action::TypeText("hi".into())]
        );
        assert_eq!(summary["ok"], true);
        assert_eq!(summary["iterations"], 1);
        assert_eq!(summary["steps_run"], 2);
        assert_eq!(summary["steps_passed"], 2);
    }

    #[test]
    fn find_click_clicks_the_best_match() {
        let steps = vec![step("find_click")];
        let (actuate, log) = recording_actuate();
        let detect = detect_returning(
            vec![Match { x: 50, y: 60, confidence: 0.9 }, Match { x: 1, y: 2, confidence: 0.5 }],
            "2 color match(es)",
        );
        let summary = run(&steps, false, 1, &detect, &actuate);

        assert_eq!(*log.lock().unwrap(), vec![Action::Click(50, 60)]);
        let r = &summary["results"][0];
        assert_eq!(r["ok"], true);
        assert_eq!(r["found_x"], 50);
        assert_eq!(r["found_y"], 60);
        assert_eq!(r["matched"], 2);
        assert_eq!(r["message"], "clicked match at (50, 60)");
    }

    #[test]
    fn a_failed_find_click_stops_the_run() {
        let steps = vec![
            step("find_click"),
            Step { step_type: "click".into(), x: 9, y: 9, ..Default::default() },
        ];
        let (actuate, log) = recording_actuate();
        let detect = detect_returning(vec![], "0 color match(es)");
        let summary = run(&steps, false, 1, &detect, &actuate);

        // The trailing click never runs — nothing was actuated.
        assert!(log.lock().unwrap().is_empty());
        assert_eq!(summary["ok"], false);
        assert_eq!(summary["steps_run"], 1);
        assert_eq!(summary["steps_passed"], 0);
        assert_eq!(
            summary["results"][0]["message"],
            "find_click: nothing found (0 color match(es))"
        );
    }

    #[test]
    fn wait_for_times_out_immediately_with_zero_timeout() {
        let steps = vec![Step { step_type: "wait_for".into(), timeout: 0.0, ..Default::default() }];
        let (actuate, _log) = recording_actuate();
        let detect = detect_returning(vec![], "0 color match(es)");
        let summary = run(&steps, false, 1, &detect, &actuate);

        let r = &summary["results"][0];
        assert_eq!(r["ok"], false);
        assert_eq!(r["message"], "timed out after 0.0s");
    }

    #[test]
    fn wait_for_returns_when_the_target_appears() {
        let steps = vec![Step { step_type: "wait_for".into(), timeout: 5.0, ..Default::default() }];
        let (actuate, _log) = recording_actuate();
        let detect = detect_returning(vec![Match { x: 7, y: 8, confidence: 0.8 }], "1 color match(es)");
        let summary = run(&steps, false, 1, &detect, &actuate);

        let r = &summary["results"][0];
        assert_eq!(r["ok"], true);
        assert_eq!(r["found_x"], 7);
        assert_eq!(r["message"], "appeared at (7, 8)");
    }

    #[test]
    fn scroll_moves_only_when_both_coordinates_are_nonzero() {
        let steps = vec![
            Step { step_type: "scroll".into(), scroll_amount: 3, x: 0, y: 0, ..Default::default() },
            Step { step_type: "scroll".into(), scroll_amount: -2, x: 5, y: 6, ..Default::default() },
        ];
        let (actuate, log) = recording_actuate();
        let detect = detect_returning(vec![], "");
        run(&steps, false, 1, &detect, &actuate);

        assert_eq!(
            *log.lock().unwrap(),
            vec![Action::Scroll(3, None), Action::Scroll(-2, Some((5, 6)))]
        );
    }

    #[test]
    fn delay_actuates_a_sleep_and_reports_a_python_float() {
        let steps = vec![Step { step_type: "delay".into(), delay: 2.0, ..Default::default() }];
        let (actuate, log) = recording_actuate();
        let detect = detect_returning(vec![], "");
        let summary = run(&steps, false, 1, &detect, &actuate);

        assert_eq!(*log.lock().unwrap(), vec![Action::Sleep(2.0)]);
        assert_eq!(summary["results"][0]["message"], "waited 2.0s");
    }

    #[test]
    fn loop_repeats_the_steps_loop_count_times() {
        let steps = vec![Step { step_type: "click".into(), x: 1, y: 1, ..Default::default() }];
        let (actuate, log) = recording_actuate();
        let detect = detect_returning(vec![], "");
        let summary = run(&steps, true, 2, &detect, &actuate);

        assert_eq!(log.lock().unwrap().len(), 2);
        assert_eq!(summary["iterations"], 2);
        assert_eq!(summary["steps_run"], 2);
    }

    #[test]
    fn an_unknown_step_type_fails_with_its_name() {
        let steps = vec![step("frobnicate")];
        let (actuate, _log) = recording_actuate();
        let detect = detect_returning(vec![], "");
        let summary = run(&steps, false, 1, &detect, &actuate);

        assert_eq!(summary["ok"], false);
        assert_eq!(summary["results"][0]["message"], "unknown step type 'frobnicate'");
    }
}
