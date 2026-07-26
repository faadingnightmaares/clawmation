//! Visible-window discovery and foreground switching for Anti-AFK.

use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM};
use windows_sys::Win32::System::Threading::{
    AttachThreadInput, GetCurrentProcessId, GetCurrentThreadId,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, EnumWindows, GetForegroundWindow, GetWindow, GetWindowTextLengthW,
    GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible,
    SetForegroundWindow, ShowWindow, GW_OWNER, SW_RESTORE,
};

use crate::engine::anti_afk::{AntiAfkAction, AntiAfkPlatform};
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
            id: hwnd_to_id(hwnd),
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

fn id_to_hwnd(id: &str) -> Option<HWND> {
    usize::from_str_radix(id, 16)
        .ok()
        .map(|value| value as HWND)
}

pub fn is_window_id(id: &str) -> bool {
    id_to_hwnd(id).is_some_and(|hwnd| unsafe { IsWindow(hwnd) != 0 })
}

pub fn foreground_window_id() -> Option<String> {
    let hwnd = unsafe { GetForegroundWindow() };
    (!hwnd.is_null()).then(|| hwnd_to_id(hwnd))
}

pub fn focus_window_id(id: &str) -> Result<(), String> {
    let hwnd = id_to_hwnd(id).ok_or_else(|| "invalid window id".to_string())?;
    if unsafe { IsWindow(hwnd) } == 0 {
        return Err("window is no longer available".to_string());
    }

    let focused = unsafe {
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

    if !focused {
        return Err("Windows refused to focus the selected window".to_string());
    }
    Ok(())
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
        std::thread::sleep(Duration::from_millis(180));
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
        AntiAfkAction::Jump => hold_key(input, "space", Duration::from_millis(140)),
        AntiAfkAction::Walk => {
            let right = walk_right.fetch_xor(true, Ordering::Relaxed);
            hold_key(
                input,
                if right { "d" } else { "a" },
                Duration::from_millis(180),
            )
        }
        AntiAfkAction::Camera => camera_nudge(input),
        AntiAfkAction::Random => Err("random action was not resolved".to_string()),
    }
}

fn hold_key(input: &dyn AntiAfkInput, key: &str, duration: Duration) -> Result<(), String> {
    input.key_down(key)?;
    input.wait(duration);
    input.key_up(key)
}

fn camera_nudge(input: &dyn AntiAfkInput) -> Result<(), String> {
    input.mouse_down("right")?;
    input.wait(Duration::from_millis(40));
    let movement = input
        .move_relative(24, 0)
        .map(|()| input.wait(Duration::from_millis(60)))
        .and_then(|()| input.move_relative(-24, 0));
    let release = input.mouse_up("right");
    movement.and(release)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct MockInput {
        events: Mutex<Vec<String>>,
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

        assert_eq!(input.take(), ["down:space", "wait:140", "up:space"]);
    }

    #[test]
    fn walk_alternates_directions_to_limit_drift() {
        let input = MockInput::default();
        let right = AtomicBool::new(true);

        perform_input_action(&input, AntiAfkAction::Walk, &right).unwrap();
        perform_input_action(&input, AntiAfkAction::Walk, &right).unwrap();

        assert_eq!(
            input.take(),
            ["down:d", "wait:180", "up:d", "down:a", "wait:180", "up:a"]
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
}
