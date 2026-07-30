//! The wiring hub (`Core`) that binds the leaf hardware Arcs to the permanent
//! guard engine and carries the playback logic `Api` kept as loose methods.
//!
//! `Core` is the Rust analogue of the mutable state `Api.__init__` builds: the
//! config, log buffer, runtime status, play stats, the reused input controller
//! and macro player, the (lazily opened) capture-and-detect pair, and one permanent
//! [`GuardEngine`]. It is cheaply `Clone` (every field is an `Arc`) so the
//! chains, scheduler, and playback watcher threads can each hold their own
//! handle.
//!
//! Acyclicity is deliberate: the guard engine's four callbacks capture only
//! *leaf* Arcs (vision, player, controller, log), never `Core`, so guards can
//! drive the same player a macro plays through without forming a reference
//! cycle. The chains/scheduler callbacks (built in `state.rs`) capture `Core`
//! clones, keeping the whole graph a DAG rooted at `AppState`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::{json, Value};

use crate::engine::guards::{Action, Actuate, Detect, GuardEngine, OnFire, PlayerState};
use crate::engine::stats::PlayStats;
use crate::hardware::capture::{Frame, ScreenCapture};
use crate::hardware::input::InputController;
use crate::hardware::ocr;
use crate::hardware::player::{CheckpointDetect, MacroPlayer, PlaybackOutcome};
use crate::hardware::preview;
use crate::hardware::recorder::MacroRecorder;
use crate::hardware::reliable_input::{ReliableInput, ReliableTarget};
use crate::hardware::vision::{is_full_region, region_pixels, Detection, Detector, VisionError};
use crate::logbuf::LogBuffer;
use crate::models::chain::Chain;
use crate::models::config::MacroConfig;
use crate::models::guard::{Guard, GuardFile};
use crate::models::macro_def::{InputEventType, Macro};
use crate::models::node_graph::NodeGraph;
use crate::models::step::Step;
use crate::notify::Notifier;
use crate::paths;
use crate::util::{py_float, round1, round3};

/// Live status the UI heartbeat reads. `mode` is one of
/// `idle | recording | playing | paused`.
pub struct Runtime {
    pub mode: String,
    /// When the current recording/playing run started; drives `elapsed` and the
    /// completion-duration stat.
    pub mode_since: Option<Instant>,
    /// Retained through an emergency mode reset so playback history can still
    /// be finalized with the real duration.
    pub play_started: Option<Instant>,
    pub last_macro: String,
    pub recorded_count: i64,
    pub indicator_alive: bool,
}

impl Default for Runtime {
    fn default() -> Self {
        Self {
            mode: "idle".to_string(),
            mode_since: None,
            play_started: None,
            last_macro: String::new(),
            recorded_count: 0,
            indicator_alive: false,
        }
    }
}

/// Whether a guard is read by the OCR engine rather than the pixel detector.
/// `"text"` is a legacy spelling of the same method that the editor still accepts
/// on load; without it here such a guard reaches a detector that has no text path
/// at all and silently never matches.
fn is_text_guard(g: &Guard) -> bool {
    g.method == "ocr" || g.method == "text"
}

/// Widen a region by a quiet zone before handing it to the recognizer, clamped to
/// the frame. Returns the crop and its top-left in screen space.
///
/// Measured on this machine: of twelve words read off a full desktop, cropping to
/// each word's own bounding box read one of them back correctly, while the same
/// words with a margin read seven. The recognizer needs background around a glyph
/// to find its edges, and a region the user drew snugly around a line of text
/// leaves it none. The margin scales with the box because text is bigger on a
/// bigger display, and is capped so a large region is not quietly turned into a
/// search of its surroundings.
fn text_roi(frame: &Frame, x1: i32, y1: i32, x2: i32, y2: i32) -> Option<(Frame, (i64, i64))> {
    let pad = ((x2 - x1).min(y2 - y1) / 3).clamp(6, 24);
    let x = (x1 - pad).max(0);
    let y = (y1 - pad).max(0);
    let w = (x2 + pad).min(frame.width as i32) - x;
    let h = (y2 + pad).min(frame.height as i32) - y;
    frame.crop(x, y, w, h).map(|c| (c, (x as i64, y as i64)))
}

/// One box for the live detection overlay: what a trigger just found, in the
/// shape something can be drawn from.
///
/// [`Detection`] carries a *centre*, which is what a click needs and the wrong
/// end of a rectangle to draw from, and a template file name, which is not what
/// the user called the trigger. Both are converted here, once, so the overlay
/// page is only ever handed numbers it can use directly.
#[derive(Debug, Clone, Serialize)]
pub struct Sighting {
    /// The trigger's name, as the user typed it.
    pub label: String,
    /// Top-left corner and size, in capture pixels.
    pub x: i64,
    pub y: i64,
    pub w: i64,
    pub h: i64,
    pub confidence: f64,
}

/// The most recent detection pass from one source, held for the overlay to read.
struct Sightings {
    /// The frame these coordinates are in. The overlay scales by it rather than
    /// assuming the capture matched the monitor, because the configured capture
    /// resolution is free to disagree with the screen it is drawn back onto.
    frame: (i64, i64),
    at: Instant,
    items: Vec<Sighting>,
}

/// One sighting as the overlay reads it: its own age, because the sources run at
/// their own rates and a box has to fade on the clock of the pass that found it.
#[derive(Serialize)]
struct Aged<'a> {
    #[serde(flatten)]
    sighting: &'a Sighting,
    age_ms: u64,
}

/// How long a source's last pass keeps contributing boxes. Past this the overlay
/// would be drawing what a detector saw before it stopped running: a guard
/// engine that has shut down leaves its last pass behind, and it should go.
const SIGHTING_TTL_MS: u128 = 1_000;

/// The vision stack: one screen capture and one [`Detector`], both lazily
/// created and shared by every consumer, the Rust seat of Python's
/// `_get_capture`/`_get_detector`, which the sidecar had turned into a
/// subprocess with its own copy of each.
///
/// The two mutexes are deliberately never held at once. A caller grabs a frame,
/// releases the capture, then detects, so a full-screen scale sweep cannot block
/// the status heartbeat's FPS read, the property the cached [`Vision::fps`]
/// used to exist to buy back from the sidecar's single io lock.
pub struct Vision {
    detector: Mutex<Detector>,
    capture: Mutex<Option<ScreenCapture>>,
    /// Set once the capture is open. Read *before* the capture mutex so that
    /// [`Vision::ensure_ready`] on an already-open capture returns without
    /// queueing behind an in-flight [`Vision::grab`]: a grab can sit in the
    /// backend for the length of a frame timeout, and a start button that waits
    /// on that reads as a hang.
    ready: AtomicBool,
    /// The backend the capture actually opened, which may not be the one asked
    /// for (dxcam falls back to GDI).
    resolved: Mutex<Option<String>>,
    fps: Mutex<f64>,
    log: Arc<Mutex<LogBuffer>>,
    /// Set once the first missing-language-pack failure has been logged, so a
    /// machine without OCR reports the reason a single time instead of every
    /// poll cycle.
    ocr_warned: Mutex<bool>,
    /// Guard templates already reported as unreadable, so a broken picture logs
    /// once rather than on every poll cycle for the length of a run.
    template_warned: Mutex<HashSet<String>>,
    /// What each detection path last saw, for the on-screen overlay to draw,
    /// keyed by the path. Kept apart rather than merged on the way in: the guard
    /// loop polls at 5Hz and a running step polls at its own rate, so a shared
    /// slot would have each one wiping the other's boxes several times a second.
    sightings: Mutex<HashMap<&'static str, Sightings>>,
}

impl Vision {
    fn new(log: Arc<Mutex<LogBuffer>>) -> Self {
        Self {
            // Resized by `ensure_ready` before anything detects; a zero screen
            // resolves every percentage region to an empty crop, so a detection
            // that somehow raced ahead finds nothing rather than misreading.
            detector: Mutex::new(Detector::new(0, 0)),
            capture: Mutex::new(None),
            ready: AtomicBool::new(false),
            resolved: Mutex::new(None),
            fps: Mutex::new(0.0),
            log,
            ocr_warned: Mutex::new(false),
            template_warned: Mutex::new(HashSet::new()),
            sightings: Mutex::new(HashMap::new()),
        }
    }

    /// Open the capture and point the detector at the target screen if that has
    /// not happened yet, logging `Capture ready (<resolved backend>)` exactly
    /// once. Idempotent, and infallible: `ScreenCapture::new` falls back to GDI
    /// rather than failing, so there is nothing here for a caller to handle.
    pub fn ensure_ready(&self, screen_w: i64, screen_h: i64, backend: &str) {
        // Re-point the detector at the live screen on EVERY call, ahead of the
        // ready fast-path: a percentage watch region expands against these dims,
        // so a resolution change — or a first call that finds the capture slot
        // already open and would otherwise return with the detector still 0×0 —
        // must not leave it expanding against stale dimensions. That is the
        // failure where a limited watch area silently crops to the wrong
        // rectangle while "Anywhere" still detects fine.
        self.detector.lock().unwrap().set_screen(screen_w, screen_h);
        if self.ready.load(Ordering::SeqCst) {
            return;
        }
        let mut slot = self.capture.lock().unwrap();
        if slot.is_some() {
            self.ready.store(true, Ordering::SeqCst);
            return;
        }
        let cap = ScreenCapture::new(backend, None);
        let resolved = cap.backend().to_string();
        *slot = Some(cap);
        self.ready.store(true, Ordering::SeqCst);
        drop(slot);

        *self.resolved.lock().unwrap() = Some(resolved.clone());
        if let Ok(mut log) = self.log.lock() {
            log.push("ok", format!("Capture ready ({resolved})"));
        }
    }

    /// One frame, and the cached FPS refreshed while the capture is in hand.
    /// Opens the capture cold if a caller reached here without `ensure_ready`,
    /// exactly as Python's lazy `_get_capture` did.
    fn grab(&self) -> Option<Frame> {
        let (frame, fps, backend) = {
            let mut slot = self.capture.lock().unwrap();
            let cap = slot.get_or_insert_with(|| ScreenCapture::new("dxcam", None));
            (cap.grab(), round1(cap.fps()), cap.backend().to_string())
        };
        self.ready.store(true, Ordering::SeqCst);
        *self.fps.lock().unwrap() = fps;
        let mut resolved = self.resolved.lock().unwrap();
        if resolved.is_none() {
            *resolved = Some(backend);
        }
        frame
    }

    /// One grab, every guard: the guard poll loop's detect callback. Preserves
    /// the poll cycle's temporal consistency (every guard in a cycle sees the
    /// same instant) that `_detect_guards` had by grabbing once per RPC. Only
    /// non-empty results are inserted; the engine treats a missing key as no
    /// detection.
    ///
    /// `source` names the loop asking (the playback guard engine or the standing
    /// watcher), so that when both are running their overlay boxes sit in their
    /// own slots instead of overwriting each other every poll.
    pub fn detect_guards_faithful(
        &self,
        source: &'static str,
        guards: &[Guard],
    ) -> HashMap<String, Vec<Detection>> {
        // Text guards are read by the platform OCR engine, not the pixel
        // detector; the sidecar's EasyOCR path was excluded from the shipped
        // build, so every `method = "ocr"` guard used to fail on import.
        let (text, pixel): (Vec<&Guard>, Vec<&Guard>) =
            guards.iter().partition(|g| is_text_guard(g));

        let mut map = HashMap::new();
        if text.is_empty() && pixel.is_empty() {
            return map;
        }
        let Some(frame) = self.grab() else {
            return map;
        };

        if !pixel.is_empty() {
            let mut detector = self.detector.lock().unwrap();
            for guard in pixel {
                match detector.detect_guard(&frame, guard) {
                    Ok(hits) if !hits.is_empty() => {
                        map.insert(guard.id.clone(), hits);
                    }
                    Ok(_) => {}
                    Err(e) => self.warn_template(guard, &e),
                }
            }
        }
        if !text.is_empty() {
            map.extend(self.detect_text_guards(&frame, &text));
        }
        self.record_sightings(source, &frame, guards, &map);
        map
    }

    /// Publish the pass for the live overlay.
    ///
    /// Recorded even when the pass found nothing: an empty pass is how the boxes
    /// come off the screen the instant the thing stops being there, instead of
    /// hanging around until they age out.
    fn record_sightings(
        &self,
        source: &'static str,
        frame: &Frame,
        guards: &[Guard],
        hits: &HashMap<String, Vec<Detection>>,
    ) {
        let mut items = Vec::new();
        for guard in guards {
            let Some(dets) = hits.get(&guard.id) else {
                continue;
            };
            items.extend(dets.iter().map(|d| Sighting {
                label: guard.name.clone(),
                x: d.x - d.w / 2,
                y: d.y - d.h / 2,
                w: d.w,
                h: d.h,
                confidence: round3(d.confidence),
            }));
        }
        self.publish(source, frame, items);
    }

    /// Publish what a running step just found, under the step's own name.
    ///
    /// The checkpoint and AI-step detectors are the other half of "what the macro
    /// is looking at": a `wait_for` can sit on the same target for its whole
    /// timeout, and that is exactly the stretch where seeing the box matters.
    fn record_step(&self, frame: &Frame, label: &str, dets: &[Detection]) {
        let items = dets
            .iter()
            .map(|d| Sighting {
                label: label.to_string(),
                x: d.x - d.w / 2,
                y: d.y - d.h / 2,
                w: d.w,
                h: d.h,
                confidence: round3(d.confidence),
            })
            .collect();
        self.publish("steps", frame, items);
    }

    fn publish(&self, source: &'static str, frame: &Frame, items: Vec<Sighting>) {
        self.sightings.lock().unwrap().insert(
            source,
            Sightings {
                frame: (frame.width as i64, frame.height as i64),
                at: Instant::now(),
                items,
            },
        );
    }

    /// Every live source's boxes, each carrying its own age: everything the
    /// overlay page draws from.
    ///
    /// Age rather than a timestamp: the two ends share no clock worth trusting,
    /// and the page only ever asks how stale a box is so it knows how far to fade
    /// it. The frame size comes from the freshest source, since a pass that just
    /// landed is the one whose capture settings are current.
    pub fn sightings(&self) -> Value {
        let slots = self.sightings.lock().unwrap();
        let mut screen = (0, 0);
        let mut newest = u128::MAX;
        let mut items = Vec::new();
        for s in slots.values() {
            let age = s.at.elapsed().as_millis();
            if age > SIGHTING_TTL_MS || s.items.is_empty() {
                continue;
            }
            if age < newest {
                newest = age;
                screen = s.frame;
            }
            items.extend(s.items.iter().map(|sighting| Aged {
                sighting,
                age_ms: age as u64,
            }));
        }
        json!({ "screen": [screen.0, screen.1], "items": items })
    }

    /// Report a guard whose picture will not decode, once per guard and file,
    /// because a poll loop would otherwise repeat it several times a second for
    /// the whole run.
    fn warn_template(&self, guard: &Guard, err: &VisionError) {
        let key = format!("{}\u{0}{}", guard.id, guard.template_path);
        if self.template_warned.lock().unwrap().insert(key) {
            if let Ok(mut log) = self.log.lock() {
                log.push(
                    "error",
                    format!("Trigger '{}' picture failed: {err}", guard.name),
                );
            }
        }
    }

    /// Read every text guard out of the cycle's frame.
    ///
    /// One grab serves all of them: each guard's percentage region is cropped
    /// out of the same frame, so N text guards cost no extra capture. Guards
    /// watching the *same* area are also recognized together: running the OS
    /// recognizer is the single most expensive thing the watcher does, and two
    /// triggers both scanning the whole screen used to pay for it twice a pass.
    /// Failure is per-region and silent-but-logged, matching `detect_guard`'s
    /// "never raises, returns []" contract: a guard that cannot be read must
    /// not abort the poll cycle.
    fn detect_text_guards(
        &self,
        frame: &Frame,
        guards: &[&Guard],
    ) -> HashMap<String, Vec<Detection>> {
        let mut out = HashMap::new();
        if !ocr::available() {
            let mut warned = self.ocr_warned.lock().unwrap();
            if !*warned {
                *warned = true;
                if let Ok(mut log) = self.log.lock() {
                    log.push(
                        "error",
                        "Text triggers need a Windows OCR language pack \
                         (Settings › Time & language › Language › Add a language)"
                            .to_string(),
                    );
                }
            }
            return out;
        }

        // Bucket by watch area first. The key is the stored percentage corners
        // bit-for-bit, so the common case (several triggers all left on
        // "anywhere") collapses to a single recognition pass.
        let mut groups: Vec<([u64; 4], Vec<&Guard>)> = Vec::new();
        for &guard in guards {
            // Same early-out as `guard.py`: an empty needle matches nothing.
            if guard.ocr_text.is_empty() {
                continue;
            }
            let key = guard.region.map(f64::to_bits);
            match groups.iter_mut().find(|(k, _)| *k == key) {
                Some((_, members)) => members.push(guard),
                None => groups.push((key, vec![guard])),
            }
        }

        for (_, members) in &groups {
            let head = members[0];
            let (x1, y1, x2, y2) =
                region_pixels(head.region, frame.width as i32, frame.height as i32);
            // The whole-screen case reads the captured frame in place; only a
            // drawn region pays for a crop.
            let cropped;
            let (roi, origin) = if is_full_region(head.region) {
                (frame, (0, 0))
            } else {
                match text_roi(frame, x1, y1, x2, y2) {
                    Some((c, o)) => {
                        cropped = c;
                        (&cropped, o)
                    }
                    None => continue,
                }
            };
            let lines = match ocr::read_lines(roi) {
                Ok(lines) => lines,
                Err(e) => {
                    if let Ok(mut log) = self.log.lock() {
                        log.push("error", format!("Text trigger '{}' failed: {e}", head.name));
                    }
                    continue;
                }
            };
            for guard in members {
                let dets = ocr::match_lines(&lines, &guard.ocr_text, origin);
                if !dets.is_empty() {
                    out.insert(guard.id.clone(), dets);
                }
            }
        }
        out
    }

    /// One poll of a running checkpoint: the play loop's per-poll detect
    /// callback. `play_macro` opens the capture before playback, so this only
    /// ever grabs. A frame that will not capture yields no detections and the
    /// checkpoint times out, as it did when the sidecar went away mid-play.
    pub fn detect_checkpoint(&self, cfg: &Value) -> Vec<Detection> {
        let Some(frame) = self.grab() else {
            return Vec::new();
        };
        let hits = self.detector.lock().unwrap().detect_checkpoint(&frame, cfg);
        let label = cfg
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("Checkpoint");
        self.record_step(&frame, label, &hits);
        hits
    }

    /// One AI-step detection against a fresh frame: the executor's `find_click`/
    /// `wait_for` detect callback. Returns the matches and the message the run log
    /// prints. A capture failure yields `(none, "")`, degrading a `find_click` to
    /// "nothing found"; Python instead crashed the run thread on a `None` frame,
    /// leaking the mode, so this is the cleaner resolution of the same case.
    pub fn ai_detect(&self, step: &Step) -> (Vec<crate::engine::ai::Match>, String) {
        let Some(frame) = self.grab() else {
            return (Vec::new(), String::new());
        };
        let (hits, message) = self.detector.lock().unwrap().ai_detect(&frame, step);
        let label = if step.label.is_empty() {
            &step.step_type
        } else {
            &step.label
        };
        self.record_step(&frame, label, &hits);
        (hits.iter().map(to_match).collect(), message)
    }

    /// Dry-run one guard for the editor's Test button, the port of
    /// `Api.guard_test`. Text guards take the OCR path (see
    /// [`Vision::guard_test_text`]); both assemble the same result dict, so the
    /// editor renders either without knowing which engine answered.
    pub fn guard_test(&self, screen_w: i64, screen_h: i64, backend: &str, guard: &Guard) -> Value {
        self.ensure_ready(screen_w, screen_h, backend);
        if is_text_guard(guard) {
            return self.guard_test_text(guard);
        }
        let Some(frame) = self.grab() else {
            return json!({ "ok": false, "error": "Could not capture frame" });
        };
        let matches = match self.detector.lock().unwrap().detect_guard(&frame, guard) {
            Ok(m) => m,
            Err(e) => return json!({ "ok": false, "error": e.to_string() }),
        };

        let region = region_pixels(guard.region, frame.width as i32, frame.height as i32);
        let preview = preview::annotate(&frame, region, &matches);
        let best = matches.first();
        let conf = best.map(|d| round3(d.confidence)).unwrap_or(0.0);
        json!({
            "ok": !matches.is_empty(),
            "matched": matches.len(),
            "found_x": best.map(|d| d.x).unwrap_or(-1),
            "found_y": best.map(|d| d.y).unwrap_or(-1),
            "confidence": conf,
            "message": if matches.is_empty() {
                "no match".to_string()
            } else {
                format!("{} match(es) · conf {conf:.2}", matches.len())
            },
            "preview": preview,
            // The sidecar's cv2 preview was a JPEG; this one is a PNG.
            "preview_mime": "image/png",
        })
    }

    /// The text-guard half of `guard_test`: grab, OCR the region, and draw the
    /// same annotated preview a pixel guard gets.
    fn guard_test_text(&self, guard: &Guard) -> Value {
        let fail = |msg: String| {
            json!({
                "ok": false, "matched": 0, "found_x": -1, "found_y": -1,
                "confidence": 0.0, "message": msg, "preview": "",
            })
        };
        if !ocr::available() {
            return fail(
                "No Windows OCR language pack installed. Add one in \
                 Settings › Time & language › Language."
                    .to_string(),
            );
        }
        if guard.ocr_text.is_empty() {
            return fail("Type the words to look for first".to_string());
        }

        let Some(frame) = self.grab() else {
            return fail("Could not capture frame".to_string());
        };

        let (x1, y1, x2, y2) = region_pixels(guard.region, frame.width as i32, frame.height as i32);
        let (roi, origin) = if is_full_region(guard.region) {
            (frame.clone(), (0, 0))
        } else {
            match text_roi(&frame, x1, y1, x2, y2) {
                Some(c) => c,
                None => return fail("That region is off screen".to_string()),
            }
        };

        let matches = match ocr::find_text(&roi, &guard.ocr_text, origin) {
            Ok(m) => m,
            Err(e) => return fail(e.to_string()),
        };
        let preview = preview::annotate(&frame, (x1, y1, x2, y2), &matches);
        let best = matches.first();
        json!({
            "ok": !matches.is_empty(),
            "matched": matches.len(),
            "found_x": best.map(|d| d.x).unwrap_or(-1),
            "found_y": best.map(|d| d.y).unwrap_or(-1),
            "confidence": if best.is_some() { 1.0 } else { 0.0 },
            "message": match best {
                Some(d) => format!("found “{}”", d.label),
                None => "no match".to_string(),
            },
            "preview": preview,
            "preview_mime": "image/png",
        })
    }

    /// Dry-run one AI step for the step editor's Test button, the port of
    /// `Api.ai_test_step`. The step is parsed *before* the frame is grabbed, so a
    /// malformed step reports "Bad step: …" even when capture would also fail;
    /// that is the monolith's order, and it differs from `guard_test`'s.
    pub fn ai_test_step(&self, screen_w: i64, screen_h: i64, backend: &str, step: &Value) -> Value {
        self.ensure_ready(screen_w, screen_h, backend);
        let st: Step = match serde_json::from_value(step.clone()) {
            Ok(s) => s,
            Err(e) => return json!({ "ok": false, "error": format!("Bad step: {e}") }),
        };
        let Some(frame) = self.grab() else {
            return json!({ "ok": false, "error": "Could not capture frame" });
        };

        // Detection runs once and feeds both the verdict and the preview rings.
        // Python re-detected colour on the preview *after* drawing its region box,
        // which could only ever add matches invented by its own annotation.
        let detected = matches!(st.step_type.as_str(), "find_click" | "wait_for");
        let (matches, message) = if detected {
            self.detector.lock().unwrap().ai_detect(&frame, &st)
        } else {
            (Vec::new(), String::new())
        };

        let crosshair = if st.step_type == "click" {
            Some((st.x, st.y))
        } else {
            None
        };
        let region = region_pixels(st.region, frame.width as i32, frame.height as i32);
        let preview = preview::annotate_step(&frame, region, &matches, crosshair);

        let mut result = step_verdict(&st, &matches, &message);
        result["preview"] = json!(preview);
        result["preview_mime"] = json!("image/png");
        result
    }

    /// Import an image file the user picks into `templates_dir` as a template
    /// PNG (`_add_template_image`), minus the sidecar and its tkinter dialog.
    pub fn add_template_image(&self, app: &tauri::AppHandle, templates_dir: &Path) -> Value {
        use tauri_plugin_dialog::DialogExt;

        let picked = app
            .dialog()
            .file()
            .set_title("Select button image")
            .add_filter("Images", &["png", "jpg", "jpeg", "bmp", "webp"])
            .add_filter("All files", &["*"])
            .blocking_pick_file()
            .and_then(|f| f.into_path().ok());
        let Some(file) = picked else {
            return json!({ "ok": false, "error": "cancelled" });
        };
        crate::hardware::picker::import_template(templates_dir, &file)
    }

    /// The backend the capture actually opened (`None` until it is opened).
    /// Mirrors `_capture.backend if _capture else None`.
    pub fn resolved_backend(&self) -> Option<String> {
        self.resolved.lock().unwrap().clone()
    }

    /// The last cached capture FPS: a pure read, safe from the heartbeat.
    pub fn capture_fps_cached(&self) -> f64 {
        *self.fps.lock().unwrap()
    }
}

/// A detection as the AI executor wants it: just the point and how sure we are.
fn to_match(d: &Detection) -> crate::engine::ai::Match {
    crate::engine::ai::Match {
        x: d.x,
        y: d.y,
        confidence: d.confidence,
    }
}

/// The dry-run verdict for one step (`steps.py::test_step`), without the preview
/// the caller layers on. Every arm reports what the step *would* do; only
/// `find_click`/`wait_for` consult `matches`.
fn step_verdict(st: &Step, matches: &[Detection], message: &str) -> Value {
    let plain = |ok: bool, msg: String| {
        json!({ "ok": ok, "message": msg, "found_x": -1, "found_y": -1,
                "matched": 0, "confidence": 0.0 })
    };
    match st.step_type.as_str() {
        "find_click" | "wait_for" => match matches.first() {
            Some(best) => json!({
                "ok": true,
                "message": format!("would click ({}, {}): {message}", best.x, best.y),
                "found_x": best.x,
                "found_y": best.y,
                "matched": matches.len(),
                "confidence": round3(best.confidence),
            }),
            None => plain(false, format!("nothing found: {message}")),
        },
        "click" => json!({
            "ok": true,
            "message": format!("would click ({}, {})", st.x, st.y),
            "found_x": st.x, "found_y": st.y, "matched": 0, "confidence": 0.0,
        }),
        "key" => plain(true, format!("would press '{}'", st.key)),
        "type" => plain(true, format!("would type '{}'", st.text)),
        // Python's `{:+d}`: the sign is always shown, so `-3` reads as a scroll
        // down and `+3` as a scroll up.
        "scroll" => plain(true, format!("would scroll {:+}", st.scroll_amount)),
        "delay" => plain(true, format!("would wait {}s", py_float(st.delay))),
        _ => plain(false, "unknown step type".to_string()),
    }
}

/// The wiring hub. Clone is cheap; every field is an `Arc`.
#[derive(Clone)]
pub struct Core {
    pub config: Arc<Mutex<MacroConfig>>,
    pub log: Arc<Mutex<LogBuffer>>,
    pub runtime: Arc<Mutex<Runtime>>,
    /// Per-macro play counts and execution history (`config/stats.json`).
    pub play_stats: Arc<PlayStats>,
    /// Reused across plays (Python recreates a `MacroPlayer` each time; one
    /// persistent player is equivalent and lets guards hold a stable handle).
    pub player: Arc<MacroPlayer>,
    pub controller: Arc<InputController>,
    /// Serialized, focus-verified input path for Watch and Loops. Recorded
    /// playback deliberately continues to use `controller` directly.
    pub reliable_input: Arc<ReliableInput>,
    /// `Some` only while recording.
    pub recorder: Arc<Mutex<Option<MacroRecorder>>>,
    pub vision: Arc<Vision>,
    /// One permanent guard engine (Python recreates it per `_start_guards`).
    pub guard_engine: Arc<GuardEngine>,
    /// Tray toasts (playback-complete / scheduled runs); its app handle is bound
    /// in `setup()`, so toasts fired before the UI is up harmlessly no-op.
    pub notifier: Arc<Notifier>,
    /// The transparent pixel-cat recording overlay; its app handle is bound in
    /// `setup()` too, so a `sync` before the window exists no-ops. Driven by
    /// `set_mode`, mirroring Python's `_sync_indicator`.
    pub indicator: Arc<crate::shell::indicator::Indicator>,
    /// The fullscreen overlay that draws a box round everything the triggers are
    /// finding. Armed by whichever detection loop is running, so it is up exactly
    /// when there is something to see.
    pub detections: Arc<crate::shell::detections::Detections>,
    /// Cancellation flag for edited steps and the directed node executor.
    pub node_running: Arc<AtomicBool>,
}

impl Core {
    pub fn new(config: MacroConfig) -> Self {
        let config = Arc::new(Mutex::new(config));
        let log = Arc::new(Mutex::new(LogBuffer::default()));
        let runtime = Arc::new(Mutex::new(Runtime::default()));
        let play_stats = Arc::new(PlayStats::new(paths::config_dir().join("stats.json")));
        let player = Arc::new(MacroPlayer::new());
        let controller = Arc::new(InputController::new());
        let reliable_input = Arc::new(ReliableInput::new(controller.clone()));
        let recorder = Arc::new(Mutex::new(None));
        let vision = Arc::new(Vision::new(log.clone()));
        let notifier = Arc::new(Notifier::new());
        let indicator = Arc::new(crate::shell::indicator::Indicator::new());
        let detections = Arc::new(crate::shell::detections::Detections::new());
        let node_running = Arc::new(AtomicBool::new(false));

        // Guard-engine callbacks capture leaf Arcs only (never `Core`) so a guard
        // pause/resume/click drives the same player and controller a macro plays
        // through, with no reference cycle.
        let detect: Detect = {
            let vision = vision.clone();
            Box::new(move |guards: &[Guard]| vision.detect_guards_faithful("guards", guards))
        };
        let player_state: PlayerState = {
            let player = player.clone();
            Box::new(move || (player.is_playing(), player.is_paused()))
        };
        let actuate: Actuate = {
            let player = player.clone();
            let controller = controller.clone();
            Box::new(move |action| execute_action(&player, &controller, action))
        };
        let on_fire: OnFire = {
            let log = log.clone();
            Box::new(move |g: &Guard, _x: i64, _y: i64| {
                if let Ok(mut log) = log.lock() {
                    log.push(
                        "warn",
                        format!("Guard '{}' fired: handling, then resuming", g.name),
                    );
                }
            })
        };
        let guard_engine = Arc::new(GuardEngine::new(
            detect,
            player_state,
            actuate,
            Some(on_fire),
        ));

        let core = Self {
            config,
            log,
            runtime,
            play_stats,
            player,
            controller,
            reliable_input,
            recorder,
            vision,
            guard_engine,
            notifier,
            indicator,
            detections,
            node_running,
        };

        // DPI self-check at startup: probe on a fresh worker thread and log the
        // verdict, so a scaling regression names itself here ("DPI MISMATCH...")
        // instead of being inferred later from missed clicks.
        let (healthy, line) = crate::hardware::dpi::report();
        core.emit(if healthy { "ok" } else { "warn" }, line);
        core
    }

    /// Append a log entry (mirrors `Api._emit`; the UI pulls the tail via
    /// `get_status`).
    pub fn emit(&self, level: &str, msg: impl Into<String>) {
        if let Ok(mut log) = self.log.lock() {
            log.push(level, msg);
        }
    }

    /// Record the mode and the instant it began, then sync the recording
    /// indicator. A success toast is emitted only after the player returns an
    /// explicit completed outcome.
    pub fn set_mode(&self, mode: &str) {
        {
            let mut rt = self.runtime.lock().unwrap();
            rt.mode = mode.to_string();
            rt.mode_since = Some(Instant::now());
        }
        self.sync_indicator();
    }

    /// Apply the live indicator preference without changing the current mode or
    /// resetting its elapsed timer.
    pub fn sync_indicator(&self) {
        let mode = self.runtime.lock().unwrap().mode.clone();
        let enabled = self.config.lock().unwrap().indicator_on_top;
        self.indicator.sync(&mode, enabled);
    }

    // ── Global hotkeys (TinyTask-style) ──────────────────────────────────────

    /// Toggle recording from the global hotkey, `Api.hotkey_record`. (Python
    /// wraps this in a `try/except`, but that only guards the `keyboard` hook
    /// against a thrown exception; `start_record`/`stop_record` return values
    /// here rather than raising, so the error branch is unreachable and unported.)
    pub fn hotkey_record(&self) {
        let mode = self.runtime.lock().unwrap().mode.clone();
        if mode == "recording" {
            self.stop_record();
        } else if mode == "idle" {
            self.start_record();
        } else {
            self.emit("warn", format!("Can't record while {mode}"));
        }
    }

    /// Play the most recent macro from the global hotkey, `Api.hotkey_play`.
    /// Uses the last-played macro, falling back to the newest on disk, and plays
    /// it with its saved loop settings (`repeat=None`).
    pub fn hotkey_play(&self) {
        let mode = self.runtime.lock().unwrap().mode.clone();
        if mode != "idle" {
            self.emit("warn", format!("Can't play while {mode}"));
            return;
        }
        let last = self.runtime.lock().unwrap().last_macro.clone();
        let name = if last.is_empty() {
            self.most_recent_macro_name()
        } else {
            last
        };
        if name.is_empty() {
            self.emit("warn", "No macro to play - record one first");
            return;
        }
        self.play_macro(&name, None, 1.0);
    }

    /// Stem of the newest `*.json` in the macros dir (`Api._most_recent_macro_name`).
    fn most_recent_macro_name(&self) -> String {
        let mut files: Vec<(PathBuf, SystemTime)> = match std::fs::read_dir(paths::macros_dir()) {
            Ok(entries) => entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("json"))
                .map(|p| {
                    let mtime = std::fs::metadata(&p)
                        .and_then(|m| m.modified())
                        .unwrap_or(UNIX_EPOCH);
                    (p, mtime)
                })
                .collect(),
            Err(_) => Vec::new(),
        };
        files.sort_by(|a, b| b.1.cmp(&a.1));
        files
            .first()
            .and_then(|(p, _)| p.file_stem())
            .and_then(|s| s.to_str())
            .map(String::from)
            .unwrap_or_default()
    }

    /// Primary screen size, falling back to the configured resolution when Win32
    /// reports nothing (`Api._get_screen_resolution`).
    pub fn resolve_screen(&self) -> (u32, u32) {
        let (w, h) = crate::hardware::screen_size();
        if w > 0 && h > 0 {
            (w, h)
        } else {
            self.config.lock().unwrap().resolution
        }
    }

    /// Keys held out of the recording: the record/play/stop hotkeys, lowercased
    /// (`Api._hotkey_ignore_set`), so pressing a hotkey to stop isn't captured.
    fn hotkey_ignore_set(&self) -> HashSet<String> {
        let c = self.config.lock().unwrap();
        [
            c.hotkey_record.to_lowercase(),
            c.hotkey_play.to_lowercase(),
            c.hotkey_stop.to_lowercase(),
        ]
        .into_iter()
        .collect()
    }

    /// Begin recording, `Api.start_record`. Refuses unless idle, then arms a
    /// fresh recorder at the live screen resolution.
    pub fn start_record(&self) -> Value {
        {
            let mode = self.runtime.lock().unwrap().mode.clone();
            if mode != "idle" {
                return json!({ "ok": false, "error": format!("Busy ({mode})") });
            }
        }
        let resolution = self.resolve_screen();
        let ignore = self.hotkey_ignore_set();
        {
            let mut slot = self.recorder.lock().unwrap();
            let mut recorder = MacroRecorder::new(resolution, ignore);
            recorder.start();
            *slot = Some(recorder);
        }
        self.set_mode("recording");
        let hk = self.config.lock().unwrap().hotkey_record.to_uppercase();
        self.emit(
            "rec",
            format!("Recording started. Press {hk} or Stop to finish"),
        );
        json!({ "ok": true })
    }

    /// Stop recording, save the macro as `macro_<unix>`, and report it:
    /// `Api.stop_record`.
    pub fn stop_record(&self) -> Value {
        let mode = self.runtime.lock().unwrap().mode.clone();
        let mut slot = self.recorder.lock().unwrap();
        if !matches!(mode.as_str(), "recording" | "paused") || slot.is_none() {
            return json!({ "ok": false, "error": "Not recording" });
        }
        let mut macro_def = slot.as_mut().unwrap().stop();
        drop(slot);

        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        macro_def.name = format!("macro_{stamp}");
        let path = paths::macros_dir().join(format!("{}.json", macro_def.name));
        if let Err(e) = macro_def.save_to(&path) {
            // `Api.stop_record` calls `macro.save()` unguarded: a failure raises
            // before the mode resets, so we surface it and stay in `recording`.
            return json!({ "ok": false, "error": e.to_string() });
        }

        let events = macro_def.events.len();
        {
            let mut rt = self.runtime.lock().unwrap();
            rt.last_macro = macro_def.name.clone();
            rt.recorded_count = events as i64;
        }
        self.set_mode("idle");
        let (w, h) = macro_def.record_resolution;
        self.emit(
            "ok",
            format!(
                "Saved {} ({events} events, {:.1}s)",
                macro_def.name,
                macro_def.duration()
            ),
        );
        json!({
            "ok": true,
            "name": macro_def.name,
            "events": events,
            "duration": round1(macro_def.duration()),
            "resolution": format!("{w}x{h}"),
        })
    }

    /// Toggle pause during recording, `Api.pause_record`. Paused time is excluded
    /// from event timestamps by the recorder, so playback stays seamless.
    pub fn pause_record(&self) -> Value {
        let mode = self.runtime.lock().unwrap().mode.clone();
        let mut slot = self.recorder.lock().unwrap();
        if !matches!(mode.as_str(), "recording" | "paused") || slot.is_none() {
            return json!({ "ok": false, "error": "Not recording" });
        }
        let recorder = slot.as_mut().unwrap();
        recorder.toggle_pause();
        let paused = recorder.is_paused();
        drop(slot);
        self.set_mode(if paused { "paused" } else { "recording" });
        self.emit(
            "info",
            if paused {
                "Recording paused"
            } else {
                "Recording resumed"
            },
        );
        json!({ "ok": true, "paused": paused })
    }

    /// Play a macro, `Api.play_macro`. `repeat=None` uses the macro's saved loop
    /// settings; `0`/`""`/`∞` loop forever; `N` plays N times. Returns the same
    /// `{ok, name, events}` / `{ok:false, error}` shapes as the source.
    pub fn play_macro(&self, name: &str, repeat: Option<Value>, speed: f64) -> Value {
        {
            let mode = self.runtime.lock().unwrap().mode.clone();
            if mode != "idle" {
                return json!({ "ok": false, "error": format!("Busy ({mode})") });
            }
        }
        let stem = name.strip_suffix(".json").unwrap_or(name);
        let path = paths::macros_dir().join(format!("{stem}.json"));
        if !path.exists() {
            return json!({ "ok": false, "error": format!("Not found: {stem}") });
        }
        match crate::migrations::ensure_macro_current(&path) {
            Ok(true) => self.emit(
                "warn",
                format!("Upgraded legacy macro '{stem}' and restored a 10-second loop boundary (backup kept)"),
            ),
            Ok(false) => {}
            Err(error) => {
                self.emit("err", format!("Could not safely upgrade {stem}: {error}"));
                return json!({ "ok": false, "error": error });
            }
        }
        let mut macro_def = match Macro::load(&path) {
            Ok(m) => m,
            Err(e) => {
                self.emit("err", format!("Failed to load {stem}: {e}"));
                return json!({ "ok": false, "error": e.to_string() });
            }
        };
        if macro_def.events.is_empty() {
            return json!({ "ok": false, "error": "Macro has no events" });
        }

        let repeat = resolve_repeat(repeat.as_ref(), &macro_def);
        if repeat == 0 {
            macro_def.loop_enabled = true;
            macro_def.loop_count = 0;
        } else if repeat == 1 {
            macro_def.loop_enabled = false;
            macro_def.loop_count = 1;
        } else {
            macro_def.loop_enabled = true;
            macro_def.loop_count = repeat;
        }

        let macro_name = macro_def.name.clone();
        let events = macro_def.events.len();
        let target = self.resolve_screen();
        if let Err(error) = self.player.validate(&macro_def, target, speed) {
            self.emit("err", format!("Refused to play {macro_name}: {error}"));
            return json!({ "ok": false, "error": error });
        }
        if repeat != 1 && !has_fail_closed_checkpoint(&macro_def) {
            self.emit(
                "warn",
                format!(
                    "Safety: '{macro_name}' repeats without a fail-closed screen checkpoint; delivery failures will stop it, but the game result cannot be verified"
                ),
            );
        }

        self.runtime.lock().unwrap().last_macro = macro_name.clone();
        // Recorded at play-start so a stopped macro still counts; the duration and
        // final status are filled in on completion by `watch_playback`.
        self.play_stats.record_play(&macro_name, 0.0, "running");

        // Eager capture open: Python's `cap = self._get_capture()` runs before
        // the mode flip and the "Playing" log, so "Capture ready" lands there and
        // a live detector backs any checkpoints. `start_guards` then no-ops.
        let backend = self.config.lock().unwrap().capture_backend.clone();
        self.vision
            .ensure_ready(target.0 as i64, target.1 as i64, &backend);

        self.set_mode("playing");
        self.runtime.lock().unwrap().play_started = Some(Instant::now());
        let repeat_msg = if repeat == 0 {
            "inf".to_string()
        } else {
            repeat.to_string()
        };
        let speed_msg = if speed != 1.0 {
            format!("{}x", py_float(speed))
        } else {
            "1x".to_string()
        };
        self.emit(
            "play",
            format!(
                "Playing {macro_name} ({events} events, repeat: {repeat_msg}, speed: {speed_msg}, target: {}x{})",
                target.0, target.1
            ),
        );

        // play_macro wires a live detector: Python builds `MacroPlayer` with a
        // detector + frame_provider here, so checkpoints RUN. The vision-agent
        // runner builds a bare player and passes None, skipping them.
        let checkpoint: CheckpointDetect = {
            let vision = self.vision.clone();
            Box::new(move |cfg: &Value| vision.detect_checkpoint(cfg))
        };
        if let Err(error) = self.player.play(macro_def, target, speed, Some(checkpoint)) {
            self.play_stats.update_last_run(&macro_name, 0.0, "failed");
            self.runtime.lock().unwrap().play_started = None;
            self.set_mode("idle");
            self.emit("err", format!("Could not start {macro_name}: {error}"));
            return json!({ "ok": false, "error": error });
        }
        // Armed for the whole run, not just for guards: the checkpoints wired
        // above detect too, and they are the ones a macro built around a picture
        // spends its time waiting on.
        self.detections.set("play", true);
        self.start_guards(&macro_name);

        let core = self.clone();
        thread::spawn(move || core.watch_playback());

        json!({ "ok": true, "name": macro_name, "events": events })
    }

    /// Attach and start the guard engine for a macro that has enabled guards:
    /// `Api._start_guards`. Opens the capture first (logging `Capture ready`),
    /// then `N guard(s) active during playback`.
    fn start_guards(&self, macro_name: &str) {
        let path = paths::guards_dir().join(format!("{macro_name}.json"));
        let enabled: Vec<Guard> = GuardFile::load(&path)
            .guards
            .into_iter()
            .filter(|g| g.enabled)
            .collect();
        if enabled.is_empty() {
            return;
        }
        let (w, h) = self.resolve_screen();
        let backend = self.config.lock().unwrap().capture_backend.clone();
        self.vision.ensure_ready(w as i64, h as i64, &backend);
        let humanize = self.config.lock().unwrap().humanize_clicks;
        let count = enabled.len();
        self.guard_engine.set_humanize(humanize);
        self.guard_engine.start(enabled);
        self.detections.set("guards", true);
        self.emit("ok", format!("{count} guard(s) active during playback"));
    }

    pub(crate) fn stop_guards(&self) {
        self.guard_engine.stop();
        self.detections.set("guards", false);
    }

    /// Wait out the playback thread, persist its honest terminal outcome, then
    /// reset the managed playback mode. Emergency stops may already have reset a
    /// different subsystem's mode, so outcome handling is unconditional.
    fn watch_playback(&self) {
        let outcome = self.player.wait();
        self.stop_guards();
        // Runs on every exit (finished, stopped, or panicked out of the player),
        // so this is the one place the overlay has to come down.
        self.detections.set("play", false);
        let (playing, duration, last) = {
            let mut rt = self.runtime.lock().unwrap();
            let playing = rt.mode == "playing";
            let duration = rt
                .play_started
                .take()
                .map(|s| s.elapsed().as_secs_f64())
                .unwrap_or(0.0);
            let last = if rt.last_macro.is_empty() {
                "unknown".to_string()
            } else {
                rt.last_macro.clone()
            };
            (playing, duration, last)
        };
        self.play_stats
            .update_last_run(&last, duration, outcome.status());
        if playing {
            self.set_mode("idle");
        }
        match outcome {
            PlaybackOutcome::Completed { iterations } => {
                self.emit(
                    "ok",
                    format!("Playback finished after {iterations} repetition(s)"),
                );
                if self.config.lock().unwrap().notify_on_complete {
                    self.notifier.notify(
                        "Clawmation: Playback Complete",
                        &format!("'{last}' finished"),
                    );
                }
            }
            PlaybackOutcome::Stopped => self.emit("warn", "Playback stopped"),
            PlaybackOutcome::Failed(error) => self.emit("err", format!("Playback failed: {error}")),
        }
    }

    /// Stop the current playback (`Api.stop_playback`).
    pub fn stop_playback(&self) -> Value {
        let playing = self.runtime.lock().unwrap().mode == "playing";
        if playing {
            self.node_running.store(false, Ordering::SeqCst);
            self.stop_guards();
            self.player.stop();
            self.emit("warn", "Stopping playback...");
            json!({ "ok": true })
        } else {
            json!({ "ok": false, "error": "Not playing" })
        }
    }

    /// Run an edited step list in a background thread, `Api.steps_run`. The
    /// frontend's Run button passes only the steps (never loop settings), so this
    /// always plays them once. Refuses unless idle, then flips to `playing` and
    /// drives the executor off-thread. Returns `{ok:true}` immediately (the
    /// pass/fail summary arrives later as a log line), or the busy / bad-steps
    /// error.
    pub fn steps_run(&self, steps: Vec<Value>) -> Value {
        {
            let mode = self.runtime.lock().unwrap().mode.clone();
            if mode != "idle" {
                return json!({ "ok": false, "error": format!("Busy ({mode})") });
            }
        }
        let parsed: Result<Vec<Step>, _> = steps.into_iter().map(serde_json::from_value).collect();
        let step_objs = match parsed {
            Ok(objs) => objs,
            Err(e) => return json!({ "ok": false, "error": format!("Bad steps: {e}") }),
        };

        self.node_running.store(true, Ordering::SeqCst);
        self.set_mode("playing");
        let count = step_objs.len();
        self.emit("play", format!("Running {count} steps"));

        let core = self.clone();
        thread::spawn(move || {
            // Python opens the capture *inside* this thread (after the "Running N
            // steps" emit), unlike play_macro's eager pre-flip open, so any
            // "Capture ready" log lands after "Running N steps".
            let (w, h) = core.resolve_screen();
            let backend = core.config.lock().unwrap().capture_backend.clone();
            core.vision.ensure_ready(w as i64, h as i64, &backend);

            let detect: crate::engine::ai::Detect = {
                let vision = core.vision.clone();
                Box::new(move |step: &Step| vision.ai_detect(step))
            };
            let actuate: crate::engine::ai::Actuate = {
                let controller = core.controller.clone();
                let reliable_input = core.reliable_input.clone();
                let target = Arc::new(Mutex::new(None));
                Box::new(move |action| {
                    execute_ai_action(&reliable_input, &controller, &target, action)
                })
            };

            core.detections.set("steps", true);
            let summary = crate::engine::ai::run_with_flag(
                &step_objs,
                false,
                1,
                &detect,
                &actuate,
                &core.node_running,
            );
            core.detections.set("steps", false);
            core.node_running.store(false, Ordering::SeqCst);
            core.set_mode("idle");
            let ok = summary["ok"].as_bool().unwrap_or(false);
            let status = if ok {
                "finished"
            } else if summary["cancelled"].as_bool().unwrap_or(false) {
                "stopped"
            } else {
                "stopped (step failed)"
            };
            let passed = summary["steps_passed"].as_i64().unwrap_or(0);
            let run = summary["steps_run"].as_i64().unwrap_or(0);
            core.emit(
                if ok { "ok" } else { "warn" },
                format!("Steps {status}: {passed}/{run} passed"),
            );
        });

        json!({ "ok": true })
    }

    /// Dry-run one step against a fresh frame for the editor's Test button:
    /// `Api.steps_test`, which delegates to `ai_test_step`.
    pub fn steps_test(&self, step: Value) -> Value {
        let (w, h) = self.resolve_screen();
        let backend = self.config.lock().unwrap().capture_backend.clone();
        self.vision
            .ai_test_step(w as i64, h as i64, &backend, &step)
    }

    /// Validate and run a directed node graph on the shared controller.
    pub fn node_graph_run(&self, graph: Value, chains: Vec<Chain>) -> Value {
        {
            let mode = self.runtime.lock().unwrap().mode.clone();
            if mode != "idle" {
                return json!({ "ok": false, "error": format!("Busy ({mode})") });
            }
        }
        let graph: NodeGraph = match serde_json::from_value(graph) {
            Ok(graph) => graph,
            Err(error) => {
                return json!({ "ok": false, "error": format!("Bad node graph: {error}") })
            }
        };
        let report = graph.validate();
        if !report.ok {
            return json!({ "ok": false, "error": report.errors.join("; ") });
        }

        self.node_running.store(true, Ordering::SeqCst);
        self.set_mode("playing");
        self.emit("play", format!("Running node graph '{}'", graph.name));

        let core = self.clone();
        thread::spawn(move || {
            let (w, h) = core.resolve_screen();
            let backend = core.config.lock().unwrap().capture_backend.clone();
            core.vision.ensure_ready(w as i64, h as i64, &backend);

            let detect: crate::engine::ai::Detect = {
                let vision = core.vision.clone();
                Box::new(move |step: &Step| vision.ai_detect(step))
            };
            let actuate: crate::engine::ai::Actuate = {
                let controller = core.controller.clone();
                let reliable_input = core.reliable_input.clone();
                let target = Arc::new(Mutex::new(None));
                Box::new(move |action| {
                    execute_ai_action(&reliable_input, &controller, &target, action)
                })
            };
            let run_macro =
                |name: &str, embedded_steps: &[Step], repeat: i64| -> Result<(), String> {
                    let loaded_steps;
                    let steps = if embedded_steps.is_empty() {
                        let ai_path = paths::macros_dir().join("ai").join(format!("{name}.json"));
                        loaded_steps = if ai_path.exists() {
                            crate::models::step::AIMacro::load(&ai_path)
                                .map_err(|error| error.to_string())?
                                .steps
                        } else {
                            let path = paths::macros_dir().join(format!("{name}.json"));
                            let recorded = Macro::load(&path).map_err(|error| error.to_string())?;
                            crate::models::step::macro_to_steps(&recorded)
                        };
                        loaded_steps.as_slice()
                    } else {
                        embedded_steps
                    };
                    let summary = crate::engine::ai::run_with_flag(
                        steps,
                        repeat > 1,
                        repeat,
                        &detect,
                        &actuate,
                        &core.node_running,
                    );
                    if summary["ok"].as_bool().unwrap_or(false) {
                        Ok(())
                    } else {
                        Err(summary["error"]
                            .as_str()
                            .map(str::to_string)
                            .unwrap_or_else(|| format!("Macro '{name}' failed")))
                    }
                };
            let run_sub_macro =
                |name: &str, embedded_steps: &[Step], repeat: i64| -> Result<String, String> {
                    run_macro(name, embedded_steps, repeat)?;
                    let source = if embedded_steps.is_empty() {
                        "linked source".to_string()
                    } else {
                        format!(
                            "{} embedded action{}",
                            embedded_steps.len(),
                            if embedded_steps.len() == 1 { "" } else { "s" }
                        )
                    };
                    Ok(format!(
                        "Macro '{name}' finished ({source}, {repeat} run{})",
                        if repeat == 1 { "" } else { "s" }
                    ))
                };
            let run_chain = |chain_id: &str| -> Result<String, String> {
                let chain = chains
                    .iter()
                    .find(|chain| chain.id == chain_id)
                    .ok_or_else(|| format!("Chain '{chain_id}' no longer exists"))?;
                if chain.macro_names.is_empty() {
                    return Err(format!("Chain '{}' has no macros", chain.name));
                }
                let mut iteration = 0_i64;
                while core.node_running.load(Ordering::SeqCst) {
                    iteration += 1;
                    if chain.repeat > 0 && iteration > chain.repeat {
                        break;
                    }
                    for (index, macro_name) in chain.macro_names.iter().enumerate() {
                        if !core.node_running.load(Ordering::SeqCst) {
                            return Err("Stopped".to_string());
                        }
                        run_macro(macro_name, &[], 1).map_err(|error| {
                            format!("Chain '{}' failed at '{}': {error}", chain.name, macro_name)
                        })?;
                        if index + 1 < chain.macro_names.len() && chain.delay_between > 0.0 {
                            let deadline =
                                Instant::now() + Duration::from_secs_f64(chain.delay_between);
                            while Instant::now() < deadline {
                                if !core.node_running.load(Ordering::SeqCst) {
                                    return Err("Stopped".to_string());
                                }
                                let remaining = deadline.saturating_duration_since(Instant::now());
                                thread::sleep(remaining.min(Duration::from_millis(50)));
                            }
                        }
                    }
                }
                if !core.node_running.load(Ordering::SeqCst) {
                    Err("Stopped".to_string())
                } else {
                    Ok(format!(
                        "Chain '{}' finished ({} iteration{})",
                        chain.name,
                        iteration,
                        if iteration == 1 { "" } else { "s" }
                    ))
                }
            };

            core.detections.set("nodes", true);
            let summary = crate::engine::node_graph::run(
                &graph,
                &detect,
                &actuate,
                &run_sub_macro,
                &run_chain,
                &core.node_running,
            );
            core.detections.set("nodes", false);
            core.node_running.store(false, Ordering::SeqCst);
            core.set_mode("idle");

            let ok = summary["ok"].as_bool().unwrap_or(false);
            let run = summary["nodes_run"].as_u64().unwrap_or(0);
            core.emit(
                if ok { "ok" } else { "warn" },
                if ok {
                    format!("Node graph finished ({run} nodes)")
                } else {
                    format!(
                        "Node graph stopped: {}",
                        summary["error"].as_str().unwrap_or("path failed")
                    )
                },
            );
        });

        json!({ "ok": true })
    }
}

/// Apply one guard [`Action`] to the shared player/controller. Each arm is a
/// single atomic hardware call, matching `GuardEngine._execute_action`.
fn execute_action(player: &MacroPlayer, controller: &InputController, action: Action) {
    match action {
        Action::Pause => player.pause(),
        Action::Resume => player.resume(),
        Action::KeyPress(key) => controller.key_press(&key),
        Action::Click(x, y) => controller.click(x as i32, y as i32, "left"),
        Action::BezierMoveTo(x, y) => controller.bezier_move_to(x as i32, y as i32, 0.0),
        Action::MoveTo(x, y) => controller.move_to(x as i32, y as i32),
        Action::MouseDown(button) => controller.mouse_down(None, &button),
        Action::MouseUp(button) => controller.mouse_up(None, &button),
    }
}

/// Apply one AI-step [`Action`](crate::engine::ai::Action) to the input
/// controller, the executor's actuate callback, mirroring `AIExecutor`'s direct
/// `controller.*` calls. `Sleep` clamps negative/NaN to zero so
/// `Duration::from_secs_f64` can't panic in the run thread (which would leak the
/// mode); Python's `time.sleep` raised on a negative delay instead.
fn execute_ai_action(
    reliable_input: &ReliableInput,
    controller: &InputController,
    target: &Mutex<Option<ReliableTarget>>,
    action: crate::engine::ai::Action,
) -> Result<(), String> {
    use crate::engine::ai::Action;
    match action {
        Action::FocusAt(x, y) => {
            let established =
                reliable_input.establish_at(screen_coord(x, "x")?, screen_coord(y, "y")?)?;
            *target
                .lock()
                .map_err(|_| "Loop target context is poisoned".to_string())? = Some(established);
            Ok(())
        }
        Action::Click(x, y) => {
            let previous = target
                .lock()
                .map_err(|_| "Loop target context is poisoned".to_string())?
                .clone();
            let established = reliable_input.click_at_with_prior(
                screen_coord(x, "x")?,
                screen_coord(y, "y")?,
                previous.as_ref(),
            )?;
            *target
                .lock()
                .map_err(|_| "Loop target context is poisoned".to_string())? = Some(established);
            Ok(())
        }
        Action::KeyPress(key) => {
            let established = target
                .lock()
                .map_err(|_| "Loop target context is poisoned".to_string())?
                .clone()
                .ok_or_else(|| {
                    "no Vision target has been established for this Loop run".to_string()
                })?;
            reliable_input.key_on(&established, &key)
        }
        Action::TypeText(text) => {
            let established = target
                .lock()
                .map_err(|_| "Loop target context is poisoned".to_string())?
                .clone()
                .ok_or_else(|| {
                    "no Vision target has been established for this Loop run".to_string()
                })?;
            for character in text.chars() {
                reliable_input.key_on(&established, &character.to_string())?;
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(())
        }
        Action::Scroll(amount, pos) => {
            let pos = match pos {
                Some((x, y)) => {
                    let point = (screen_coord(x, "x")?, screen_coord(y, "y")?);
                    let established = reliable_input.establish_at(point.0, point.1)?;
                    *target
                        .lock()
                        .map_err(|_| "Loop target context is poisoned".to_string())? =
                        Some(established);
                    Some(point)
                }
                None => None,
            };
            controller
                .try_scroll(
                    amount
                        .try_into()
                        .map_err(|_| format!("scroll amount {amount} is outside the i32 range"))?,
                    pos,
                )
                .map_err(|error| error.to_string())
        }
        Action::Sleep(secs) => {
            std::thread::sleep(std::time::Duration::from_secs_f64(secs.max(0.0)));
            Ok(())
        }
    }
}

fn screen_coord(value: i64, axis: &str) -> Result<i32, String> {
    value
        .try_into()
        .map_err(|_| format!("{axis} coordinate {value} is outside the Windows screen range"))
}

fn has_fail_closed_checkpoint(macro_def: &Macro) -> bool {
    macro_def.events.iter().any(|input| {
        input.event_type == InputEventType::Checkpoint
            && input.checkpoint.as_ref().is_some_and(|cfg| {
                matches!(cfg, Value::Object(map) if !map.is_empty())
                    && cfg.get("on_timeout").and_then(Value::as_str) != Some("continue")
            })
    })
}

/// Resolve `play_macro`'s `repeat` argument. `None` reads the macro's saved loop
/// settings; a value coerces through Python's `int()` rules, with `""`/`∞`
/// meaning infinite (0).
fn resolve_repeat(repeat: Option<&Value>, m: &Macro) -> i64 {
    match repeat {
        None => {
            if m.loop_enabled && m.loop_count == 0 {
                0
            } else if !m.loop_enabled {
                1
            } else {
                m.loop_count
            }
        }
        Some(v) => {
            if let Some(s) = v.as_str() {
                if s.is_empty() || s == "\u{221e}" {
                    return 0;
                }
            }
            py_int(v).unwrap_or(1)
        }
    }
}

/// Python `int(value)` for the shapes `repeat` can arrive as: an int/float
/// (truncated toward zero), a base-10 string, or a bool. Anything else is `None`.
fn py_int(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Value::String(s) => s.trim().parse::<i64>().ok(),
        Value::Bool(b) => Some(*b as i64),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        has_fail_closed_checkpoint, is_full_region, is_text_guard, py_int, region_pixels,
        resolve_repeat, text_roi, Sighting, Vision, SIGHTING_TTL_MS,
    };
    use crate::hardware::capture::Frame;
    use crate::logbuf::LogBuffer;
    use crate::models::guard::Guard;
    use crate::models::macro_def::{InputEventType, Macro, MacroEvent};
    use serde_json::json;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    #[test]
    fn the_overlay_sees_every_live_source_and_no_dead_ones() {
        let vision = Vision::new(Arc::new(Mutex::new(LogBuffer::default())));
        let frame = Frame {
            bgr: Vec::new(),
            width: 1920,
            height: 1080,
        };
        let seen = |label: &str| Sighting {
            label: label.to_string(),
            x: 10,
            y: 20,
            w: 30,
            h: 40,
            confidence: 0.9,
        };

        vision.publish("guards", &frame, vec![seen("Health low")]);
        vision.publish("steps", &frame, vec![seen("Claim reward")]);

        let out = vision.sightings();
        assert_eq!(out["screen"], json!([1920, 1080]));
        let items = out["items"].as_array().unwrap();
        assert_eq!(items.len(), 2, "both sources draw at once");
        assert!(
            items.iter().all(|i| i["age_ms"].is_u64()),
            "each box fades on its own clock"
        );

        // A loop that has stopped leaves its last pass behind. It must stop
        // contributing rather than hang on screen for the rest of the session.
        vision
            .sightings
            .lock()
            .unwrap()
            .get_mut("guards")
            .unwrap()
            .at = Instant::now() - Duration::from_millis(SIGHTING_TTL_MS as u64 + 500);
        let out = vision.sightings();
        let items = out["items"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["label"], "Claim reward");

        // And a pass that found nothing takes its boxes off immediately.
        vision.publish("steps", &frame, Vec::new());
        let out = vision.sightings();
        assert!(out["items"].as_array().unwrap().is_empty());
        assert_eq!(
            out["screen"],
            json!([0, 0]),
            "nothing live, nothing to scale by"
        );
    }

    #[test]
    fn region_pixels_matches_python_to_pixels() {
        // Corners, not width/height: `frame[y1:y2, x1:x2]`.
        assert_eq!(
            region_pixels([0.0, 0.0, 100.0, 100.0], 2560, 1440),
            (0, 0, 2560, 1440)
        );
        assert_eq!(
            region_pixels([25.0, 50.0, 75.0, 100.0], 2560, 1440),
            (640, 720, 1920, 1440)
        );
        // int() truncates: 33.3% of 100 is 33.3 → 33, never 34.
        assert_eq!(
            region_pixels([33.3, 33.3, 66.7, 66.7], 100, 100),
            (33, 33, 66, 66)
        );
    }

    #[test]
    fn full_region_uses_the_python_tolerances() {
        assert!(is_full_region([0.0, 0.0, 100.0, 100.0]));
        assert!(is_full_region([0.5, 0.5, 99.5, 99.5])); // exactly on the bound
        assert!(!is_full_region([0.6, 0.0, 100.0, 100.0]));
        assert!(!is_full_region([0.0, 0.0, 99.4, 100.0]));
    }

    #[test]
    fn text_guards_include_the_legacy_spelling() {
        let g = |method: &str| Guard {
            method: method.to_string(),
            ..Default::default()
        };
        assert!(is_text_guard(&g("ocr")));
        assert!(is_text_guard(&g("text")));
        assert!(!is_text_guard(&g("color")));
        assert!(!is_text_guard(&g("template")));
    }

    #[test]
    fn text_roi_pads_the_region_and_reports_its_origin() {
        let frame = Frame {
            bgr: vec![0u8; 200 * 200 * 3],
            width: 200,
            height: 200,
        };
        // 60px box: a fifth of it either side, so 20px wider each way.
        let (roi, origin) = text_roi(&frame, 80, 80, 140, 140).unwrap();
        assert_eq!(origin, (60, 60));
        assert_eq!((roi.width, roi.height), (100, 100));
    }

    #[test]
    fn text_roi_padding_stops_at_the_frame_edge() {
        let frame = Frame {
            bgr: vec![0u8; 200 * 200 * 3],
            width: 200,
            height: 200,
        };
        // Flush against the top-left: the pad has nowhere to go on those sides,
        // and the origin must still be where the crop actually starts.
        let (roi, origin) = text_roi(&frame, 0, 0, 30, 30).unwrap();
        assert_eq!(origin, (0, 0));
        assert_eq!((roi.width, roi.height), (40, 40));
        // A wide, one-line region gets the floor, not a third of its long side.
        let (roi, origin) = text_roi(&frame, 100, 100, 190, 112).unwrap();
        assert_eq!(origin, (94, 94));
        assert_eq!((roi.width, roi.height), (102, 24));
    }

    #[test]
    fn py_int_matches_python_int() {
        // int() truncates toward zero and strips surrounding whitespace.
        assert_eq!(py_int(&json!(3)), Some(3));
        assert_eq!(py_int(&json!(2.9)), Some(2));
        assert_eq!(py_int(&json!(-2.7)), Some(-2));
        assert_eq!(py_int(&json!("5")), Some(5));
        assert_eq!(py_int(&json!("  7 ")), Some(7));
        assert_eq!(py_int(&json!(true)), Some(1));
        assert_eq!(py_int(&json!("abc")), None);
        assert_eq!(py_int(&json!(null)), None);
    }

    fn macro_with(loop_enabled: bool, loop_count: i64) -> Macro {
        Macro {
            loop_enabled,
            loop_count,
            ..Default::default()
        }
    }

    #[test]
    fn resolve_repeat_none_reads_saved_loop_settings() {
        assert_eq!(resolve_repeat(None, &macro_with(true, 0)), 0); // loop enabled, count 0 → forever
        assert_eq!(resolve_repeat(None, &macro_with(false, 1)), 1); // no loop → once
        assert_eq!(resolve_repeat(None, &macro_with(true, 5)), 5); // fixed count
    }

    #[test]
    fn resolve_repeat_value_coerces_like_python() {
        let m = macro_with(false, 1);
        assert_eq!(resolve_repeat(Some(&json!("")), &m), 0); // empty string → infinite
        assert_eq!(resolve_repeat(Some(&json!("\u{221e}")), &m), 0); // ∞ → infinite
        assert_eq!(resolve_repeat(Some(&json!("3")), &m), 3);
        assert_eq!(resolve_repeat(Some(&json!(3)), &m), 3);
        assert_eq!(resolve_repeat(Some(&json!(2.9)), &m), 2); // int() truncates
        assert_eq!(resolve_repeat(Some(&json!("abc")), &m), 1); // int() raises → fallback 1
    }

    #[test]
    fn only_a_fail_closed_checkpoint_can_verify_a_repeating_run() {
        let checkpoint = |policy: Option<&str>, populated: bool| MacroEvent {
            event_type: InputEventType::Checkpoint,
            timestamp: 0.0,
            x: 0,
            y: 0,
            mouse_motion: None,
            dx: 0,
            dy: 0,
            button: "left".to_string(),
            key: String::new(),
            delta: 0,
            duration: 0.0,
            checkpoint: Some(match policy {
                Some(policy) => json!({ "on_timeout": policy }),
                None if populated => json!({ "mode": "wait_for" }),
                None => json!({}),
            }),
        };
        let safe = Macro {
            events: vec![checkpoint(Some("stop"), true)],
            ..Default::default()
        };
        assert!(has_fail_closed_checkpoint(&safe));
        let migrated_default = Macro {
            events: vec![checkpoint(None, true)],
            ..Default::default()
        };
        assert!(has_fail_closed_checkpoint(&migrated_default));
        let unsafe_continue = Macro {
            events: vec![checkpoint(Some("continue"), true)],
            ..Default::default()
        };
        assert!(!has_fail_closed_checkpoint(&unsafe_continue));
        let empty = Macro {
            events: vec![checkpoint(None, false)],
            ..Default::default()
        };
        assert!(!has_fail_closed_checkpoint(&empty));
    }
}
