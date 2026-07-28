//! Step / AI-macro data model: `macros/ai/<name>.json`.
//!
//! Mirrors `anime_macro/ai_macro.py::{Step, AIMacro}`. `Step.from_dict` preserves
//! persisted `0`/`""` (no coercion), so container `#[serde(default)]` + a manual
//! `Default` reproduces it. Note `AIMacro.loop_count` defaults to 1 (unlike the
//! recorded `Macro`, whose default is 0). Field order matches Python's `to_dict`.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::models::macro_def::{InputEventType, Macro, MacroEvent};
use crate::util::{py_float, round2};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Step {
    pub id: String,
    /// One of: click, key, type, scroll, delay, find_click, wait_for.
    #[serde(rename = "type")]
    pub step_type: String,
    pub enabled: bool,
    pub label: String,
    pub x: i64,
    pub y: i64,
    pub key: String,
    pub text: String,
    pub delay: f64,
    pub scroll_amount: i64,
    /// Detection mode: `"color"`, `"template"`, or `"features"`.
    pub detect_mode: String,
    pub hsv_low: [i64; 3],
    pub hsv_high: [i64; 3],
    pub template: String,
    pub region: [f64; 4],
    pub min_area: i64,
    pub timeout: f64,
    pub confidence: f64,
}

impl Default for Step {
    fn default() -> Self {
        Self {
            id: String::new(),
            step_type: String::new(),
            enabled: true,
            label: String::new(),
            x: 0,
            y: 0,
            key: String::new(),
            text: String::new(),
            delay: 0.0,
            scroll_amount: 0,
            detect_mode: "color".to_string(),
            hsv_low: [0, 0, 0],
            hsv_high: [179, 255, 255],
            template: String::new(),
            region: [0.0, 0.0, 100.0, 100.0],
            min_area: 40,
            timeout: 10.0,
            confidence: 0.8,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AIMacro {
    pub name: String,
    #[serde(rename = "loop")]
    pub loop_enabled: bool,
    pub loop_count: i64,
    pub steps: Vec<Step>,
}

impl Default for AIMacro {
    fn default() -> Self {
        Self {
            name: String::new(),
            loop_enabled: false,
            loop_count: 1,
            steps: Vec::new(),
        }
    }
}

impl AIMacro {
    /// Load a saved fine-tune from `macros/ai/<name>.json`. The step editor
    /// reads this back verbatim so a save round-trips — reopening the editor
    /// shows the user's edited steps, not a re-derivation of the recording
    /// (issue #3).
    pub fn load(path: &Path) -> serde_json::Result<Self> {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        serde_json::from_str(&text)
    }

    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(self).expect("AIMacro always serializes");
        crate::util::write_atomic(path, json.as_bytes())
    }
}

/// An 8-hex-char id, mirroring Python's `str(uuid.uuid4())[:8]`. The value carries
/// no meaning (only uniqueness within one step list matters), so a process-global
/// counter mixed with the clock avoids the collisions a bare `now_millis()` would
/// hit when `macro_to_steps` mints many ids in one tight loop, without adding a uuid
/// dependency the codebase otherwise avoids.
fn short_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mix = (crate::util::now_millis() as u64)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(n);
    format!("{:08x}", mix & 0xFFFF_FFFF)
}

/// Convert a recorded [`Macro`] into an editable list of [`Step`]s, Python's
/// `ai_macro.macro_to_steps`. Drops mouse-move noise, pairs down/up into clicks and
/// key down/up into presses (skipping the matching up-event only when it is the very
/// next event), and inserts a `delay` step for gaps over 0.5s between actions.
pub fn macro_to_steps(macro_def: &Macro) -> Vec<Step> {
    let mut steps: Vec<Step> = Vec::new();
    let mut prev_time = 0.0_f64;

    // Drop mouse-move noise; only meaningful actions remain.
    let events: Vec<&MacroEvent> = macro_def
        .events
        .iter()
        .filter(|e| e.event_type != InputEventType::MouseMove)
        .collect();

    let mut i = 0;
    while i < events.len() {
        let e = events[i];

        // Insert a delay step for gaps > 0.5s between actions.
        let gap = e.timestamp - prev_time;
        if gap > 0.5 {
            let d = round2(gap);
            steps.push(Step {
                id: short_id(),
                step_type: "delay".to_string(),
                delay: d,
                label: format!("Wait {}s", py_float(d)),
                ..Default::default()
            });
        }

        match e.event_type {
            InputEventType::MouseDown => {
                steps.push(Step {
                    id: short_id(),
                    step_type: "click".to_string(),
                    x: e.x,
                    y: e.y,
                    label: format!("Click ({}, {})", e.x, e.y),
                    ..Default::default()
                });
                // Skip the matching MOUSE_UP if it's the very next event.
                if i + 1 < events.len() && events[i + 1].event_type == InputEventType::MouseUp {
                    prev_time = events[i + 1].timestamp;
                    i += 2;
                } else {
                    prev_time = e.timestamp;
                    i += 1;
                }
            }
            InputEventType::KeyDown => {
                let key = if e.key.is_empty() {
                    "?".to_string()
                } else {
                    e.key.clone()
                };
                steps.push(Step {
                    id: short_id(),
                    step_type: "key".to_string(),
                    key: key.clone(),
                    label: format!("Press {key}"),
                    ..Default::default()
                });
                // Skip the matching KEY_UP if it's the very next event.
                if i + 1 < events.len() && events[i + 1].event_type == InputEventType::KeyUp {
                    prev_time = events[i + 1].timestamp;
                    i += 2;
                } else {
                    prev_time = e.timestamp;
                    i += 1;
                }
            }
            InputEventType::Scroll => {
                let delta = e.delta;
                steps.push(Step {
                    id: short_id(),
                    step_type: "scroll".to_string(),
                    x: e.x,
                    y: e.y,
                    scroll_amount: delta,
                    label: format!("Scroll {delta:+}"),
                    ..Default::default()
                });
                prev_time = e.timestamp;
                i += 1;
            }
            _ => {
                // Skip lone MOUSE_UP, MOUSE_CLICK, etc.
                prev_time = e.timestamp;
                i += 1;
            }
        }
    }

    steps
}

#[cfg(test)]
mod tests {
    use super::{macro_to_steps, short_id};
    use crate::models::macro_def::{InputEventType, Macro, MacroEvent};

    fn ev(event_type: InputEventType, timestamp: f64) -> MacroEvent {
        MacroEvent {
            event_type,
            timestamp,
            x: 0,
            y: 0,
            mouse_motion: None,
            dx: 0,
            dy: 0,
            button: "left".to_string(),
            key: String::new(),
            delta: 0,
            duration: 0.0,
            checkpoint: None,
        }
    }

    fn macro_of(events: Vec<MacroEvent>) -> Macro {
        Macro {
            events,
            ..Default::default()
        }
    }

    #[test]
    fn short_id_is_eight_hex_and_unique() {
        let a = short_id();
        let b = short_id();
        assert_eq!(a.len(), 8);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn drops_mouse_move_and_pairs_down_up_into_one_click() {
        let mut down = ev(InputEventType::MouseDown, 0.1);
        down.x = 100;
        down.y = 200;
        let steps = macro_of(vec![
            ev(InputEventType::MouseMove, 0.0),
            down,
            ev(InputEventType::MouseUp, 0.12),
        ]);
        let steps = macro_to_steps(&steps);
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].step_type, "click");
        assert_eq!(steps[0].x, 100);
        assert_eq!(steps[0].y, 200);
        assert_eq!(steps[0].label, "Click (100, 200)");
        assert!(steps[0].enabled);
    }

    #[test]
    fn inserts_delay_step_for_gaps_over_half_a_second() {
        let steps = macro_to_steps(&macro_of(vec![
            ev(InputEventType::KeyDown, 0.2),
            ev(InputEventType::KeyUp, 0.25),
            // 0.95s gap from prev_time (0.25) to 1.2 → a delay step precedes the key.
            ev(InputEventType::KeyDown, 1.2),
            ev(InputEventType::KeyUp, 1.25),
        ]));
        // key, delay, key
        assert_eq!(steps.len(), 3);
        assert_eq!(steps[1].step_type, "delay");
        assert_eq!(steps[1].delay, 0.95);
        assert_eq!(steps[1].label, "Wait 0.95s");
    }

    #[test]
    fn key_down_uses_the_key_and_empty_becomes_question_mark() {
        let mut a = ev(InputEventType::KeyDown, 0.1);
        a.key = "a".to_string();
        let lone = ev(InputEventType::KeyDown, 0.2); // empty key, no matching KEY_UP
        let steps = macro_to_steps(&macro_of(vec![a, ev(InputEventType::KeyUp, 0.12), lone]));
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].step_type, "key");
        assert_eq!(steps[0].key, "a");
        assert_eq!(steps[0].label, "Press a");
        assert_eq!(steps[1].key, "?");
        assert_eq!(steps[1].label, "Press ?");
    }

    #[test]
    fn scroll_step_carries_signed_label() {
        let mut up = ev(InputEventType::Scroll, 0.1);
        up.delta = 3;
        let mut down = ev(InputEventType::Scroll, 0.2);
        down.delta = -2;
        let steps = macro_to_steps(&macro_of(vec![up, down]));
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].step_type, "scroll");
        assert_eq!(steps[0].scroll_amount, 3);
        assert_eq!(steps[0].label, "Scroll +3");
        assert_eq!(steps[1].label, "Scroll -2");
    }

    #[test]
    fn lone_mouse_up_is_skipped_but_advances_the_clock() {
        // A lone MOUSE_UP produces no step; the following click sees prev_time
        // advanced to the up-event's timestamp, so its 0.3s gap stays under 0.5s.
        let mut click = ev(InputEventType::MouseDown, 0.6);
        click.x = 5;
        click.y = 6;
        let steps = macro_to_steps(&macro_of(vec![
            ev(InputEventType::MouseUp, 0.3),
            click,
            ev(InputEventType::MouseUp, 0.62),
        ]));
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].step_type, "click");
    }
}
