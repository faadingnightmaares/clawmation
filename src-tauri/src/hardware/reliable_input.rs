//! Serialized, frame-safe input transactions for autonomous Vision actions.
//!
//! Recorded playback owns its original timing and continues to use
//! [`super::input::InputController`] directly. Watch and Loops use this layer:
//! it establishes the intended external window, verifies focus and cursor
//! placement, then keeps each press visible across game frames.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::input::InputController;
use super::window::{self, SelectableWindow};

const PREPARE_ATTEMPTS: usize = 3;
const FOCUS_SETTLE: Duration = Duration::from_millis(80);
const POINTER_ARM_FRAME: Duration = Duration::from_millis(16);
const HOVER_SETTLE: Duration = Duration::from_millis(50);
const PRESS_HOLD: Duration = Duration::from_millis(80);
const PREPARE_RETRY_DELAY: Duration = Duration::from_millis(40);
const CURSOR_TOLERANCE: i32 = 1;
const POINTER_ARM_DELTA: i32 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReliableTarget {
    window: SelectableWindow,
    anchor: (i32, i32),
}

impl ReliableTarget {
    pub fn id(&self) -> &str {
        &self.window.id
    }

    fn display_name(&self) -> &str {
        let title = self.window.title.trim();
        if title.is_empty() {
            &self.window.id
        } else {
            title
        }
    }
}

trait Platform: Send + Sync {
    fn target_at(&self, x: i32, y: i32) -> Result<SelectableWindow, String>;
    fn focus(&self, target: &SelectableWindow) -> Result<(), String>;
    fn is_foreground(&self, target: &SelectableWindow) -> bool;
    fn move_to(&self, x: i32, y: i32) -> Result<(), String>;
    fn move_relative_no_coalesce(&self, dx: i32, dy: i32) -> Result<(), String>;
    fn sync_cursor_to(&self, x: i32, y: i32) -> Result<(), String>;
    fn cursor_position(&self) -> Result<(i32, i32), String>;
    fn mouse_down(&self) -> Result<(), String>;
    fn mouse_up(&self) -> Result<(), String>;
    fn key_down(&self, key: &str) -> Result<(), String>;
    fn key_up(&self, key: &str) -> Result<(), String>;
    fn nudge(&self) -> Result<(), String>;
    fn sleep(&self, duration: Duration);
}

struct NativePlatform {
    controller: Arc<InputController>,
}

impl Platform for NativePlatform {
    fn target_at(&self, x: i32, y: i32) -> Result<SelectableWindow, String> {
        window::window_at_point(x, y)?
            .ok_or_else(|| format!("no external window exists beneath ({x}, {y})"))
    }

    fn focus(&self, target: &SelectableWindow) -> Result<(), String> {
        window::focus_selectable_window(target)
    }

    fn is_foreground(&self, target: &SelectableWindow) -> bool {
        window::foreground_window_id().as_deref() == Some(target.id.as_str())
    }

    fn move_to(&self, x: i32, y: i32) -> Result<(), String> {
        self.controller
            .try_move_to(x, y)
            .map_err(|error| error.to_string())
    }

    fn move_relative_no_coalesce(&self, dx: i32, dy: i32) -> Result<(), String> {
        self.controller
            .try_move_relative_no_coalesce(dx, dy)
            .map_err(|error| error.to_string())
    }

    fn sync_cursor_to(&self, x: i32, y: i32) -> Result<(), String> {
        self.controller
            .try_sync_cursor_to(x, y)
            .map_err(|error| error.to_string())
    }

    fn cursor_position(&self) -> Result<(i32, i32), String> {
        self.controller
            .try_cursor_position()
            .map_err(|error| error.to_string())
    }

    fn mouse_down(&self) -> Result<(), String> {
        self.controller
            .try_mouse_down(None, "left")
            .map_err(|error| error.to_string())
    }

    fn mouse_up(&self) -> Result<(), String> {
        self.controller
            .try_mouse_up(None, "left")
            .map_err(|error| error.to_string())
    }

    fn key_down(&self, key: &str) -> Result<(), String> {
        self.controller
            .try_key_down(key)
            .map_err(|error| error.to_string())
    }

    fn key_up(&self, key: &str) -> Result<(), String> {
        self.controller
            .try_key_up(key)
            .map_err(|error| error.to_string())
    }

    fn nudge(&self) -> Result<(), String> {
        self.controller
            .try_nudge()
            .map_err(|error| error.to_string())
    }

    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

/// One process-wide autonomous actuator. Its lock covers the whole transaction,
/// preventing two Watch/Loop detections from interleaving down/up edges.
pub struct ReliableInput {
    platform: Arc<dyn Platform>,
    transaction: Mutex<()>,
}

impl ReliableInput {
    pub fn new(controller: Arc<InputController>) -> Self {
        Self {
            platform: Arc::new(NativePlatform { controller }),
            transaction: Mutex::new(()),
        }
    }

    pub fn click_at(&self, x: i32, y: i32) -> Result<ReliableTarget, String> {
        let _transaction = self.lock_transaction()?;
        let target = self.resolve_target(x, y)?;
        self.prepare_pointer(&target, x, y)?;
        self.platform
            .mouse_down()
            .map_err(|error| phase("mouse press", &target, error))?;
        self.platform.sleep(PRESS_HOLD);
        let kept_focus = self.platform.is_foreground(&target.window);
        let release = self.release_mouse(&target);
        if !kept_focus {
            // The global release clears physical state. Re-focus and send one
            // more release so a target that captured the press cannot remain
            // logically held after another window stole foreground.
            let _ = self.platform.focus(&target.window);
            let _ = self.platform.mouse_up();
            return Err(phase(
                "mouse gesture",
                &target,
                "target lost foreground while the button was held".to_string(),
            ));
        }
        release?;
        Ok(target)
    }

    pub fn key_at(&self, x: i32, y: i32, key: &str) -> Result<ReliableTarget, String> {
        let _transaction = self.lock_transaction()?;
        let target = self.resolve_target(x, y)?;
        self.prepare_pointer(&target, x, y)?;
        self.press_key(&target, key)?;
        Ok(target)
    }

    pub fn nudge_at(&self, x: i32, y: i32) -> Result<ReliableTarget, String> {
        let _transaction = self.lock_transaction()?;
        let target = self.resolve_target(x, y)?;
        self.prepare_pointer(&target, x, y)?;
        self.platform
            .nudge()
            .map_err(|error| phase("mouse nudge", &target, error))?;
        Ok(target)
    }

    /// Establish a target for a later key-only Loop node without injecting an
    /// input event. Used when a wait-for Vision node succeeds.
    pub fn establish_at(&self, x: i32, y: i32) -> Result<ReliableTarget, String> {
        let _transaction = self.lock_transaction()?;
        let target = self.resolve_target(x, y)?;
        self.prepare_pointer(&target, x, y)?;
        Ok(target)
    }

    pub fn key_on(&self, target: &ReliableTarget, key: &str) -> Result<(), String> {
        let _transaction = self.lock_transaction()?;
        self.prepare_pointer(target, target.anchor.0, target.anchor.1)?;
        self.press_key(target, key)
    }

    fn lock_transaction(&self) -> Result<std::sync::MutexGuard<'_, ()>, String> {
        self.transaction
            .lock()
            .map_err(|_| "autonomous input transaction lock is poisoned".to_string())
    }

    fn resolve_target(&self, x: i32, y: i32) -> Result<ReliableTarget, String> {
        self.platform
            .target_at(x, y)
            .map(|window| ReliableTarget {
                window,
                anchor: (x, y),
            })
            .map_err(|error| format!("target resolution failed: {error}"))
    }

    fn prepare_pointer(&self, target: &ReliableTarget, x: i32, y: i32) -> Result<(), String> {
        let mut last_error = String::new();
        for attempt in 1..=PREPARE_ATTEMPTS {
            match self.prepare_pointer_once(target, x, y) {
                Ok(()) => return Ok(()),
                Err(error) => last_error = error,
            }
            if attempt < PREPARE_ATTEMPTS {
                self.platform.sleep(PREPARE_RETRY_DELAY);
            }
        }
        Err(phase("cursor preparation", target, last_error))
    }

    fn prepare_pointer_once(&self, target: &ReliableTarget, x: i32, y: i32) -> Result<(), String> {
        self.platform.focus(&target.window)?;
        self.platform.sleep(FOCUS_SETTLE);
        if !self.platform.is_foreground(&target.window) {
            return Err("Windows reported a different foreground window".to_string());
        }
        self.platform.move_to(x, y)?;
        self.platform
            .move_relative_no_coalesce(POINTER_ARM_DELTA, 0)?;
        self.platform.sleep(POINTER_ARM_FRAME);
        self.platform
            .move_relative_no_coalesce(-POINTER_ARM_DELTA, 0)?;
        self.platform.sync_cursor_to(x, y)?;
        self.platform.sleep(HOVER_SETTLE);
        if !self.platform.is_foreground(&target.window) {
            return Err("target lost foreground while positioning the cursor".to_string());
        }
        let (actual_x, actual_y) = self.platform.cursor_position()?;
        if (actual_x - x).abs() > CURSOR_TOLERANCE || (actual_y - y).abs() > CURSOR_TOLERANCE {
            return Err(format!(
                "cursor landed at ({actual_x}, {actual_y}) instead of ({x}, {y})"
            ));
        }
        Ok(())
    }

    fn release_mouse(&self, target: &ReliableTarget) -> Result<(), String> {
        match self.platform.mouse_up() {
            Ok(()) => Ok(()),
            Err(error) => {
                let _ = self.platform.mouse_up();
                Err(phase("mouse release", target, error))
            }
        }
    }

    fn press_key(&self, target: &ReliableTarget, key: &str) -> Result<(), String> {
        let key = key.trim();
        if key.is_empty() {
            return Err("key press failed: no key is configured".to_string());
        }
        self.platform
            .key_down(key)
            .map_err(|error| phase("key press", target, error))?;
        self.platform.sleep(PRESS_HOLD);
        let kept_focus = self.platform.is_foreground(&target.window);
        let release = match self.platform.key_up(key) {
            Ok(()) => Ok(()),
            Err(error) => {
                let _ = self.platform.key_up(key);
                Err(phase("key release", target, error))
            }
        };
        if !kept_focus {
            let _ = self.platform.focus(&target.window);
            let _ = self.platform.key_up(key);
            return Err(phase(
                "key gesture",
                target,
                "target lost foreground while the key was held".to_string(),
            ));
        }
        release
    }

    #[cfg(test)]
    fn with_platform(platform: Arc<dyn Platform>) -> Self {
        Self {
            platform,
            transaction: Mutex::new(()),
        }
    }
}

fn phase(phase: &str, target: &ReliableTarget, detail: String) -> String {
    format!("{phase} failed for {:?}: {detail}", target.display_name())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    #[derive(Default)]
    struct MockState {
        foreground: bool,
        cursor: (i32, i32),
        relative_motion_since_absolute: bool,
        log: Vec<String>,
    }

    struct MockPlatform {
        state: Mutex<MockState>,
        focus_failures: AtomicUsize,
        cursor_drifts: AtomicUsize,
        mouse_up_failures: AtomicUsize,
        press_held: AtomicBool,
        lose_focus_on_hold: AtomicBool,
    }

    impl MockPlatform {
        fn new() -> Self {
            Self {
                state: Mutex::new(MockState::default()),
                focus_failures: AtomicUsize::new(0),
                cursor_drifts: AtomicUsize::new(0),
                mouse_up_failures: AtomicUsize::new(0),
                press_held: AtomicBool::new(false),
                lose_focus_on_hold: AtomicBool::new(false),
            }
        }

        fn log(&self) -> Vec<String> {
            self.state.lock().unwrap().log.clone()
        }
    }

    impl Platform for MockPlatform {
        fn target_at(&self, x: i32, y: i32) -> Result<SelectableWindow, String> {
            self.state
                .lock()
                .unwrap()
                .log
                .push(format!("target:{x},{y}"));
            Ok(SelectableWindow {
                id: "ABCD:42".to_string(),
                title: "Roblox".to_string(),
                pid: 42,
            })
        }

        fn focus(&self, _target: &SelectableWindow) -> Result<(), String> {
            self.state.lock().unwrap().log.push("focus".to_string());
            if self
                .focus_failures
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok()
            {
                return Err("focus refused".to_string());
            }
            self.state.lock().unwrap().foreground = true;
            Ok(())
        }

        fn is_foreground(&self, _target: &SelectableWindow) -> bool {
            self.state.lock().unwrap().foreground
        }

        fn move_to(&self, x: i32, y: i32) -> Result<(), String> {
            let mut state = self.state.lock().unwrap();
            state.cursor = (x, y);
            state.relative_motion_since_absolute = false;
            state.log.push(format!("move:{x},{y}"));
            Ok(())
        }

        fn move_relative_no_coalesce(&self, dx: i32, dy: i32) -> Result<(), String> {
            let mut state = self.state.lock().unwrap();
            state.cursor.0 += dx;
            state.cursor.1 += dy;
            state.relative_motion_since_absolute = true;
            state.log.push(format!("relative:{dx},{dy}"));
            Ok(())
        }

        fn sync_cursor_to(&self, x: i32, y: i32) -> Result<(), String> {
            let mut state = self.state.lock().unwrap();
            state.cursor = (x, y);
            state.log.push(format!("sync:{x},{y}"));
            Ok(())
        }

        fn cursor_position(&self) -> Result<(i32, i32), String> {
            if self
                .cursor_drifts
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok()
            {
                let (x, y) = self.state.lock().unwrap().cursor;
                return Ok((x + 20, y + 20));
            }
            Ok(self.state.lock().unwrap().cursor)
        }

        fn mouse_down(&self) -> Result<(), String> {
            let mut state = self.state.lock().unwrap();
            if !state.relative_motion_since_absolute {
                return Err(
                    "Roblox ignored the click because no relative motion was observed".to_string(),
                );
            }
            self.press_held.store(true, Ordering::SeqCst);
            let (x, y) = state.cursor;
            state.log.push(format!("down:{x},{y}"));
            Ok(())
        }

        fn mouse_up(&self) -> Result<(), String> {
            self.press_held.store(false, Ordering::SeqCst);
            let mut state = self.state.lock().unwrap();
            let (x, y) = state.cursor;
            state.log.push(format!("up:{x},{y}"));
            if self
                .mouse_up_failures
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok()
            {
                Err("release rejected".to_string())
            } else {
                Ok(())
            }
        }

        fn key_down(&self, key: &str) -> Result<(), String> {
            self.press_held.store(true, Ordering::SeqCst);
            self.state
                .lock()
                .unwrap()
                .log
                .push(format!("key-down:{key}"));
            Ok(())
        }

        fn key_up(&self, key: &str) -> Result<(), String> {
            self.press_held.store(false, Ordering::SeqCst);
            self.state.lock().unwrap().log.push(format!("key-up:{key}"));
            Ok(())
        }

        fn nudge(&self) -> Result<(), String> {
            self.state.lock().unwrap().log.push("nudge".to_string());
            Ok(())
        }

        fn sleep(&self, duration: Duration) {
            if duration == PRESS_HOLD
                && self.press_held.load(Ordering::SeqCst)
                && self.lose_focus_on_hold.swap(false, Ordering::SeqCst)
            {
                self.state.lock().unwrap().foreground = false;
            }
            self.state
                .lock()
                .unwrap()
                .log
                .push(format!("sleep:{}", duration.as_millis()));
        }
    }

    #[test]
    fn click_arms_raw_input_then_establishes_hover_and_frame_spanning_press() {
        let platform = Arc::new(MockPlatform::new());
        let input = ReliableInput::with_platform(platform.clone());
        let target = input.click_at(120, 240).unwrap();
        assert_eq!(target.id(), "ABCD:42");
        assert_eq!(
            platform.log(),
            vec![
                "target:120,240",
                "focus",
                "sleep:80",
                "move:120,240",
                "relative:2,0",
                "sleep:16",
                "relative:-2,0",
                "sync:120,240",
                "sleep:50",
                "down:120,240",
                "sleep:80",
                "up:120,240",
            ]
        );
    }

    #[test]
    fn repeated_clicks_keep_complete_gestures_ordered() {
        let platform = Arc::new(MockPlatform::new());
        let input = ReliableInput::with_platform(platform.clone());
        for index in 0..500 {
            input.click_at(index, 9).unwrap();
        }
        let edges: Vec<String> = platform
            .log()
            .into_iter()
            .filter(|entry| entry.starts_with("down:") || entry.starts_with("up:"))
            .collect();
        assert_eq!(edges.len(), 1000);
        for (index, pair) in edges.chunks_exact(2).enumerate() {
            assert_eq!(pair[0], format!("down:{index},9"));
            assert_eq!(pair[1], format!("up:{index},9"));
        }
    }

    #[test]
    fn concurrent_actions_cannot_interleave_button_edges() {
        let platform = Arc::new(MockPlatform::new());
        let input = Arc::new(ReliableInput::with_platform(platform.clone()));
        let workers: Vec<_> = (0..32)
            .map(|index| {
                let input = input.clone();
                std::thread::spawn(move || input.click_at(index, 7).unwrap())
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }

        let edges: Vec<String> = platform
            .log()
            .into_iter()
            .filter(|entry| entry.starts_with("down:") || entry.starts_with("up:"))
            .collect();
        assert_eq!(edges.len(), 64);
        for pair in edges.chunks_exact(2) {
            assert_eq!(
                pair[0].strip_prefix("down:"),
                pair[1].strip_prefix("up:"),
                "a second transaction interleaved between down and up"
            );
        }
    }

    #[test]
    fn focus_failure_retries_before_any_press() {
        let platform = Arc::new(MockPlatform::new());
        platform.focus_failures.store(3, Ordering::SeqCst);
        let input = ReliableInput::with_platform(platform.clone());
        let error = input.click_at(10, 20).unwrap_err();
        assert!(error.contains("focus refused"));
        assert_eq!(
            platform
                .log()
                .iter()
                .filter(|entry| *entry == "focus")
                .count(),
            PREPARE_ATTEMPTS
        );
        assert!(!platform
            .log()
            .iter()
            .any(|entry| entry.starts_with("down:")));
    }

    #[test]
    fn cursor_drift_is_corrected_before_pressing() {
        let platform = Arc::new(MockPlatform::new());
        platform.cursor_drifts.store(1, Ordering::SeqCst);
        let input = ReliableInput::with_platform(platform.clone());
        input.click_at(10, 20).unwrap();
        assert_eq!(
            platform
                .log()
                .iter()
                .filter(|entry| *entry == "move:10,20")
                .count(),
            2
        );
        assert_eq!(
            platform
                .log()
                .iter()
                .filter(|entry| entry.starts_with("down:"))
                .count(),
            1
        );
    }

    #[test]
    fn failed_release_gets_recovery_attempt_and_surfaces_error() {
        let platform = Arc::new(MockPlatform::new());
        platform.mouse_up_failures.store(2, Ordering::SeqCst);
        let input = ReliableInput::with_platform(platform.clone());
        let error = input.click_at(10, 20).unwrap_err();
        assert!(error.contains("mouse release"));
        assert_eq!(
            platform
                .log()
                .iter()
                .filter(|entry| entry.starts_with("up:"))
                .count(),
            2
        );
    }

    #[test]
    fn focus_loss_during_a_press_releases_recovers_and_reports_failure() {
        let platform = Arc::new(MockPlatform::new());
        platform.lose_focus_on_hold.store(true, Ordering::SeqCst);
        let input = ReliableInput::with_platform(platform.clone());
        let error = input.click_at(10, 20).unwrap_err();
        assert!(error.contains("lost foreground"));
        let log = platform.log();
        assert_eq!(
            log.iter().filter(|entry| entry.starts_with("up:")).count(),
            2,
            "global release plus a release after re-focusing the target"
        );
    }

    #[test]
    fn key_uses_the_same_target_and_frame_hold() {
        let platform = Arc::new(MockPlatform::new());
        let input = ReliableInput::with_platform(platform.clone());
        let target = input.establish_at(10, 20).unwrap();
        input.key_on(&target, "e").unwrap();
        let log = platform.log();
        let down = log.iter().position(|entry| entry == "key-down:e").unwrap();
        assert_eq!(log[down + 1], "sleep:80");
        assert_eq!(log[down + 2], "key-up:e");
    }
}
