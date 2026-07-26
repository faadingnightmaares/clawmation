//! Input simulation: raw Win32 `SendInput` for 1:1 macro playback.
//!
//! Faithful port of `anime_macro/input.py::InputController`. pyautogui-style
//! libraries add per-call pauses and extra move-to-target steps that make replay
//! feel laggy; for exact macros we inject mouse/keyboard events directly via
//! `SendInput` with zero artificial delay. Timing is owned entirely by the
//! player (absolute timestamps).
//!
//! Intentionally not ported (dead in the shipped Python app; see
//! MIGRATION-NOTES):
//!   * the `humanize` jitter and `click_delay_ms` inter-click sleep: the
//!     controller is always constructed with defaults, so neither ever fires and
//!     every recorded click is a single click. The user-facing "humanize clicks"
//!     option lives in the *guard* engine (bezier-move then click), not here.
//!   * `double_click` / `right_click` / `hotkey` / `type_text`: no live caller.
//!
//! The Python 1-second `GetSystemMetrics` cache is also dropped: it existed to
//! avoid ctypes marshalling overhead on every move; a direct syscall in Rust is
//! negligible and the result is behaviorally identical.

use std::ffi::c_void;
use std::mem::size_of;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use windows_sys::Win32::Foundation::POINT;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    MapVirtualKeyW, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT,
    KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, MAPVK_VK_TO_VSC, MOUSEEVENTF_ABSOLUTE,
    MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP,
    MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK,
    MOUSEEVENTF_WHEEL, MOUSEINPUT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetSystemMetrics, SetCursorPos, SystemParametersInfoW, SM_CXSCREEN,
    SM_CXVIRTUALSCREEN, SM_CYSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
    SPI_GETMOUSE, SPI_GETMOUSESPEED, SPI_SETMOUSE, SPI_SETMOUSESPEED,
};

use crate::hardware::dpi::PerMonitorAware;
use crate::models::macro_def::{InputEventType, MacroEvent};

/// One wheel notch, as Win32 defines it.
const WHEEL_DELTA: i32 = 120;

// ── Pure helpers (unit-tested without hardware) ──────────────────────────────

/// Map screen pixels → `SendInput` absolute coords (0..65535 over the virtual
/// desktop), given the virtual-screen origin/size. `round`-to-nearest (ties to
/// even, matching Python's `round()`) avoids the 0.5px downward bias `int()`
/// would introduce; the result is clamped because multi-monitor edges can
/// produce out-of-range values.
fn to_absolute(x: i32, y: i32, ox: i32, oy: i32, w: i32, h: i32) -> (i32, i32) {
    let ax = ((x - ox) as f64 * 65535.0 / (w - 1).max(1) as f64).round_ties_even() as i32;
    let ay = ((y - oy) as f64 * 65535.0 / (h - 1).max(1) as f64).round_ties_even() as i32;
    (ax.clamp(0, 65535), ay.clamp(0, 65535))
}

/// `SendInput` flag for a mouse button press (`down`) or release.
fn button_flags(button: &str, down: bool) -> u32 {
    match button.to_ascii_lowercase().as_str() {
        "right" | "secondary" => {
            if down {
                MOUSEEVENTF_RIGHTDOWN
            } else {
                MOUSEEVENTF_RIGHTUP
            }
        }
        "middle" | "center" => {
            if down {
                MOUSEEVENTF_MIDDLEDOWN
            } else {
                MOUSEEVENTF_MIDDLEUP
            }
        }
        _ => {
            if down {
                MOUSEEVENTF_LEFTDOWN
            } else {
                MOUSEEVENTF_LEFTUP
            }
        }
    }
}

/// Named pynput keys → virtual-key code. Alphanumerics (`a`-`z`, `0`-`9`) are
/// resolved by the single-char branch in [`resolve_vk`] instead of listed here,
/// since `ord(upper)` / `ord(digit)` give exactly the values Python's `_VK`
/// loops build. The trailing OEM punctuation entries (US layout) stand in for
/// Python's `pydirectinput` fallback: they resolve to the same scan codes
/// `pydirectinput` would emit (verified via `MapVirtualKeyW`), so a punctuation
/// key still presses correctly instead of being dropped.
fn vk_lookup(k: &str) -> Option<u16> {
    Some(match k {
        "backspace" => 0x08,
        "tab" => 0x09,
        "enter" | "return" => 0x0D,
        "shift" => 0x10,
        "ctrl" | "control" => 0x11,
        "alt" => 0x12,
        "pause" => 0x13,
        "caps_lock" => 0x14,
        "escape" | "esc" => 0x1B,
        "space" => 0x20,
        "page_up" => 0x21,
        "page_down" => 0x22,
        "end" => 0x23,
        "home" => 0x24,
        "left" => 0x25,
        "up" => 0x26,
        "right" => 0x27,
        "down" => 0x28,
        "print_screen" => 0x2C,
        "insert" => 0x2D,
        "delete" => 0x2E,
        "cmd" | "cmd_l" | "win" => 0x5B,
        "cmd_r" => 0x5C,
        "shift_l" => 0xA0,
        "shift_r" => 0xA1,
        "ctrl_l" => 0xA2,
        "ctrl_r" => 0xA3,
        "alt_l" => 0xA4,
        "alt_r" => 0xA5,
        "f1" => 0x70,
        "f2" => 0x71,
        "f3" => 0x72,
        "f4" => 0x73,
        "f5" => 0x74,
        "f6" => 0x75,
        "f7" => 0x76,
        "f8" => 0x77,
        "f9" => 0x78,
        "f10" => 0x79,
        "f11" => 0x7A,
        "f12" => 0x7B,
        ";" => 0xBA,
        "=" => 0xBB,
        "," => 0xBC,
        "-" => 0xBD,
        "." => 0xBE,
        "/" => 0xBF,
        "`" => 0xC0,
        "[" => 0xDB,
        "\\" => 0xDC,
        "]" => 0xDD,
        "'" => 0xDE,
        _ => return None,
    })
}

/// Resolve a recorded key string to a virtual-key code, mirroring Python's
/// `_resolve_vk`: exact map hit first, then a single character (letter →
/// uppercase VK, digit → its ASCII code), then a `Key.`-prefixed pynput name.
/// Anything unresolvable returns `None` (a no-op press); this app's macros
/// never contain such keys.
///
/// Exposed to the crate so the recorder's `key_name` can be verified as its
/// exact inverse (record a VK → replay resolves back to the same VK).
pub(crate) fn resolve_vk(key: &str) -> Option<u16> {
    if key.is_empty() {
        return None;
    }
    let k = key.to_ascii_lowercase();
    if let Some(vk) = vk_lookup(&k) {
        return Some(vk);
    }
    if k.chars().count() == 1 {
        let ch = k.chars().next().unwrap();
        return if ch.is_ascii_alphabetic() {
            Some(ch.to_ascii_uppercase() as u16)
        } else if ch.is_ascii_alphanumeric() {
            Some(ch as u16)
        } else {
            None
        };
    }
    vk_lookup(&k.replace("key.", ""))
}

// ── Humanized-move math (unit-tested without hardware) ───────────────────────

/// A cubic Bézier's scalar value at parameter `t` for control values `a`..`d`.
/// Evaluated per axis to bend an autonomous cursor move into a natural arc.
fn cubic_bezier(t: f64, a: f64, b: f64, c: f64, d: f64) -> f64 {
    let mt = 1.0 - t;
    mt * mt * mt * a + 3.0 * mt * mt * t * b + 3.0 * mt * t * t * c + t * t * t * d
}

/// Smoothstep easing (`3t² − 2t³`): eases velocity in then out, so the move
/// ramps up and back down instead of travelling at a constant speed.
fn smoothstep(t: f64) -> f64 {
    t * t * (3.0 - 2.0 * t)
}

/// A tiny SplitMix64 PRNG, enough to jitter the two Bézier control points so an
/// autonomous path is never identical twice. The app carries no `rand`
/// dependency and the exact distribution is irrelevant here (this is anti-cheat
/// noise, not anything a user observes), so a hand-rolled mixer fits the
/// codebase's minimal-dependency ethos. Seeded per move from the wall clock.
struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform `f64` in `[lo, hi)`, mirroring `random.uniform`. Uses the top 53
    /// bits (the mantissa width) for an evenly spaced unit float.
    fn uniform(&mut self, lo: f64, hi: f64) -> f64 {
        let unit = (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64;
        lo + (hi - lo) * unit
    }
}

/// Seed a PRNG from the wall clock (nanoseconds). SplitMix64 avalanches even a
/// low-entropy, near-sequential seed, so a coarse clock is fine as a source.
fn clock_seed() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E37_79B9_7F4A_7C15)
}

// ── Win32 plumbing ───────────────────────────────────────────────────────────

/// `(origin_x, origin_y, width, height)` of the virtual desktop. Falls back to
/// the primary monitor's size when the virtual metrics report zero, matching
/// `input.py::_virtual_screen`.
fn virtual_screen() -> (i32, i32, i32, i32) {
    unsafe {
        let ox = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let oy = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let vw = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let w = if vw != 0 { vw } else { GetSystemMetrics(SM_CXSCREEN) };
        let vh = GetSystemMetrics(SM_CYVIRTUALSCREEN);
        let h = if vh != 0 { vh } else { GetSystemMetrics(SM_CYSCREEN) };
        (ox, oy, w.max(1), h.max(1))
    }
}

fn send(inputs: &[INPUT]) {
    if inputs.is_empty() {
        return;
    }
    // A short send is best-effort (a foreground app can block injection); the
    // Python port only debug-logs it, so we likewise proceed.
    unsafe {
        SendInput(inputs.len() as u32, inputs.as_ptr(), size_of::<INPUT>() as i32);
    }
}

fn mouse_input(flags: u32, dx: i32, dy: i32, data: i32) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: data as u32,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn key_input(vk: u16, up: bool) -> INPUT {
    let scan = unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC) } as u16;
    let flags = KEYEVENTF_SCANCODE | if up { KEYEVENTF_KEYUP } else { 0 };
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: 0,
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

// ── Mouse-acceleration control ───────────────────────────────────────────────
// Relative `MOUSEEVENTF_MOVE` deltas are scaled by "Enhance pointer precision"
// (mouse acceleration) and the pointer-speed slider. For exact replay we pin
// speed to the neutral 10/20 and turn acceleration off for the duration of a
// run, then restore the user's settings.

fn get_mouse_settings() -> Option<(i32, i32)> {
    unsafe {
        let mut speed: i32 = 10;
        let ok_speed = SystemParametersInfoW(
            SPI_GETMOUSESPEED,
            0,
            &mut speed as *mut i32 as *mut c_void,
            0,
        );
        let mut accel: [i32; 3] = [0, 0, 0];
        let ok_accel =
            SystemParametersInfoW(SPI_GETMOUSE, 0, accel.as_mut_ptr() as *mut c_void, 0);
        if ok_speed == 0 || ok_accel == 0 {
            return None;
        }
        Some((speed, accel[2]))
    }
}

fn set_mouse_settings(speed: i32, accel: i32) {
    unsafe {
        // SPI_SETMOUSESPEED passes the speed *as* pvParam, not through it.
        SystemParametersInfoW(SPI_SETMOUSESPEED, 0, speed as usize as *mut c_void, 0);
        let mut vals: [i32; 3] = [0, 0, accel];
        SystemParametersInfoW(SPI_SETMOUSE, 0, vals.as_mut_ptr() as *mut c_void, 0);
    }
}

/// RAII form of Python's `_NoAcceleration` context manager: disables mouse
/// acceleration on construction and restores the previous settings on drop. If
/// the current settings can't be read it leaves them untouched (rather than
/// clobbering them with a guess).
pub struct NoAcceleration {
    previous: Option<(i32, i32)>,
}

impl NoAcceleration {
    pub fn new() -> Self {
        let previous = get_mouse_settings();
        if previous.is_some() {
            // speed 10 = neutral 1:1, accel 0 = no "enhance pointer precision".
            set_mouse_settings(10, 0);
        }
        Self { previous }
    }
}

impl Default for NoAcceleration {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for NoAcceleration {
    fn drop(&mut self) {
        if let Some((speed, accel)) = self.previous {
            set_mouse_settings(speed, accel);
        }
    }
}

// ── DPI handling ─────────────────────────────────────────────────────────────
// Absolute cursor placement (`SetCursorPos` and the absolute `SendInput`
// normalization) is DPI-virtualized unless the *calling thread* is Per-Monitor
// aware: on a scaled display an unaware thread stretches every absolute
// coordinate by the DPI factor, so a recorded move to a UI button lands past
// it. `move_to` therefore holds a `PerMonitorAware` guard (see `hardware::dpi`)
// so recorded physical pixels are placed physically, and `bezier_move_to`
// holds one too because it reads the curve's start point with `GetCursorPos`.
// Relative `MOUSEEVENTF_MOVE` deltas are raw counts rather than screen
// coordinates, so `move_relative` passes the recorded physical delta through
// unchanged for Raw Input consumers. Windows can still scale the visible cursor
// path, so the player follows each relative event with a `SetCursorPos`-only
// reconciliation. The recorder guards its hook thread, the player guards the
// playback thread, and `screen_size` guards the resolution query the same way.

// ── Controller ───────────────────────────────────────────────────────────────

/// Game-safe input simulation via raw `SendInput` (zero per-call pause).
pub struct InputController;

impl InputController {
    pub fn new() -> Self {
        Self
    }

    /// Instant absolute cursor move.
    ///
    /// Uses both `SendInput` (for apps that read the input queue) and
    /// `SetCursorPos`; the latter is essential because many games and
    /// DirectInput apps ignore `SendInput` mouse moves entirely, and without it
    /// the cursor doesn't move.
    pub fn move_to(&self, x: i32, y: i32) {
        // Physical coordinates in, physical placement out (see `PerMonitorAware`):
        // without this a scaled display re-expands the recorded physical point by
        // the DPI factor and the cursor lands past its target.
        let _aware = PerMonitorAware::new();
        let (ox, oy, w, h) = virtual_screen();
        let (ax, ay) = to_absolute(x, y, ox, oy, w, h);
        send(&[mouse_input(
            MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
            ax,
            ay,
            0,
        )]);
        unsafe {
            SetCursorPos(x, y);
        }
    }

    /// Move the cursor by a relative delta.
    ///
    /// This is the only way to drive camera rotation in games like Roblox: they
    /// read mouse movement via Windows Raw Input (`WM_INPUT`), which is
    /// generated by relative `MOUSEEVENTF_MOVE` events. `SetCursorPos` and
    /// absolute moves reposition the cursor but produce no relative-motion
    /// deltas, so the camera never rotates. A single relative send moves the
    /// visible cursor *and* generates `WM_INPUT`, so no `SetCursorPos` is needed
    /// (that would double the movement).
    pub fn move_relative(&self, dx: i32, dy: i32) {
        if dx == 0 && dy == 0 {
            return;
        }
        // `MOUSEINPUT::dx`/`dy` are Raw Input counts, not screen coordinates.
        // Preserve the recorded delta for game camera movement; the player
        // separately reconciles the visible cursor to its physical target.
        let (dx, dy) = crate::hardware::dpi::relative_counts(dx, dy);
        send(&[mouse_input(MOUSEEVENTF_MOVE, dx, dy, 0)]);
    }

    /// Reconcile the visible cursor with a recorded physical target without
    /// injecting another relative event. Relative `SendInput` is still sent
    /// first so Raw Input consumers receive the recorded movement; this call
    /// removes any cursor drift introduced by display scaling, pointer speed,
    /// acceleration, rounding, or a game temporarily recentering the pointer.
    pub(crate) fn sync_cursor_to(&self, x: i32, y: i32) {
        let _aware = PerMonitorAware::new();
        unsafe {
            SetCursorPos(x, y);
        }
    }

    /// The smallest motion that still counts as mouse activity: 1px out and 1px
    /// back, so the cursor ends exactly where it started no matter how often it
    /// fires — subpixel moves do not exist, hardware input is whole pixels, and
    /// this is the floor. Relative halves (not an absolute round-trip) so Raw
    /// Input consumers see genuine movement, with a frame-sized pause between
    /// them so a game polling the cursor position once per frame still catches
    /// the displaced position rather than only the cancellation.
    pub fn nudge(&self) {
        self.move_relative(1, 0);
        std::thread::sleep(Duration::from_millis(16));
        self.move_relative(-1, 0);
    }

    /// Human-like cursor move to `(x, y)` along a cubic Bézier curve.
    ///
    /// Used for AUTONOMOUS clicks (vision triggers, guards): a straight-line
    /// teleport to a target is a dead giveaway to anti-cheat heuristics, so the
    /// cursor travels a gently curved path with an ease-in/out velocity profile
    /// that reads as human. `duration <= 0` picks a distance-scaled duration.
    /// Recorded-macro replay never calls this; it replays the user's exact
    /// sampled path instead. Ported from `input.py::bezier_move_to`.
    pub fn bezier_move_to(&self, x: i32, y: i32, duration: f64) {
        // Physical coordinates in, physical path out — this reads the curve's
        // start point with `GetCursorPos`, which is DPI-virtualized per thread,
        // so the guard covers the start read as well as the moves that follow.
        let _aware = PerMonitorAware::new();
        // The current cursor position is the curve's start point.
        let mut pt = POINT { x: 0, y: 0 };
        unsafe {
            GetCursorPos(&mut pt);
        }
        let (sx, sy) = (pt.x as f64, pt.y as f64);
        let (tx, ty) = (x as f64, y as f64);
        let (dx, dy) = (tx - sx, ty - sy);
        let dist = (dx * dx + dy * dy).sqrt();
        if dist < 2.0 {
            // Already there (or trivially close), so a plain move is fine.
            self.move_to(x, y);
            return;
        }

        // Scale steps and duration to the distance so short hops stay snappy and
        // long sweeps stay smooth; cap both so neither stalls.
        let duration = if duration <= 0.0 {
            (dist / 2600.0).clamp(0.08, 0.45)
        } else {
            duration
        };
        let steps = (dist / 14.0).clamp(12.0, 90.0) as i32;

        // Two control points offset perpendicular to the travel direction bend
        // the path into an arc; the offsets scale with distance but stay modest.
        let ang = dy.atan2(dx);
        let perp = ang + std::f64::consts::FRAC_PI_2;
        let spread = (dist * 0.18).min(120.0);
        let mut rng = SplitMix64::new(clock_seed());
        let c1x = sx + dx * rng.uniform(0.2, 0.4) + perp.cos() * rng.uniform(-spread, spread);
        let c1y = sy + dy * rng.uniform(0.2, 0.4) + perp.sin() * rng.uniform(-spread, spread);
        let c2x = sx + dx * rng.uniform(0.6, 0.8) + perp.cos() * rng.uniform(-spread, spread);
        let c2y = sy + dy * rng.uniform(0.6, 0.8) + perp.sin() * rng.uniform(-spread, spread);

        let start = Instant::now();
        for i in 1..=steps {
            let target_t = i as f64 / steps as f64;
            // Ease with a smoothstep curve so velocity ramps up then down.
            let eased = smoothstep(target_t);
            let px = cubic_bezier(eased, sx, c1x, c2x, tx).round_ties_even() as i32;
            let py = cubic_bezier(eased, sy, c1y, c2y, ty).round_ties_even() as i32;
            self.move_to(px, py);
            // Sleep to this step's absolute target time (drift-free).
            let target = Duration::from_secs_f64(duration * target_t);
            let elapsed = start.elapsed();
            if target > elapsed {
                std::thread::sleep(target - elapsed);
            }
        }
        // Land exactly on the target.
        self.move_to(x, y);
    }

    pub fn mouse_down(&self, pos: Option<(i32, i32)>, button: &str) {
        if let Some((x, y)) = pos {
            self.move_to(x, y);
        }
        send(&[mouse_input(button_flags(button, true), 0, 0, 0)]);
    }

    pub fn mouse_up(&self, pos: Option<(i32, i32)>, button: &str) {
        if let Some((x, y)) = pos {
            self.move_to(x, y);
        }
        send(&[mouse_input(button_flags(button, false), 0, 0, 0)]);
    }

    pub fn click(&self, x: i32, y: i32, button: &str) {
        self.move_to(x, y);
        send(&[
            mouse_input(button_flags(button, true), 0, 0, 0),
            mouse_input(button_flags(button, false), 0, 0, 0),
        ]);
    }

    pub fn key_press(&self, key: &str) {
        self.key_down(key);
        self.key_up(key);
    }

    pub fn key_down(&self, key: &str) {
        if let Some(vk) = resolve_vk(key) {
            send(&[key_input(vk, false)]);
        }
    }

    pub fn key_up(&self, key: &str) {
        if let Some(vk) = resolve_vk(key) {
            send(&[key_input(vk, true)]);
        }
    }

    /// Type a string one character at a time (`InputController.type_text`). Each
    /// char is a key press followed by a 20 ms pause (Python's default
    /// `interval=0.02`, always truthy, so it pauses after every char, the last
    /// included). The AI `type` step is the only caller, and it never overrides the
    /// interval, so the parameter is dropped. A character `resolve_vk` cannot map is
    /// silently skipped, exactly as `key_down` already behaves across this layer
    /// (Python's pydirectinput fallback was not ported).
    pub fn type_text(&self, text: &str) {
        for ch in text.chars() {
            self.key_press(&ch.to_string());
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    /// Positive `clicks` scrolls up.
    pub fn scroll(&self, clicks: i32, pos: Option<(i32, i32)>) {
        if let Some((x, y)) = pos {
            self.move_to(x, y);
        }
        send(&[mouse_input(MOUSEEVENTF_WHEEL, 0, 0, clicks * WHEEL_DELTA)]);
    }

    pub fn wait(&self, seconds: f64) {
        if seconds > 0.0 {
            std::thread::sleep(std::time::Duration::from_secs_f64(seconds));
        }
    }

    /// Replay a single recorded event with an optional coordinate transform
    /// (offset + scale), mirroring `InputController.replay_event`. `CHECKPOINT`
    /// events are handled by the player, not here, so they're a no-op.
    pub fn replay_event(
        &self,
        event: &MacroEvent,
        x_offset: i32,
        y_offset: i32,
        x_scale: f64,
        y_scale: f64,
    ) {
        let x = (event.x as f64 * x_scale) as i32 + x_offset;
        let y = (event.y as f64 * y_scale) as i32 + y_offset;

        match event.event_type {
            InputEventType::MouseMove => self.move_to(x, y),
            InputEventType::MouseDown => self.mouse_down(Some((x, y)), &event.button),
            InputEventType::MouseUp => self.mouse_up(Some((x, y)), &event.button),
            // Legacy macros: a synthetic click = down+up at the point.
            InputEventType::MouseClick => self.click(x, y, &event.button),
            InputEventType::KeyPress => self.key_press(&event.key),
            InputEventType::KeyDown => self.key_down(&event.key),
            InputEventType::KeyUp => self.key_up(&event.key),
            InputEventType::Scroll => self.scroll(event.delta as i32, Some((x, y))),
            InputEventType::Wait => self.wait(event.duration),
            InputEventType::Checkpoint => {}
        }
    }
}

impl Default for InputController {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_absolute_maps_corners_and_clamps() {
        // Origin pixel → 0, far corner → full-scale 65535.
        assert_eq!(to_absolute(0, 0, 0, 0, 1920, 1080), (0, 0));
        assert_eq!(to_absolute(1919, 1079, 0, 0, 1920, 1080), (65535, 65535));
        // A non-zero virtual-screen origin is subtracted off first.
        assert_eq!(to_absolute(100, 50, 100, 50, 1920, 1080), (0, 0));
        // Out-of-range (multi-monitor edge) clamps into [0, 65535].
        assert_eq!(to_absolute(5000, 5000, 0, 0, 1920, 1080), (65535, 65535));
        assert_eq!(to_absolute(-500, -500, 0, 0, 1920, 1080), (0, 0));
    }

    #[test]
    fn button_flags_cover_every_alias() {
        assert_eq!(button_flags("left", true), MOUSEEVENTF_LEFTDOWN);
        assert_eq!(button_flags("left", false), MOUSEEVENTF_LEFTUP);
        assert_eq!(button_flags("right", true), MOUSEEVENTF_RIGHTDOWN);
        assert_eq!(button_flags("secondary", false), MOUSEEVENTF_RIGHTUP);
        assert_eq!(button_flags("middle", true), MOUSEEVENTF_MIDDLEDOWN);
        assert_eq!(button_flags("center", false), MOUSEEVENTF_MIDDLEUP);
        // Unknown / empty falls back to left, like Python's `_button_flags`.
        assert_eq!(button_flags("", true), MOUSEEVENTF_LEFTDOWN);
        assert_eq!(button_flags("bogus", false), MOUSEEVENTF_LEFTUP);
    }

    #[test]
    fn resolve_vk_matches_python_vk_table() {
        // Letters: upper- and lower-case both resolve to the uppercase VK.
        assert_eq!(resolve_vk("a"), Some(0x41));
        assert_eq!(resolve_vk("A"), Some(0x41));
        assert_eq!(resolve_vk("z"), Some(0x5A));
        // Digits map to their ASCII code (== 0x30 + n).
        assert_eq!(resolve_vk("5"), Some(0x35));
        // Named specials and function keys.
        assert_eq!(resolve_vk("space"), Some(0x20));
        assert_eq!(resolve_vk("enter"), Some(0x0D));
        assert_eq!(resolve_vk("return"), Some(0x0D));
        assert_eq!(resolve_vk("ctrl"), Some(0x11));
        assert_eq!(resolve_vk("f9"), Some(0x78));
        assert_eq!(resolve_vk("f12"), Some(0x7B));
        // `Key.`-prefixed pynput name.
        assert_eq!(resolve_vk("Key.space"), Some(0x20));
        // OEM punctuation (standing in for the pydirectinput fallback).
        assert_eq!(resolve_vk(";"), Some(0xBA));
        assert_eq!(resolve_vk("."), Some(0xBE));
        assert_eq!(resolve_vk("/"), Some(0xBF));
        // Empty / unresolvable → no key.
        assert_eq!(resolve_vk(""), None);
    }

    #[test]
    fn cubic_bezier_hits_its_endpoints() {
        // At t=0 the curve equals the first control value; at t=1, the last.
        assert_eq!(cubic_bezier(0.0, 5.0, 10.0, 20.0, 40.0), 5.0);
        assert_eq!(cubic_bezier(1.0, 5.0, 10.0, 20.0, 40.0), 40.0);
        // Evenly spaced controls trace a straight ramp, linear at the midpoint.
        assert!((cubic_bezier(0.5, 0.0, 1.0, 2.0, 3.0) - 1.5).abs() < 1e-9);
    }

    #[test]
    fn smoothstep_eases_symmetrically() {
        assert_eq!(smoothstep(0.0), 0.0);
        assert_eq!(smoothstep(1.0), 1.0);
        assert!((smoothstep(0.5) - 0.5).abs() < 1e-9);
        // Monotone across the unit interval.
        assert!(smoothstep(0.25) < smoothstep(0.75));
    }

    #[test]
    fn splitmix64_is_deterministic_and_bounded() {
        // Same seed → same sequence (a move is reproducible under a fixed seed).
        let mut a = SplitMix64::new(42);
        let mut b = SplitMix64::new(42);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
        // Different seeds diverge on the first draw.
        assert_ne!(SplitMix64::new(1).next_u64(), SplitMix64::new(2).next_u64());
        // `uniform` stays inside [lo, hi).
        let mut r = SplitMix64::new(7);
        for _ in 0..1000 {
            let v = r.uniform(-3.0, 5.0);
            assert!((-3.0..5.0).contains(&v), "uniform out of range: {v}");
        }
    }

    /// Moves the *real* cursor, so it's ignored by default. Run explicitly with
    /// `cargo test -- --ignored move_to_round_trips_cursor_position`.
    #[test]
    #[ignore = "moves the real mouse cursor"]
    fn move_to_round_trips_cursor_position() {
        // Mirror `run()`: raise the process before anything moves. The readback
        // below is deliberately UNGUARDED, so on a scaled display it reports the
        // logical position if the raise did not take — failing the test instead
        // of masking a regression that would shift every replayed click.
        crate::hardware::dpi::raise_process_to_per_monitor_v2();
        let controller = InputController::new();
        controller.move_to(400, 300);
        let mut pt = POINT { x: 0, y: 0 };
        unsafe {
            GetCursorPos(&mut pt);
        }
        // SetCursorPos lands exactly; allow a pixel of slack for DPI rounding.
        assert!((pt.x - 400).abs() <= 1, "cursor x was {}", pt.x);
        assert!((pt.y - 300).abs() <= 1, "cursor y was {}", pt.y);
    }
}
