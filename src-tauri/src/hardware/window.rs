//! Visible-window discovery and foreground switching for Anti-AFK.

use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, RECT};
use windows_sys::Win32::System::Threading::{
    AttachThreadInput, GetCurrentProcessId, GetCurrentThreadId,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, EnumWindows, GetForegroundWindow, GetWindow, GetWindowRect,
    GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindow,
    IsWindowVisible, SetForegroundWindow, ShowWindow, GW_OWNER, SW_RESTORE,
};

const FOCUS_ATTEMPTS: usize = 4;
const FOCUS_RETRY_DELAY: Duration = Duration::from_millis(45);
const FOCUS_SETTLE_DELAY: Duration = Duration::from_millis(180);
const WATCH_FOCUS_SETTLE_DELAY: Duration = Duration::from_millis(100);
const INPUT_RELEASE_ATTEMPTS: usize = 3;
const INPUT_RELEASE_RETRY_DELAY: Duration = Duration::from_millis(20);
const JUMP_HOLD: Duration = Duration::from_millis(140);
const WALK_HOLD: Duration = Duration::from_millis(180);
const KEY_SETTLE_DELAY: Duration = Duration::from_millis(35);
const CAMERA_BUTTON_SETTLE: Duration = Duration::from_millis(40);
const CAMERA_TRAVEL_SETTLE: Duration = Duration::from_millis(60);
const CAMERA_DELTA: i32 = 24;

use crate::engine::anti_afk::{AntiAfkAction, AntiAfkPlatform};
use crate::hardware::dpi::PerMonitorAware;
use crate::hardware::input::InputController;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SelectableWindow {
    pub id: String,
    pub title: String,
    pub pid: u32,
}

pub fn list_selectable_windows() -> Vec<SelectableWindow> {
    let mut windows = Vec::<SelectableWindow>::new();
    unsafe {
        let _ = EnumWindows(
            Some(collect_window),
            &mut windows as *mut Vec<SelectableWindow> as LPARAM,
        );
    }
    windows.sort_by(|a, b| {
        a.title
            .to_ascii_lowercase()
            .cmp(&b.title.to_ascii_lowercase())
            .then(a.pid.cmp(&b.pid))
    });
    windows
}

unsafe extern "system" fn collect_window(hwnd: HWND, data: LPARAM) -> BOOL {
    let visible = IsWindowVisible(hwnd) != 0;
    let owner = GetWindow(hwnd, GW_OWNER);
    let title = window_title(hwnd);
    let mut pid = 0;
    GetWindowThreadProcessId(hwnd, &mut pid);
    let own_pid = GetCurrentProcessId();

    if is_selectable(visible, !owner.is_null(), &title, pid, own_pid) {
        let windows = &mut *(data as *mut Vec<SelectableWindow>);
        windows.push(SelectableWindow {
            id: selectable_window_id(hwnd, pid),
            title,
            pid,
        });
    }
    1
}

fn is_selectable(visible: bool, owned: bool, title: &str, pid: u32, own_pid: u32) -> bool {
    visible && !owned && !title.trim().is_empty() && pid != 0 && pid != own_pid
}

unsafe fn window_title(hwnd: HWND) -> String {
    let len = GetWindowTextLengthW(hwnd);
    if len <= 0 {
        return String::new();
    }
    let mut buffer = vec![0u16; len as usize + 1];
    let copied = GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32);
    String::from_utf16_lossy(&buffer[..copied.max(0) as usize])
}

fn hwnd_to_id(hwnd: HWND) -> String {
    format!("{:X}", hwnd as usize)
}

fn selectable_window_id(hwnd: HWND, pid: u32) -> String {
    format!("{}:{pid}", hwnd_to_id(hwnd))
}

fn id_to_hwnd(id: &str) -> Option<HWND> {
    let raw = id.split(':').next().unwrap_or(id);
    let raw = raw
        .strip_prefix("0x")
        .or_else(|| raw.strip_prefix("0X"))
        .unwrap_or(raw);
    usize::from_str_radix(raw, 16)
        .ok()
        .map(|value| value as HWND)
}

fn id_process_id(id: &str) -> Option<u32> {
    id.split_once(':')?.1.parse().ok()
}

pub fn is_window_id(id: &str) -> bool {
    id_to_hwnd(id).is_some_and(|hwnd| unsafe {
        if IsWindow(hwnd) == 0 {
            return false;
        }
        match id_process_id(id) {
            Some(expected_pid) => {
                let mut pid = 0;
                GetWindowThreadProcessId(hwnd, &mut pid);
                pid == expected_pid
            }
            None => true,
        }
    })
}

pub fn foreground_window_id() -> Option<String> {
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.is_null() {
        return None;
    }
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    Some(selectable_window_id(hwnd, pid))
}

struct WindowAtPoint {
    x: i32,
    y: i32,
    own_pid: u32,
    hwnd: HWND,
    pid: u32,
}

fn point_in_rect(x: i32, y: i32, rect: RECT) -> bool {
    x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom
}

unsafe extern "system" fn collect_window_at_point(hwnd: HWND, data: LPARAM) -> BOOL {
    let search = &mut *(data as *mut WindowAtPoint);
    if !search.hwnd.is_null() || IsWindowVisible(hwnd) == 0 {
        return 1;
    }

    let owner = GetWindow(hwnd, GW_OWNER);
    let title = window_title(hwnd);
    let mut pid = 0;
    GetWindowThreadProcessId(hwnd, &mut pid);
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    if is_selectable(true, !owner.is_null(), &title, pid, search.own_pid)
        && GetWindowRect(hwnd, &mut rect) != 0
        && point_in_rect(search.x, search.y, rect)
    {
        search.hwnd = hwnd;
        search.pid = pid;
        return 0;
    }
    1
}

/// Focus the external top-level window beneath a physical screen point.
///
/// Watch runs while Clawmation may still be foreground. Mouse movement alone
/// can paint Roblox's hover state, while its first press is consumed by window
/// activation and keyboard/raw-mouse input still goes to Clawmation. Enumerating
/// top to bottom and skipping this process also reaches the game beneath our
/// capture-excluded windows.
pub fn focus_window_at_point(x: i32, y: i32) -> Result<(), String> {
    let _aware = PerMonitorAware::new();
    let mut search = WindowAtPoint {
        x,
        y,
        own_pid: unsafe { GetCurrentProcessId() },
        hwnd: null_mut(),
        pid: 0,
    };
    unsafe {
        let _ = EnumWindows(
            Some(collect_window_at_point),
            &mut search as *mut WindowAtPoint as LPARAM,
        );
    }
    if search.hwnd.is_null() {
        // A desktop point has no app to activate. Keep the action useful by
        // sending it to the current foreground window instead of dropping it.
        return Ok(());
    }
    if unsafe { GetForegroundWindow() == search.hwnd } {
        return Ok(());
    }
    focus_window_id(&selectable_window_id(search.hwnd, search.pid))?;
    std::thread::sleep(WATCH_FOCUS_SETTLE_DELAY);
    Ok(())
}

pub fn focus_window_id(id: &str) -> Result<(), String> {
    let hwnd = id_to_hwnd(id).ok_or_else(|| "invalid window id".to_string())?;
    if !is_window_id(id) {
        return Err("window is no longer available".to_string());
    }

    for _ in 0..FOCUS_ATTEMPTS {
        let requested = unsafe {
            if IsIconic(hwnd) != 0 {
                ShowWindow(hwnd, SW_RESTORE);
            }

            let foreground = GetForegroundWindow();
            let current_thread = GetCurrentThreadId();
            let foreground_thread = if !foreground.is_null() {
                GetWindowThreadProcessId(foreground, null_mut())
            } else {
                0
            };
            let attached = foreground_thread != 0
                && foreground_thread != current_thread
                && AttachThreadInput(current_thread, foreground_thread, 1) != 0;

            let _ = BringWindowToTop(hwnd);
            let ok = SetForegroundWindow(hwnd) != 0;

            if attached {
                let _ = AttachThreadInput(current_thread, foreground_thread, 0);
            }
            ok
        };

        if requested && unsafe { GetForegroundWindow() == hwnd } {
            return Ok(());
        }
        std::thread::sleep(FOCUS_RETRY_DELAY);
    }
    Err("Windows refused to focus the selected window".to_string())
}

pub struct NativeAntiAfkPlatform {
    controller: Arc<InputController>,
    walk_right: AtomicBool,
}

impl NativeAntiAfkPlatform {
    pub fn new(controller: Arc<InputController>) -> Self {
        Self {
            controller,
            walk_right: AtomicBool::new(true),
        }
    }
}

impl AntiAfkPlatform for NativeAntiAfkPlatform {
    fn is_window(&self, id: &str) -> bool {
        is_window_id(id)
    }

    fn foreground_window(&self) -> Option<String> {
        foreground_window_id()
    }

    fn focus_window(&self, id: &str) -> Result<(), String> {
        focus_window_id(id)?;
        std::thread::sleep(FOCUS_SETTLE_DELAY);
        Ok(())
    }

    fn perform_action(&self, action: AntiAfkAction) -> Result<(), String> {
        perform_input_action(&*self.controller, action, &self.walk_right)
    }
}

trait AntiAfkInput {
    fn key_down(&self, key: &str) -> Result<(), String>;
    fn key_up(&self, key: &str) -> Result<(), String>;
    fn mouse_down(&self, button: &str) -> Result<(), String>;
    fn mouse_up(&self, button: &str) -> Result<(), String>;
    fn move_relative(&self, dx: i32, dy: i32) -> Result<(), String>;
    fn wait(&self, duration: Duration);
}

impl AntiAfkInput for InputController {
    fn key_down(&self, key: &str) -> Result<(), String> {
        self.try_key_down(key).map_err(|error| error.to_string())
    }

    fn key_up(&self, key: &str) -> Result<(), String> {
        self.try_key_up(key).map_err(|error| error.to_string())
    }

    fn mouse_down(&self, button: &str) -> Result<(), String> {
        self.try_mouse_down(None, button)
            .map_err(|error| error.to_string())
    }

    fn mouse_up(&self, button: &str) -> Result<(), String> {
        self.try_mouse_up(None, button)
            .map_err(|error| error.to_string())
    }

    fn move_relative(&self, dx: i32, dy: i32) -> Result<(), String> {
        self.try_move_relative(dx, dy)
            .map_err(|error| error.to_string())
    }

    fn wait(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

fn perform_input_action(
    input: &dyn AntiAfkInput,
    action: AntiAfkAction,
    walk_right: &AtomicBool,
) -> Result<(), String> {
    match action {
        AntiAfkAction::Jump => hold_key(input, "space", JUMP_HOLD),
        AntiAfkAction::Walk => {
            let right = walk_right.fetch_xor(true, Ordering::Relaxed);
            hold_key(input, if right { "d" } else { "a" }, WALK_HOLD)
        }
        AntiAfkAction::Camera => camera_nudge(input),
        AntiAfkAction::Random => Err("random action was not resolved".to_string()),
    }
}

fn hold_key(input: &dyn AntiAfkInput, key: &str, duration: Duration) -> Result<(), String> {
    // Clear any stale key state left by a previous interrupted cycle before
    // sending the next press. Some games ignore a keydown while they still
    // believe the key is held, which otherwise makes every other cycle flaky.
    let _ = release_with_retry(|| input.key_up(key), input);
    input.wait(KEY_SETTLE_DELAY);
    if let Err(error) = input.key_down(key) {
        let _ = release_with_retry(|| input.key_up(key), input);
        return Err(error);
    }
    input.wait(duration);
    release_with_retry(|| input.key_up(key), input)
}

fn camera_nudge(input: &dyn AntiAfkInput) -> Result<(), String> {
    input.mouse_down("right")?;
    input.wait(CAMERA_BUTTON_SETTLE);
    let movement = input
        .move_relative(CAMERA_DELTA, 0)
        .map(|()| input.wait(CAMERA_TRAVEL_SETTLE))
        .and_then(|()| input.move_relative(-CAMERA_DELTA, 0));
    let release = release_with_retry(|| input.mouse_up("right"), input);
    movement.and(release)
}

fn release_with_retry<F>(mut release: F, input: &dyn AntiAfkInput) -> Result<(), String>
where
    F: FnMut() -> Result<(), String>,
{
    let mut last_error = None;
    for attempt in 0..INPUT_RELEASE_ATTEMPTS {
        match release() {
            Ok(()) => return Ok(()),
            Err(error) => {
                last_error = Some(error);
                if attempt + 1 < INPUT_RELEASE_ATTEMPTS {
                    input.wait(INPUT_RELEASE_RETRY_DELAY);
                }
            }
        }
    }
    Err(last_error.unwrap_or_else(|| "input release failed".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_hit_testing_uses_half_open_window_bounds() {
        let rect = RECT {
            left: 100,
            top: 200,
            right: 300,
            bottom: 400,
        };
        assert!(point_in_rect(100, 200, rect));
        assert!(point_in_rect(299, 399, rect));
        assert!(!point_in_rect(300, 399, rect));
        assert!(!point_in_rect(299, 400, rect));
    }
    use std::sync::{atomic::AtomicUsize, Mutex};

    #[derive(Default)]
    struct MockInput {
        events: Mutex<Vec<String>>,
        key_up_failures: AtomicUsize,
        mouse_up_failures: AtomicUsize,
    }

    impl MockInput {
        fn take(&self) -> Vec<String> {
            std::mem::take(&mut *self.events.lock().unwrap())
        }
    }

    impl AntiAfkInput for MockInput {
        fn key_down(&self, key: &str) -> Result<(), String> {
            self.events.lock().unwrap().push(format!("down:{key}"));
            Ok(())
        }

        fn key_up(&self, key: &str) -> Result<(), String> {
            self.events.lock().unwrap().push(format!("up:{key}"));
            if self
                .key_up_failures
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |remaining| {
                    if remaining > 0 {
                        Some(remaining - 1)
                    } else {
                        None
                    }
                })
                .is_ok()
            {
                return Err("simulated key release failure".to_string());
            }
            Ok(())
        }

        fn mouse_down(&self, button: &str) -> Result<(), String> {
            self.events
                .lock()
                .unwrap()
                .push(format!("mouse_down:{button}"));
            Ok(())
        }

        fn mouse_up(&self, button: &str) -> Result<(), String> {
            self.events
                .lock()
                .unwrap()
                .push(format!("mouse_up:{button}"));
            if self
                .mouse_up_failures
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |remaining| {
                    if remaining > 0 {
                        Some(remaining - 1)
                    } else {
                        None
                    }
                })
                .is_ok()
            {
                return Err("simulated mouse release failure".to_string());
            }
            Ok(())
        }

        fn move_relative(&self, dx: i32, dy: i32) -> Result<(), String> {
            self.events.lock().unwrap().push(format!("move:{dx}:{dy}"));
            Ok(())
        }

        fn wait(&self, duration: Duration) {
            self.events
                .lock()
                .unwrap()
                .push(format!("wait:{}", duration.as_millis()));
        }
    }

    #[test]
    fn window_ids_round_trip_without_losing_pointer_bits() {
        let hwnd = 0x1234_ABCDusize as HWND;
        assert_eq!(id_to_hwnd(&hwnd_to_id(hwnd)), Some(hwnd));
    }

    #[test]
    fn selectable_window_ids_bind_the_handle_to_its_process() {
        let hwnd = 0x1234_ABCDusize as HWND;
        let id = selectable_window_id(hwnd, 4242);

        assert_eq!(id, "1234ABCD:4242");
        assert_eq!(id_to_hwnd(&id), Some(hwnd));
        assert_eq!(id_process_id(&id), Some(4242));
    }

    #[test]
    fn selection_excludes_hidden_owned_untitled_and_own_process_windows() {
        assert!(is_selectable(true, false, "Game", 22, 11));
        assert!(!is_selectable(false, false, "Game", 22, 11));
        assert!(!is_selectable(true, true, "Dialog", 22, 11));
        assert!(!is_selectable(true, false, "   ", 22, 11));
        assert!(!is_selectable(true, false, "Clawmation", 11, 11));
    }

    #[test]
    fn jump_is_held_long_enough_for_games_to_register_it() {
        let input = MockInput::default();

        perform_input_action(&input, AntiAfkAction::Jump, &AtomicBool::new(true)).unwrap();

        assert_eq!(
            input.take(),
            ["up:space", "wait:35", "down:space", "wait:140", "up:space"]
        );
    }

    #[test]
    fn walk_alternates_directions_to_limit_drift() {
        let input = MockInput::default();
        let right = AtomicBool::new(true);

        perform_input_action(&input, AntiAfkAction::Walk, &right).unwrap();
        perform_input_action(&input, AntiAfkAction::Walk, &right).unwrap();

        assert_eq!(
            input.take(),
            [
                "up:d", "wait:35", "down:d", "wait:180", "up:d", "up:a", "wait:35", "down:a",
                "wait:180", "up:a"
            ]
        );
    }

    #[test]
    fn camera_nudge_returns_and_releases_right_mouse() {
        let input = MockInput::default();

        perform_input_action(&input, AntiAfkAction::Camera, &AtomicBool::new(true)).unwrap();

        assert_eq!(
            input.take(),
            [
                "mouse_down:right",
                "wait:40",
                "move:24:0",
                "wait:60",
                "move:-24:0",
                "mouse_up:right"
            ]
        );
    }

    #[test]
    fn key_release_is_retried_after_transient_input_errors() {
        let input = MockInput {
            key_up_failures: AtomicUsize::new(2),
            ..Default::default()
        };

        perform_input_action(&input, AntiAfkAction::Jump, &AtomicBool::new(true)).unwrap();

        assert_eq!(
            input.take(),
            [
                "up:space",
                "wait:20",
                "up:space",
                "wait:20",
                "up:space",
                "wait:35",
                "down:space",
                "wait:140",
                "up:space"
            ]
        );
    }

    #[test]
    fn camera_release_is_retried_after_transient_input_errors() {
        let input = MockInput {
            mouse_up_failures: AtomicUsize::new(1),
            ..Default::default()
        };

        perform_input_action(&input, AntiAfkAction::Camera, &AtomicBool::new(true)).unwrap();

        assert_eq!(
            input.take(),
            [
                "mouse_down:right",
                "wait:40",
                "move:24:0",
                "wait:60",
                "move:-24:0",
                "mouse_up:right",
                "wait:20",
                "mouse_up:right"
            ]
        );
    }
}
