//! Vision sidecar client — spawns and drives the Python `clawmation_vision`
//! process over newline-delimited JSON-RPC on its stdin/stdout.
//!
//! The one piece of the app that stays Python is the computer-vision compute
//! (OpenCV template/colour matching, easyocr) — kept verbatim so detection
//! behaviour is *identical*, not merely equivalent. This client is the Rust end
//! of that seam: the engines call typed methods here, and each one is a single
//! request/response round-trip to the sidecar.
//!
//! Concurrency mirrors the process it drives. The sidecar answers one request at
//! a time against a single shared detector (Python's GuardEngine, VisionAgent,
//! and guard-test all shared one detector too), so a request write and its
//! response read are guarded together by one `Mutex<Io>`: concurrent callers
//! serialize into clean round-trips rather than interleaving on the pipe. No id
//! correlation map or reader thread is needed — the lock *is* the correlation.
//!
//! The child handle lives behind its own mutex, separate from the io lock. That
//! split is deliberate: `Drop` (and, in a later slice, a liveness/restart check)
//! can take the child and `kill()` it to unwedge a `read_line` that is blocked on
//! a hung sidecar, without contending for the io lock the blocked caller holds.
//!
//! The blocking read has no timeout, and that is a conscious choice, not an
//! oversight: Python's in-process detection also blocked with no deadline; a
//! *dead* child yields EOF → [`VisionError::Closed`] (the caller respawns); and a
//! *hung* child's blast radius is confined to the vision subsystem (the webview
//! and status heartbeat never take the io lock — provided `capture_fps` is cached
//! off the hot path at the status layer, not polled through here). A timeout
//! would be speculative robustness for a failure the `Closed` path already covers.
//!
//! Intentionally not exposed yet (each lands with its first caller, as the
//! capture slice deferred `grab_rgb`): typed wrappers for `load_template`,
//! `detect_color`, `color_present`, and `ocr_find`. The guard path is
//! self-contained — `detect_guard` loads its own template inside the sidecar — so
//! the orchestration and guard-test slices need only the methods below.

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::models::guard::Guard;

/// One detection returned by the sidecar — the wire form of `detection.py`'s
/// `Detection`, matching `server.py::_det_dict` field-for-field. `x`/`y` are the
/// match *centre* (not the top-left); `roi_offset` is the region origin the
/// coordinates are already absolute to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Detection {
    pub label: String,
    pub x: i64,
    pub y: i64,
    pub w: i64,
    pub h: i64,
    pub confidence: f64,
    pub roi_offset: [i64; 2],
}

/// Why a sidecar round-trip failed. The variants are distinct because the
/// orchestration layer acts on them differently: `Remote` is a per-guard
/// detection error to log and carry past, whereas `Closed` means the process is
/// gone and must be respawned before the next call.
#[derive(Debug)]
pub enum VisionError {
    /// The pipe to the sidecar failed on write or read.
    Io(io::Error),
    /// The sidecar closed its stdout (EOF): the process is gone — respawn it.
    Closed,
    /// The sidecar answered `ok:false` — a detection/handler error, recoverable.
    Remote(String),
    /// The sidecar sent something off-contract (undecodable line, id mismatch).
    Protocol(String),
}

impl std::fmt::Display for VisionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "vision sidecar I/O error: {e}"),
            Self::Closed => write!(f, "vision sidecar closed the connection"),
            Self::Remote(m) => write!(f, "vision sidecar error: {m}"),
            Self::Protocol(m) => write!(f, "vision sidecar protocol error: {m}"),
        }
    }
}

impl std::error::Error for VisionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

/// The response envelope. `result` is present on success, `error` on failure;
/// both default so one struct decodes either shape.
#[derive(Deserialize)]
struct Response {
    #[serde(default)]
    id: Option<u64>,
    ok: bool,
    #[serde(default)]
    result: Value,
    #[serde(default)]
    error: String,
}

/// Writer, reader, and the request counter locked as a unit so a request and its
/// response can never interleave with another caller's on the shared pipe.
struct Io {
    writer: Box<dyn Write + Send>,
    reader: Box<dyn BufRead + Send>,
    next_id: u64,
}

/// A running vision sidecar and the stdio channel to it.
pub struct VisionClient {
    io: Mutex<Io>,
    /// The child process, kept for `kill`/`wait`. Its own mutex so teardown can
    /// reach it without the io lock (see the module note on the two-mutex split).
    child: Mutex<Option<Child>>,
}

impl VisionClient {
    /// Spawn the sidecar. `program`/`args` are the interpreter+module in dev
    /// (`python -m clawmation_vision.server`) or the frozen executable in a
    /// bundled build; `cwd` is where the `clawmation_vision` package resolves
    /// from. stderr is inherited so the sidecar's logs reach our own stderr and
    /// can never back-pressure into the protocol stream.
    pub fn spawn(program: &str, args: &[&str], cwd: Option<&Path>) -> io::Result<Self> {
        let mut cmd = Command::new(program);
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }
        // A bundled release build is a GUI (windows) subsystem process with no
        // console, so launching the console-subsystem sidecar would pop a console
        // window on every spawn. `CREATE_NO_WINDOW` suppresses it; stderr is still
        // inherited by handle, so the sidecar's logs reach ours regardless.
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = cmd.spawn()?;
        let writer = child.stdin.take().expect("stdin was piped");
        let reader = child.stdout.take().expect("stdout was piped");
        Ok(Self::from_io(Box::new(writer), Box::new(BufReader::new(reader)), Some(child)))
    }

    /// Construct over arbitrary streams — the seam the protocol tests drive with
    /// in-memory pipes, no Python required.
    fn from_io(
        writer: Box<dyn Write + Send>,
        reader: Box<dyn BufRead + Send>,
        child: Option<Child>,
    ) -> Self {
        Self { io: Mutex::new(Io { writer, reader, next_id: 1 }), child: Mutex::new(child) }
    }

    /// Write one request line, read one response line, correlate the id. Holds
    /// the io lock across the whole round-trip so it is atomic against other
    /// callers.
    fn call(&self, method: &str, params: Value) -> Result<Value, VisionError> {
        let mut guard = self.io.lock().unwrap();
        let io = &mut *guard;

        let id = io.next_id;
        io.next_id += 1;

        let mut line = serde_json::to_string(&json!({ "id": id, "method": method, "params": params }))
            .expect("a request object always serializes");
        line.push('\n');
        io.writer.write_all(line.as_bytes()).map_err(VisionError::Io)?;
        io.writer.flush().map_err(VisionError::Io)?;

        let mut raw = String::new();
        if io.reader.read_line(&mut raw).map_err(VisionError::Io)? == 0 {
            return Err(VisionError::Closed);
        }
        let resp: Response = serde_json::from_str(raw.trim())
            .map_err(|e| VisionError::Protocol(format!("undecodable response {:?}: {e}", raw.trim())))?;
        if resp.id != Some(id) {
            return Err(VisionError::Protocol(format!(
                "response id {:?} does not match request id {id}",
                resp.id
            )));
        }
        if resp.ok {
            Ok(resp.result)
        } else {
            Err(VisionError::Remote(resp.error))
        }
    }

    fn call_typed<T: for<'de> Deserialize<'de>>(
        &self,
        method: &str,
        params: Value,
    ) -> Result<T, VisionError> {
        let value = self.call(method, params)?;
        serde_json::from_value(value)
            .map_err(|e| VisionError::Protocol(format!("bad {method} result: {e}")))
    }

    /// Liveness probe. `ready` is false until [`init`](Self::init) has built the
    /// detector and capture.
    pub fn ping(&self) -> Result<bool, VisionError> {
        let r = self.call("ping", json!({}))?;
        Ok(r.get("ready").and_then(Value::as_bool).unwrap_or(false))
    }

    /// Build the sidecar's detector and capture. `backend` is the *requested*
    /// capture backend name (`"dxcam"` | `"mss"`); the sidecar may fall back (e.g.
    /// dxcam→mss when dxcam is unavailable), so this returns the *resolved*
    /// backend it actually opened — what `get_status` reports and Python logged as
    /// `Capture ready (…)`.
    pub fn init(&self, screen_w: i64, screen_h: i64, backend: &str) -> Result<String, VisionError> {
        let r = self.call(
            "init",
            json!({ "screen_w": screen_w, "screen_h": screen_h, "capture_backend": backend }),
        )?;
        Ok(r.get("backend").and_then(Value::as_str).unwrap_or(backend).to_string())
    }

    /// Detect one guard against a freshly grabbed frame.
    pub fn detect_guard(&self, guard: &Guard) -> Result<Vec<Detection>, VisionError> {
        self.call_typed("detect_guard", json!({ "guard": guard }))
    }

    /// Detect every guard against a *single* grabbed frame, keyed by guard id.
    /// This is the poll-loop entry point: one grab per cycle keeps every guard in
    /// a cycle looking at the same frame, exactly as Python's loop did.
    pub fn detect_guards(
        &self,
        guards: &[Guard],
    ) -> Result<HashMap<String, Vec<Detection>>, VisionError> {
        self.call_typed("detect_guards", json!({ "guards": guards }))
    }

    /// Detect a checkpoint's target against a freshly grabbed frame. `cfg` is the
    /// checkpoint dict itself (`method`/`region`/`hsv_low`/`hsv_high`/`template`/
    /// `threshold`/`min_area`); the sidecar reads those keys directly, collapses
    /// the region, and routes the method — the detection half of Python's
    /// `MacroPlayer._run_checkpoint`. The surrounding poll/timeout/action
    /// orchestration stays in the Rust player.
    pub fn detect_checkpoint(&self, cfg: &Value) -> Result<Vec<Detection>, VisionError> {
        self.call_typed("detect_checkpoint", cfg.clone())
    }

    /// Dry-run one guard against a fresh frame for the editor's Test button.
    /// Returns the sidecar's result dict verbatim —
    /// `{ok, matched, found_x, found_y, confidence, message, preview}` — passed
    /// straight through to the webview rather than typed here, because its shape
    /// is exactly what the desktop app's `guard_test` returned to the UI.
    pub fn guard_test(&self, guard: &Guard) -> Result<Value, VisionError> {
        self.call("guard_test", json!({ "guard": guard }))
    }

    /// Detect one AI step's target against a freshly grabbed frame. `step` is the
    /// step dict itself (`detect_mode`/`region`/`hsv_low`/`hsv_high`/`template`/
    /// `confidence`/`min_area`); the sidecar reads those keys and returns
    /// `{matches:[{x,y,confidence}], message}` — the detection half of the AI
    /// executor's `find_click`/`wait_for`. The run loop stays in the Rust engine.
    pub fn ai_detect(&self, step: &Value) -> Result<Value, VisionError> {
        self.call("ai_detect", json!({ "step": step }))
    }

    /// Dry-run one AI step against a fresh frame for the step editor's Test button.
    /// Returns the sidecar's result dict verbatim —
    /// `{ok, message, found_x, found_y, matched, confidence, preview}` — passed
    /// straight through to the webview, the shape `Api.ai_test_step` returned.
    pub fn ai_test_step(&self, step: &Value) -> Result<Value, VisionError> {
        self.call("ai_test_step", json!({ "step": step }))
    }

    /// Open the colour-sampler overlay for the guard editor. Blocks in the sidecar
    /// until the user clicks a pixel (or presses Esc) — the io lock is held for
    /// that whole span, exactly as the desktop app blocked its UI thread. Returns
    /// the sidecar's dict verbatim (`{ok, x, y, hsv, hsv_low, hsv_high, hex}` on a
    /// pick, `{ok:false, error:"cancelled"}` on Esc), the shape
    /// `Api.guard_pick_color` handed the editor.
    pub fn guard_pick_color(&self) -> Result<Value, VisionError> {
        self.call("guard_pick_color", json!({}))
    }

    /// Open the region-drag overlay. Blocks until the user drags a watch rectangle
    /// (or cancels). Returns `{ok, x, y, w, h}` in screen pixels, or the
    /// cancellation dict — verbatim `Api.guard_pick_region`.
    pub fn guard_pick_region(&self) -> Result<Value, VisionError> {
        self.call("guard_pick_region", json!({}))
    }

    /// Drag a box around an on-screen button and save the crop as a template PNG
    /// under `templates_dir`. The directory is owned by the Rust side and passed by
    /// value so the sidecar keeps no path globals (its one departure from the
    /// verbatim `Api.capture_template`, which read `guards.TEMPLATES_DIR`). Returns
    /// `{ok, path, w, h, thumb}` — the saved path plus a base64 PNG thumbnail.
    pub fn capture_template(&self, templates_dir: &str) -> Result<Value, VisionError> {
        self.call("capture_template", json!({ "templates_dir": templates_dir }))
    }

    /// Pick an image file from disk (native file dialog) and copy it into
    /// `templates_dir` as a template. Same result shape as
    /// [`capture_template`](Self::capture_template); verbatim `Api.add_template_image`.
    pub fn add_template_image(&self, templates_dir: &str) -> Result<Value, VisionError> {
        self.call("add_template_image", json!({ "templates_dir": templates_dir }))
    }

    /// Two-phase surgical capture: drag a tight box around an element, then draw
    /// the exact pixels to hit. Saves the crop into `templates_dir` and returns
    /// `{ok, path, click_lines, click_line, offset, w, h, thumb}` — verbatim
    /// `Api.surgical_capture`, minus its toasts (the editor owns those).
    pub fn surgical_capture(&self, templates_dir: &str) -> Result<Value, VisionError> {
        self.call("surgical_capture", json!({ "templates_dir": templates_dir }))
    }

    /// The capture's rolling-average FPS (cosmetic UI telemetry). Callers on a
    /// heartbeat should cache this rather than poll it through the io lock.
    pub fn capture_fps(&self) -> Result<f64, VisionError> {
        let r = self.call("capture_fps", json!({}))?;
        Ok(r.get("fps").and_then(Value::as_f64).unwrap_or(0.0))
    }

    /// Ask the sidecar to end its serve loop and exit. After this the process
    /// closes on its own; [`Drop`] still reaps it.
    pub fn shutdown(&self) -> Result<(), VisionError> {
        self.call("shutdown", json!({}))?;
        Ok(())
    }
}

impl Drop for VisionClient {
    /// Kill and reap the child so no orphaned Python lingers. Dropping the io
    /// streams first would already close the sidecar's stdin (EOF → clean exit);
    /// the explicit kill also unwedges a process hung mid-detection.
    fn drop(&mut self) {
        if let Ok(slot) = self.child.get_mut() {
            if let Some(child) = slot {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::sync::{Arc, Mutex as StdMutex};

    /// A `Write` that appends to a shared buffer the test can inspect afterward —
    /// stands in for the sidecar's stdin so we can assert what was sent.
    #[derive(Clone)]
    struct SharedWriter(Arc<StdMutex<Vec<u8>>>);

    impl Write for SharedWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// A client whose "sidecar" is a fixed script of response lines, plus a handle
    /// on everything it writes. next_id is deterministic (1, 2, …), so the canned
    /// responses can echo the ids the client will send.
    fn scripted(responses: &str) -> (VisionClient, Arc<StdMutex<Vec<u8>>>) {
        let sent = Arc::new(StdMutex::new(Vec::new()));
        let client = VisionClient::from_io(
            Box::new(SharedWriter(sent.clone())),
            Box::new(Cursor::new(responses.as_bytes().to_vec())),
            None,
        );
        (client, sent)
    }

    fn sent_lines(sent: &Arc<StdMutex<Vec<u8>>>) -> Vec<Value> {
        String::from_utf8(sent.lock().unwrap().clone())
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    #[test]
    fn frames_requests_and_parses_results_in_order() {
        let (client, sent) = scripted(
            "{\"id\":1,\"ok\":true,\"result\":{\"pong\":true,\"ready\":false}}\n\
             {\"id\":2,\"ok\":true,\"result\":{\"backend\":\"mss\",\"screen_w\":1400,\"screen_h\":900}}\n\
             {\"id\":3,\"ok\":true,\"result\":[{\"label\":\"Reconnect\",\"x\":448,\"y\":282,\"w\":96,\"h\":64,\"confidence\":0.97,\"roi_offset\":[0,0]}]}\n",
        );

        assert!(!client.ping().unwrap()); // ready:false before init
        client.init(1400, 900, "mss").unwrap();
        let dets = client.detect_guard(&Guard { id: "g".into(), ..Default::default() }).unwrap();
        assert_eq!(
            dets,
            vec![Detection {
                label: "Reconnect".into(),
                x: 448,
                y: 282,
                w: 96,
                h: 64,
                confidence: 0.97,
                roi_offset: [0, 0],
            }]
        );

        // The requests carried monotonic ids and the right methods/params.
        let reqs = sent_lines(&sent);
        assert_eq!(reqs[0]["id"], 1);
        assert_eq!(reqs[0]["method"], "ping");
        assert_eq!(reqs[1]["id"], 2);
        assert_eq!(reqs[1]["method"], "init");
        assert_eq!(reqs[1]["params"]["capture_backend"], "mss");
        assert_eq!(reqs[1]["params"]["screen_w"], 1400);
        assert_eq!(reqs[2]["id"], 3);
        assert_eq!(reqs[2]["method"], "detect_guard");
        assert_eq!(reqs[2]["params"]["guard"]["id"], "g");
    }

    #[test]
    fn detect_guards_decodes_the_keyed_map() {
        let (client, _sent) = scripted(
            "{\"id\":1,\"ok\":true,\"result\":{\"a\":[],\"b\":[{\"label\":\"x\",\"x\":1,\"y\":2,\"w\":3,\"h\":4,\"confidence\":0.5,\"roi_offset\":[10,20]}]}}\n",
        );
        let map = client
            .detect_guards(&[
                Guard { id: "a".into(), ..Default::default() },
                Guard { id: "b".into(), ..Default::default() },
            ])
            .unwrap();
        assert!(map["a"].is_empty());
        assert_eq!(map["b"][0].roi_offset, [10, 20]);
        assert_eq!(map["b"][0].confidence, 0.5);
    }

    #[test]
    fn guard_test_passes_the_result_dict_through() {
        // guard_test's payload is the editor's Test result, not a typed Detection
        // list — it must arrive at the caller byte-for-byte (preview base64 and all).
        let (client, sent) = scripted(
            "{\"id\":1,\"ok\":true,\"result\":{\"ok\":true,\"matched\":2,\"found_x\":540,\"found_y\":330,\"confidence\":0.821,\"message\":\"2 match(es) \u{00b7} conf 0.82\",\"preview\":\"/9j/AAAA\"}}\n",
        );
        let res = client.guard_test(&Guard { id: "g".into(), ..Default::default() }).unwrap();
        assert_eq!(res["ok"], true);
        assert_eq!(res["matched"], 2);
        assert_eq!(res["found_x"], 540);
        assert_eq!(res["found_y"], 330);
        assert_eq!(res["message"], "2 match(es) \u{00b7} conf 0.82");
        assert_eq!(res["preview"], "/9j/AAAA");

        let reqs = sent_lines(&sent);
        assert_eq!(reqs[0]["method"], "guard_test");
        assert_eq!(reqs[0]["params"]["guard"]["id"], "g");
    }

    #[test]
    fn pickers_forward_params_and_pass_results_through() {
        // Each editor picker returns the sidecar's dict verbatim (never typed),
        // and the two template pickers must carry templates_dir in their params.
        let (client, sent) = scripted(
            "{\"id\":1,\"ok\":true,\"result\":{\"ok\":true,\"hex\":\"#c8461e\",\"hsv_low\":[5,120,120],\"hsv_high\":[15,255,255]}}\n\
             {\"id\":2,\"ok\":true,\"result\":{\"ok\":true,\"x\":10,\"y\":20,\"w\":30,\"h\":40}}\n\
             {\"id\":3,\"ok\":true,\"result\":{\"ok\":true,\"path\":\"C:/t/template_1_60x70.png\",\"w\":60,\"h\":70,\"thumb\":\"iVBOR\"}}\n\
             {\"id\":4,\"ok\":true,\"result\":{\"ok\":false,\"error\":\"cancelled\"}}\n",
        );

        assert_eq!(client.guard_pick_color().unwrap()["hex"], "#c8461e");
        assert_eq!(client.guard_pick_region().unwrap()["w"], 30);
        assert_eq!(client.capture_template("C:/t").unwrap()["path"], "C:/t/template_1_60x70.png");
        let added = client.add_template_image("C:/t").unwrap();
        assert_eq!(added["ok"], false);
        assert_eq!(added["error"], "cancelled");

        let reqs = sent_lines(&sent);
        assert_eq!(reqs[0]["method"], "guard_pick_color");
        assert_eq!(reqs[1]["method"], "guard_pick_region");
        assert_eq!(reqs[2]["method"], "capture_template");
        assert_eq!(reqs[2]["params"]["templates_dir"], "C:/t");
        assert_eq!(reqs[3]["method"], "add_template_image");
        assert_eq!(reqs[3]["params"]["templates_dir"], "C:/t");
    }

    #[test]
    fn ok_false_becomes_a_remote_error() {
        let (client, _sent) =
            scripted("{\"id\":1,\"ok\":false,\"error\":\"RuntimeError: capture returned no frame\"}\n");
        match client.capture_fps() {
            Err(VisionError::Remote(m)) => assert!(m.contains("capture returned no frame")),
            other => panic!("expected Remote error, got {other:?}"),
        }
    }

    #[test]
    fn wrong_id_is_a_protocol_error() {
        // A well-formed response for the wrong request id must not be accepted as
        // this call's answer — that would desync every later call.
        let (client, _sent) = scripted("{\"id\":99,\"ok\":true,\"result\":{\"pong\":true,\"ready\":true}}\n");
        assert!(matches!(client.ping(), Err(VisionError::Protocol(_))));
    }

    #[test]
    fn undecodable_line_is_a_protocol_error_not_a_silent_skip() {
        // The easyocr banner ("Using CPU. …") on stdout would land here; it must
        // surface, never be skipped past to hunt for the "real" line.
        let (client, _sent) = scripted("Using CPU. Note: this module is much faster with a GPU.\n");
        assert!(matches!(client.ping(), Err(VisionError::Protocol(_))));
    }

    #[test]
    fn eof_before_a_response_is_closed() {
        let (client, _sent) = scripted(""); // sidecar died: stdout at EOF
        assert!(matches!(client.ping(), Err(VisionError::Closed)));
    }

    /// Drives the real Python sidecar over a spawned subprocess. Ignored by
    /// default (needs the repo `.venv` with opencv/mss and a live screen for the
    /// grab); run explicitly with
    /// `cargo test -- --ignored real_sidecar_round_trip`.
    #[test]
    #[ignore = "spawns the Python sidecar and grabs the real screen"]
    fn real_sidecar_round_trip() {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        let repo_root = manifest.parent().unwrap().parent().unwrap();
        let python = repo_root.join(".venv").join("Scripts").join("python.exe");
        let sidecar = repo_root.join("clawmation").join("sidecar");

        let client =
            VisionClient::spawn(python.to_str().unwrap(), &["-m", "clawmation_vision.server"], Some(&sidecar))
                .expect("spawn sidecar");

        assert!(!client.ping().expect("ping before init"));
        client.init(1400, 900, "mss").expect("init");
        assert!(client.ping().expect("ping after init"), "ready after init");
        assert!(client.capture_fps().expect("capture_fps") >= 0.0);

        // A guard that runs the colour path against a real grabbed frame; the
        // key must come back whether or not the (impossible) colour is present.
        let guard = Guard {
            id: "probe".into(),
            method: "color".into(),
            hsv_low: [0, 250, 250],
            hsv_high: [0, 255, 255],
            region: [0.0, 0.0, 100.0, 100.0],
            min_area: 40,
            ..Default::default()
        };
        let map = client.detect_guards(std::slice::from_ref(&guard)).expect("detect_guards");
        assert!(map.contains_key("probe"), "grab + detect path returned the guard key");

        client.shutdown().expect("shutdown");
    }

    /// The same round-trip as [`real_sidecar_round_trip`], but against the
    /// *frozen* `clawmation_vision.exe` a bundled build ships — spawned exactly as
    /// `core::spawn_sidecar` does in release (no args, cwd = the exe's own dir).
    /// Proves the PyInstaller freeze carries a working opencv/mss stack, not just
    /// an importable one. Skipped when the exe has not been built; run with
    /// `cargo test -- --ignored frozen_sidecar_round_trip`.
    #[test]
    #[ignore = "spawns the frozen sidecar exe and grabs the real screen"]
    fn frozen_sidecar_round_trip() {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        let exe = manifest
            .join("binaries")
            .join("clawmation_vision-x86_64-pc-windows-msvc.exe");
        if !exe.exists() {
            eprintln!("skipping: {} not built (freeze the sidecar first)", exe.display());
            return;
        }
        let dir = exe.parent().unwrap();

        let client =
            VisionClient::spawn(exe.to_str().unwrap(), &[], Some(dir)).expect("spawn frozen sidecar");

        assert!(!client.ping().expect("ping before init"));
        client.init(1400, 900, "mss").expect("init");
        assert!(client.ping().expect("ping after init"), "ready after init");
        assert!(client.capture_fps().expect("capture_fps") >= 0.0);

        let guard = Guard {
            id: "probe".into(),
            method: "color".into(),
            hsv_low: [0, 250, 250],
            hsv_high: [0, 255, 255],
            region: [0.0, 0.0, 100.0, 100.0],
            min_area: 40,
            ..Default::default()
        };
        let map = client.detect_guards(std::slice::from_ref(&guard)).expect("detect_guards");
        assert!(map.contains_key("probe"), "grab + detect path returned the guard key");

        client.shutdown().expect("shutdown");
    }
}
