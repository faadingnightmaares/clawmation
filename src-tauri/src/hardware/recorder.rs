//! Global input recording: raw Win32 low-level hooks capturing mouse/keyboard.
//!
//! Faithful port of `anime_macro/recorder.py::MacroRecorder`. The Python app
//! records through pynput, which installs `WH_MOUSE_LL` / `WH_KEYBOARD_LL` hooks
//! under the hood; we install the same two low-level hooks directly via
//! `windows-sys` rather than pull in a heavier wrapper crate. That keeps the
//! recorded event shape and key naming under our exact control: every virtual
//! key we record resolves back through [`input::resolve_vk`](crate::hardware::input)
//! (verified in tests), so a recording round-trips to identical playback.
//!
//! Two structural facts of low-level hooks shape this module:
//!   * the hook procedure is a bare C callback with no user-data parameter, so
//!     the recording state lives in a process-global `Mutex` the procs lock.
//!     That is honest rather than ugly: a system-wide hook is inherently a
//!     singleton, exactly like pynput's module-level listeners.
//!   * a low-level hook only fires while its installing thread pumps messages,
//!     so recording runs on a dedicated thread with a `GetMessage` loop, stopped
//!     by posting `WM_QUIT` to it.
//!
//! The hooks are non-suppressing (every event is passed on via `CallNextHookEx`),
//! so the user's real clicks and keys still reach the game while recording,
//! matching pynput's observe-only listeners.

use std::collections::HashSet;
use std::sync::mpsc;
use std::sync::Mutex;
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, PostThreadMessageW, SetWindowsHookExW,
    TranslateMessage, UnhookWindowsHookEx, HC_ACTION, KBDLLHOOKSTRUCT, MSG, MSLLHOOKSTRUCT,
    WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_QUIT,
    WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSKEYDOWN, WM_SYSKEYUP, WM_XBUTTONDOWN, WM_XBUTTONUP,
};

use crate::models::macro_def::{InputEventType, Macro, MacroEvent};

/// One wheel notch, as Win32 defines it (matches `input.rs`).
const WHEEL_DELTA: i64 = 120;

// ── Pure helpers (unit-tested without hardware) ──────────────────────────────

/// Round an event timestamp to 4 decimals, mirroring Python's
/// `InputEvent.to_dict` (`round(timestamp, 4)`). Every macro on disk was written
/// through that path, so freshly-recorded events must round the same way to land
/// at identical precision. Ties-to-even matches Python 3's `round`.
pub(crate) fn round4(ts: f64) -> f64 {
    (ts * 10_000.0).round_ties_even() / 10_000.0
}

/// Vertical wheel notches from a `WM_MOUSEWHEEL` `mouseData`, matching pynput:
/// the signed high word floor-divided by one notch. A high-resolution wheel that
/// reports a fraction of a notch therefore contributes 0 until a full notch
/// accumulates, bug-for-bug with pynput's integer `// WHEEL_DELTA`.
fn wheel_notches(mouse_data: u32) -> i64 {
    let raw = (mouse_data >> 16) as u16 as i16 as i64;
    raw.div_euclid(WHEEL_DELTA)
}

/// pynput's `Button.name` for an X-button event, from the `mouseData` high word.
fn xbutton_name(mouse_data: u32) -> Option<&'static str> {
    match (mouse_data >> 16) & 0xFFFF {
        1 => Some("x1"),
        2 => Some("x2"),
        _ => None,
    }
}

/// The recorded key string for a virtual-key code, the inverse of
/// [`input::resolve_vk`](crate::hardware::input) and the fidelity crux of this
/// slice. It reproduces what pynput's `_key_name` (`key.char or ""`, else
/// `key.name`) would have written for the same physical key, so recordings made
/// here are interchangeable with pynput-made ones.
///
/// * Special keys map to pynput's `Key.name` (`space`, `enter`, `shift_l`,
///   `f3`, …). The low-level hook delivers the *distinguished* modifier codes
///   (`VK_LSHIFT`/`VK_RSHIFT`/…), and we map the generic codes too, so whichever
///   the OS reports we produce pynput's name for it.
/// * Character keys map to the **unshifted** base character (`A`→`"a"`, digits,
///   US-layout OEM punctuation). That is replay-equivalent to pynput recording
///   the shifted char: `resolve_vk` lower-cases anyway, and Shift is captured as
///   its own event, so `Shift`+`"a"` reproduces the `"A"` pynput would store.
///   Skipping the layout-dependent `ToUnicode` lookup keeps the map pure.
/// * Anything unmapped returns `""`, exactly as pynput's `key.char or ""` does
///   when a key yields no character.
///
/// Best-effort, matching the `resolve_vk` asymmetry documented in
/// MIGRATION-NOTES: numpad keys map to their digit/operator character (a
/// pynput-equivalent character, not a strict inverse) and pynput names Python's
/// `_VK` can't resolve (`num_lock`, `scroll_lock`, `menu`, `f13` to `f20`) are
/// still recorded faithfully but replay as no-ops just as they would in Python.
/// None of these appear in this app's macros (0 of 604 real events use any key
/// beyond `a`/`b`).
fn key_name(vk: u16) -> String {
    // Function keys: pynput names f1..f20 (VK_F1..VK_F20 = 0x70..0x83).
    if (0x70..=0x83).contains(&vk) {
        return format!("f{}", vk - 0x70 + 1);
    }
    if let Some(name) = special_key_name(vk) {
        return name.to_string();
    }
    let ch = match vk {
        0x30..=0x39 => vk as u8 as char,                        // 0-9
        0x41..=0x5A => (vk as u8 as char).to_ascii_lowercase(), // A-Z → a-z
        0x60..=0x69 => (b'0' + (vk - 0x60) as u8) as char,      // numpad digits
        0x6A => '*',
        0x6B => '+',
        0x6D => '-',
        0x6E => '.',
        0x6F => '/',
        0xBA => ';',
        0xBB => '=',
        0xBC => ',',
        0xBD => '-',
        0xBE => '.',
        0xBF => '/',
        0xC0 => '`',
        0xDB => '[',
        0xDC => '\\',
        0xDD => ']',
        0xDE => '\'',
        _ => return String::new(),
    };
    ch.to_string()
}

/// pynput's `Key.name` for the special (non-character) virtual keys. The inverse
/// of `input.rs`'s `vk_lookup` for the entries pynput treats as special, plus
/// the few pynput names (`menu`, `num_lock`, `scroll_lock`) that Python's `_VK`
/// omits.
fn special_key_name(vk: u16) -> Option<&'static str> {
    Some(match vk {
        0x08 => "backspace",
        0x09 => "tab",
        0x0D => "enter",
        0x10 => "shift",
        0x11 => "ctrl",
        0x12 => "alt",
        0x13 => "pause",
        0x14 => "caps_lock",
        0x1B => "esc",
        0x20 => "space",
        0x21 => "page_up",
        0x22 => "page_down",
        0x23 => "end",
        0x24 => "home",
        0x25 => "left",
        0x26 => "up",
        0x27 => "right",
        0x28 => "down",
        0x2C => "print_screen",
        0x2D => "insert",
        0x2E => "delete",
        0x5B => "cmd",
        0x5C => "cmd_r",
        0x5D => "menu",
        0x90 => "num_lock",
        0x91 => "scroll_lock",
        0xA0 => "shift_l",
        0xA1 => "shift_r",
        0xA2 => "ctrl_l",
        0xA3 => "ctrl_r",
        0xA4 => "alt_l",
        0xA5 => "alt_r",
        _ => return None,
    })
}

/// A zeroed [`MacroEvent`] of the given kind and timestamp, with the same field
/// defaults `MacroEvent`'s serde config uses (`button = "left"`). Callers set the
/// fields that matter via struct-update syntax.
fn new_event(event_type: InputEventType, timestamp: f64) -> MacroEvent {
    MacroEvent {
        event_type,
        timestamp,
        x: 0,
        y: 0,
        button: "left".to_string(),
        key: String::new(),
        delta: 0,
        duration: 0.0,
        checkpoint: None,
    }
}

/// Idle shorter than this is the user's stop-press reaction time, not a
/// deliberate pause, so it is not worth preserving as a replay delay.
const TRAILING_WAIT_MIN: f64 = 0.05;

/// Capture the idle time after the last recorded action as a trailing WAIT
/// event stamped at the recording's total elapsed time. The player paces off
/// event timestamps and replays WAIT as a pure delay, so a looping macro now
/// waits this gap out before restarting instead of jumping straight into the
/// next iteration (issue #2): without it every loop was shorter than the
/// recording by exactly the time between the last action and the stop press.
fn append_trailing_wait(events: &mut Vec<MacroEvent>, total_elapsed: f64) {
    let total = round4(total_elapsed);
    if let Some(last) = events.last() {
        if total - last.timestamp > TRAILING_WAIT_MIN {
            events.push(new_event(InputEventType::Wait, total));
        }
    }
}

// ── Recording state (shared with the hook procedures) ─────────────────────────

/// The mutable recording state. Lives in the process-global [`struct@SHARED`]
/// because the low-level hook procedures are bare C callbacks that can't carry a
/// context pointer. Every event-building rule (the same-pixel move de-dup, the
/// paused-time exclusion, the ignore-key filter) lives here so it is unit-tested
/// on a local instance, hardware-free.
struct RecorderState {
    events: Vec<MacroEvent>,
    start: Instant,
    recording: bool,
    paused: bool,
    pause_start: Instant,
    paused_total: Duration,
    /// Hotkeys that drive the app itself, lower-cased; never baked into a macro.
    ignore_keys: HashSet<String>,
}

impl RecorderState {
    fn new(ignore_keys: HashSet<String>) -> Self {
        let now = Instant::now();
        Self {
            events: Vec::new(),
            start: now,
            recording: true,
            paused: false,
            pause_start: now,
            paused_total: Duration::ZERO,
            ignore_keys,
        }
    }

    /// Seconds since `start`, excluding time spent paused, the mirror of
    /// Python's `_elapsed` (`perf_counter() - start - paused_total`), so playback
    /// has no dead gaps where the user paused to think.
    fn elapsed(&self) -> f64 {
        self.start.elapsed().as_secs_f64() - self.paused_total.as_secs_f64()
    }

    fn active(&self) -> bool {
        self.recording && !self.paused
    }

    fn handle_move(&mut self, x: i64, y: i64) {
        if !self.active() {
            return;
        }
        // Skip only exact same-pixel duplicates (mouse jitter at rest). NEVER
        // mutate the previous event's timestamp; that would compress the gap to
        // the next real event and make playback fire early.
        if let Some(last) = self.events.last() {
            if last.event_type == InputEventType::MouseMove && last.x == x && last.y == y {
                return;
            }
        }
        let ts = self.elapsed();
        self.events.push(MacroEvent {
            x,
            y,
            ..new_event(InputEventType::MouseMove, ts)
        });
    }

    fn handle_button(&mut self, down: bool, x: i64, y: i64, button: &str) {
        if !self.active() {
            return;
        }
        let kind = if down {
            InputEventType::MouseDown
        } else {
            InputEventType::MouseUp
        };
        let ts = self.elapsed();
        self.events.push(MacroEvent {
            x,
            y,
            button: button.to_string(),
            ..new_event(kind, ts)
        });
    }

    fn handle_scroll(&mut self, x: i64, y: i64, delta: i64) {
        if !self.active() {
            return;
        }
        let ts = self.elapsed();
        self.events.push(MacroEvent {
            x,
            y,
            delta,
            ..new_event(InputEventType::Scroll, ts)
        });
    }

    fn handle_key(&mut self, down: bool, vk: u16) {
        if !self.active() {
            return;
        }
        let name = key_name(vk);
        if self.ignore_keys.contains(&name.to_ascii_lowercase()) {
            return;
        }
        let kind = if down {
            InputEventType::KeyDown
        } else {
            InputEventType::KeyUp
        };
        let ts = self.elapsed();
        self.events.push(MacroEvent {
            key: name,
            ..new_event(kind, ts)
        });
    }

    fn pause(&mut self) {
        if self.recording && !self.paused {
            self.paused = true;
            self.pause_start = Instant::now();
        }
    }

    fn resume(&mut self) {
        if self.recording && self.paused {
            self.paused_total += self.pause_start.elapsed();
            self.paused = false;
        }
    }
}

// ── Low-level hook procedures ─────────────────────────────────────────────────

/// The one live recording's state. `None` when not recording.
static SHARED: Mutex<Option<RecorderState>> = Mutex::new(None);

/// Lock the shared state, recovering from a poisoned mutex. A panic must never
/// unwind out of a C callback, so the hook procs (and everyone else) tolerate a
/// previous panic rather than re-panicking.
fn lock_shared() -> std::sync::MutexGuard<'static, Option<RecorderState>> {
    match SHARED.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Run `f` against the live recording state, if any. Called from the hook procs;
/// the per-event `active()` guard lives inside each `handle_*` method (mirroring
/// Python's `if not self._recording or self._paused: return`).
fn dispatch<F: FnOnce(&mut RecorderState)>(f: F) {
    if let Some(state) = lock_shared().as_mut() {
        f(state);
    }
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let data = &*(lparam as *const MSLLHOOKSTRUCT);
        let (x, y) = (data.pt.x as i64, data.pt.y as i64);
        let mouse_data = data.mouseData;
        match wparam as u32 {
            WM_MOUSEMOVE => dispatch(|s| s.handle_move(x, y)),
            WM_LBUTTONDOWN => dispatch(|s| s.handle_button(true, x, y, "left")),
            WM_LBUTTONUP => dispatch(|s| s.handle_button(false, x, y, "left")),
            WM_RBUTTONDOWN => dispatch(|s| s.handle_button(true, x, y, "right")),
            WM_RBUTTONUP => dispatch(|s| s.handle_button(false, x, y, "right")),
            WM_MBUTTONDOWN => dispatch(|s| s.handle_button(true, x, y, "middle")),
            WM_MBUTTONUP => dispatch(|s| s.handle_button(false, x, y, "middle")),
            WM_XBUTTONDOWN => {
                if let Some(b) = xbutton_name(mouse_data) {
                    dispatch(|s| s.handle_button(true, x, y, b));
                }
            }
            WM_XBUTTONUP => {
                if let Some(b) = xbutton_name(mouse_data) {
                    dispatch(|s| s.handle_button(false, x, y, b));
                }
            }
            // pynput reports vertical wheel as on_scroll dy and horizontal as
            // dx (dy = 0); the recorder stores dy, so a horizontal wheel is a
            // delta-0 scroll.
            WM_MOUSEWHEEL => dispatch(|s| s.handle_scroll(x, y, wheel_notches(mouse_data))),
            WM_MOUSEHWHEEL => dispatch(|s| s.handle_scroll(x, y, 0)),
            _ => {}
        }
    }
    CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let data = &*(lparam as *const KBDLLHOOKSTRUCT);
        let vk = data.vkCode as u16;
        match wparam as u32 {
            WM_KEYDOWN | WM_SYSKEYDOWN => dispatch(|s| s.handle_key(true, vk)),
            WM_KEYUP | WM_SYSKEYUP => dispatch(|s| s.handle_key(false, vk)),
            _ => {}
        }
    }
    CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
}

/// Install both low-level hooks and pump messages until `WM_QUIT`. Runs on its
/// own thread; reports that thread's native id back so [`MacroRecorder::stop`]
/// can wake it. The hooks are unhooked as the loop exits.
fn hook_thread(ready: mpsc::Sender<u32>) {
    // Record in physical pixels. A low-level mouse hook reports coordinates in
    // the DPI space of the thread that installed it, so make this thread
    // Per-Monitor-V2 aware BEFORE installing the hooks (see `hardware::dpi`).
    // With `screen_size` querying the stored resolution under the same context,
    // the recorded coordinates and `record_resolution` live in one physical
    // space, so replay on the recording display is scale-1.0 at any DPI. Held
    // for the whole thread — the hook callbacks run here too.
    let _aware = crate::hardware::dpi::PerMonitorAware::new();
    unsafe {
        let hmod = GetModuleHandleW(std::ptr::null());
        let mouse_hook = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), hmod, 0);
        let key_hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), hmod, 0);

        // Signal readiness only once the hooks are live, so start() cannot return
        // before recording is actually capturing.
        let _ = ready.send(GetCurrentThreadId());

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        UnhookWindowsHookEx(mouse_hook);
        UnhookWindowsHookEx(key_hook);
    }
}

/// Current wall-clock time as `(unix_seconds_int, unix_seconds_float)`, the two
/// forms Python takes from `time.time()`: `int(time.time())` for the macro name
/// and the raw float for `created_at`.
fn now_unix() -> (u64, f64) {
    let d = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    (d.as_secs(), d.as_secs_f64())
}

// ── Recorder ─────────────────────────────────────────────────────────────────

/// Records mouse and keyboard events via low-level Win32 hooks. A faithful port
/// of Python's `MacroRecorder`.
///
/// Because the hooks are system-wide, this is effectively a singleton: only one
/// recording can be live at a time (`start` replaces any prior state). The app
/// keeps a single instance, exactly as the Python app does.
pub struct MacroRecorder {
    resolution: (u32, u32),
    ignore_keys: HashSet<String>,
    thread: Option<JoinHandle<()>>,
    thread_id: u32,
}

impl MacroRecorder {
    pub fn new(resolution: (u32, u32), ignore_keys: HashSet<String>) -> Self {
        let ignore_keys = ignore_keys.into_iter().map(|k| k.to_lowercase()).collect();
        Self {
            resolution,
            ignore_keys,
            thread: None,
            thread_id: 0,
        }
    }

    pub fn is_recording(&self) -> bool {
        lock_shared().as_ref().is_some_and(|s| s.recording)
    }

    pub fn is_paused(&self) -> bool {
        lock_shared().as_ref().is_some_and(|s| s.paused)
    }

    pub fn event_count(&self) -> usize {
        lock_shared().as_ref().map_or(0, |s| s.events.len())
    }

    pub fn pause(&self) {
        if let Some(s) = lock_shared().as_mut() {
            s.pause();
        }
    }

    pub fn resume(&self) {
        if let Some(s) = lock_shared().as_mut() {
            s.resume();
        }
    }

    pub fn toggle_pause(&self) {
        if self.is_paused() {
            self.resume();
        } else {
            self.pause();
        }
    }

    /// Start recording input events. Installs the hooks on a fresh thread and
    /// blocks until they are live.
    pub fn start(&mut self) {
        *lock_shared() = Some(RecorderState::new(self.ignore_keys.clone()));

        let (tx, rx) = mpsc::channel();
        let handle = std::thread::spawn(move || hook_thread(tx));
        self.thread_id = rx.recv().unwrap_or(0);
        self.thread = Some(handle);
    }

    /// Stop recording and return the captured macro. Timestamps are rounded to 4
    /// decimals here, mirroring Python's `to_dict`, so the saved file matches the
    /// precision of every existing macro on disk.
    pub fn stop(&mut self) -> Macro {
        // Stop accepting events first (any in-flight callback returns early),
        // then wake the hook thread so it unhooks and exits.
        if let Some(s) = lock_shared().as_mut() {
            s.recording = false;
        }
        if self.thread_id != 0 {
            unsafe {
                PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0);
            }
        }
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
        self.thread_id = 0;

        // Take the events and the total elapsed time together: the trailing
        // idle (now minus the last action, pauses excluded) is captured below
        // so a loop's cadence matches the recording's.
        let (mut events, total_elapsed) = lock_shared()
            .take()
            .map(|s| {
                let total = s.elapsed();
                (s.events, total)
            })
            .unwrap_or_default();
        for e in &mut events {
            e.timestamp = round4(e.timestamp);
        }
        append_trailing_wait(&mut events, total_elapsed);

        let (secs, created_at) = now_unix();
        Macro {
            name: format!("recording_{secs}"),
            record_resolution: self.resolution,
            created_at,
            events,
            ..Macro::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hardware::input::resolve_vk;

    fn state() -> RecorderState {
        RecorderState::new(HashSet::new())
    }

    #[test]
    fn round4_matches_python_round() {
        assert_eq!(round4(1.23456), 1.2346);
        assert_eq!(round4(0.12344), 0.1234);
        assert_eq!(round4(0.12346), 0.1235);
        assert_eq!(round4(0.99321), 0.9932);
        assert_eq!(round4(0.05), 0.05);
        assert_eq!(round4(0.0), 0.0);
    }

    #[test]
    fn trailing_wait_preserves_idle_after_the_last_action() {
        let mut events = vec![new_event(InputEventType::KeyDown, 1.0)];
        append_trailing_wait(&mut events, 3.5);
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].event_type, InputEventType::Wait);
        assert_eq!(events[1].timestamp, 3.5, "stamped at the total elapsed time");
    }

    #[test]
    fn trailing_wait_skips_reaction_time_and_empty_recordings() {
        // Sub-threshold idle is the stop-press reaction, not a deliberate pause.
        let mut events = vec![new_event(InputEventType::KeyDown, 1.0)];
        append_trailing_wait(&mut events, 1.03);
        assert_eq!(events.len(), 1);

        // An all-idle recording (no actions) stays empty.
        let mut empty: Vec<MacroEvent> = Vec::new();
        append_trailing_wait(&mut empty, 5.0);
        assert!(empty.is_empty());
    }

    #[test]
    fn wheel_notches_are_signed_and_floor_divided() {
        // High word 120 = one notch up; 0xFF88 (= -120) = one notch down.
        assert_eq!(wheel_notches(120 << 16), 1);
        assert_eq!(wheel_notches(240 << 16), 2);
        assert_eq!(wheel_notches((-120i32 as u32 as u16 as u32) << 16), -1);
        // A high-resolution partial notch floors to 0, matching pynput.
        assert_eq!(wheel_notches(40 << 16), 0);
    }

    #[test]
    fn xbutton_name_reads_the_high_word() {
        assert_eq!(xbutton_name(1 << 16), Some("x1"));
        assert_eq!(xbutton_name(2 << 16), Some("x2"));
        assert_eq!(xbutton_name(0), None);
    }

    #[test]
    fn key_name_matches_pynput_strings() {
        // Letters lower-case; digits are their character.
        assert_eq!(key_name(0x41), "a");
        assert_eq!(key_name(0x5A), "z");
        assert_eq!(key_name(0x30), "0");
        // Function keys: f3/f4/f6 are the app's ignore hotkeys.
        assert_eq!(key_name(0x72), "f3");
        assert_eq!(key_name(0x7B), "f12");
        // Special keys use pynput's Key.name.
        assert_eq!(key_name(0x20), "space");
        assert_eq!(key_name(0x0D), "enter");
        assert_eq!(key_name(0x1B), "esc");
        // The low-level hook delivers distinguished modifiers; both the specific
        // and generic codes yield pynput's name.
        assert_eq!(key_name(0xA0), "shift_l");
        assert_eq!(key_name(0x10), "shift");
        // OEM punctuation (US layout).
        assert_eq!(key_name(0xBA), ";");
        // pynput names Python's _VK can't resolve are still recorded.
        assert_eq!(key_name(0x90), "num_lock");
        // No character → empty string, exactly as `key.char or ""`.
        assert_eq!(key_name(0x00), "");
    }

    #[test]
    fn key_name_is_the_inverse_of_resolve_vk() {
        // Every key both sides know must round-trip: record a VK → the recorded
        // string resolves straight back to the same VK. This ties the recorder
        // and player halves together.
        let mut vks: Vec<u16> = Vec::new();
        vks.extend(0x41..=0x5Au16); // a-z
        vks.extend(0x30..=0x39u16); // 0-9
        vks.extend(0x70..=0x7Bu16); // f1-f12
        vks.extend([
            0x08, 0x09, 0x0D, 0x10, 0x11, 0x12, 0x13, 0x14, 0x1B, 0x20, 0x21, 0x22, 0x23, 0x24,
            0x25, 0x26, 0x27, 0x28, 0x2C, 0x2D, 0x2E, 0x5B, 0x5C, 0xA0, 0xA1, 0xA2, 0xA3, 0xA4,
            0xA5, 0xBA, 0xBB, 0xBC, 0xBD, 0xBE, 0xBF, 0xC0, 0xDB, 0xDC, 0xDD, 0xDE,
        ]);
        for vk in vks {
            let name = key_name(vk);
            assert_eq!(
                resolve_vk(&name),
                Some(vk),
                "vk {vk:#04x} recorded as {name:?} did not resolve back"
            );
        }
    }

    #[test]
    fn move_dedup_skips_only_exact_same_pixel() {
        let mut s = state();
        s.handle_move(100, 200);
        s.handle_move(100, 200); // same pixel, skipped
        s.handle_move(101, 200); // moved, kept
        assert_eq!(s.events.len(), 2);
        assert_eq!(s.events[0].x, 100);
        assert_eq!(s.events[1].x, 101);

        // A non-move between two identical moves breaks the de-dup (last event
        // is no longer a move), so the repeat is kept.
        s.handle_button(true, 101, 200, "left");
        s.handle_move(101, 200);
        assert_eq!(s.events.len(), 4);
        assert_eq!(s.events[3].event_type, InputEventType::MouseMove);
    }

    #[test]
    fn button_and_scroll_events_carry_their_fields() {
        let mut s = state();
        s.handle_button(true, 10, 20, "right");
        s.handle_button(false, 10, 20, "right");
        s.handle_scroll(10, 20, -3);
        assert_eq!(s.events[0].event_type, InputEventType::MouseDown);
        assert_eq!(s.events[0].button, "right");
        assert_eq!(s.events[1].event_type, InputEventType::MouseUp);
        assert_eq!(s.events[2].event_type, InputEventType::Scroll);
        assert_eq!(s.events[2].delta, -3);
    }

    #[test]
    fn key_events_record_the_name_and_honor_ignore_keys() {
        let mut s = RecorderState::new(HashSet::from(["f3".to_string()]));
        s.handle_key(true, 0x41); // 'a', kept
        s.handle_key(false, 0x41);
        s.handle_key(true, 0x72); // f3, ignored (app hotkey)
        s.handle_key(false, 0x72);
        assert_eq!(s.events.len(), 2);
        assert_eq!(s.events[0].event_type, InputEventType::KeyDown);
        assert_eq!(s.events[0].key, "a");
        assert_eq!(s.events[1].event_type, InputEventType::KeyUp);
    }

    #[test]
    fn paused_recording_drops_events() {
        let mut s = state();
        s.handle_move(1, 1);
        s.pause();
        s.handle_move(2, 2); // dropped while paused
        s.handle_key(true, 0x41);
        assert_eq!(s.events.len(), 1);
        s.resume();
        s.handle_move(3, 3); // captured again
        assert_eq!(s.events.len(), 2);
    }

    /// End-to-end hardware check: install the real hooks, synthesize input with
    /// the `InputController`, and confirm it is captured. Moves the real cursor
    /// and presses keys, so it is ignored by default. Run explicitly with
    /// `cargo test -- --ignored records_synthesized_input`.
    #[test]
    #[ignore = "installs global hooks and moves the real mouse/keyboard"]
    fn records_synthesized_input() {
        use crate::hardware::input::InputController;

        let mut rec = MacroRecorder::new((1920, 1080), HashSet::new());
        rec.start();
        assert!(rec.is_recording());

        let controller = InputController::new();
        controller.move_to(500, 400);
        controller.click(500, 400, "left");
        controller.key_press("a");

        // Injected events reach the hook on its own thread's message pump; give
        // it a moment to drain before tearing the hooks down.
        std::thread::sleep(Duration::from_millis(150));

        let macro_ = rec.stop();
        assert!(!rec.is_recording());

        let has = |t: InputEventType| macro_.events.iter().any(|e| e.event_type == t);
        assert!(has(InputEventType::MouseMove), "no mouse move captured");
        assert!(has(InputEventType::MouseDown), "no mouse down captured");
        assert!(has(InputEventType::MouseUp), "no mouse up captured");
        assert!(
            macro_
                .events
                .iter()
                .any(|e| e.event_type == InputEventType::KeyDown && e.key == "a"),
            "no 'a' key-down captured"
        );
        // Timestamps were rounded to 4 decimals on the way out.
        for e in &macro_.events {
            assert_eq!(e.timestamp, round4(e.timestamp));
        }
    }
}
