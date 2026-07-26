//! Session-scoped Anti-AFK worker.
//!
//! The engine owns timing and validation while [`AntiAfkPlatform`] keeps Win32
//! focus and input details outside this module. That boundary makes the
//! focus-jump-restore contract deterministic to test without moving a real
//! window.

use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

pub const MIN_INTERVAL_MIN: u32 = 1;
pub const MAX_INTERVAL_MIN: u32 = 20;
pub const DEFAULT_INTERVAL_MIN: u32 = 15;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AntiAfkAction {
    Jump,
    Walk,
    Camera,
    #[default]
    Random,
}

impl AntiAfkAction {
    pub fn from_config(value: &str) -> Self {
        match value {
            "jump" => Self::Jump,
            "walk" => Self::Walk,
            "camera" => Self::Camera,
            _ => Self::Random,
        }
    }

    pub fn as_config(self) -> &'static str {
        match self {
            Self::Jump => "jump",
            Self::Walk => "walk",
            Self::Camera => "camera",
            Self::Random => "random",
        }
    }
}

pub trait AntiAfkPlatform: Send + Sync + 'static {
    fn is_window(&self, id: &str) -> bool;
    fn foreground_window(&self) -> Option<String>;
    fn focus_window(&self, id: &str) -> Result<(), String>;
    fn perform_action(&self, action: AntiAfkAction) -> Result<(), String>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AntiAfkSnapshot {
    pub enabled: bool,
    pub target_id: Option<String>,
    pub interval_min: u32,
    pub action: AntiAfkAction,
    pub status: String,
    pub error: Option<String>,
}

struct WorkerState {
    enabled: bool,
    target_id: Option<String>,
    interval_min: u32,
    action: AntiAfkAction,
    random_state: u64,
    status: String,
    error: Option<String>,
    next_fire: Option<Instant>,
    shutdown: bool,
}

impl WorkerState {
    fn snapshot(&self) -> AntiAfkSnapshot {
        AntiAfkSnapshot {
            enabled: self.enabled,
            target_id: self.target_id.clone(),
            interval_min: self.interval_min,
            action: self.action,
            status: self.status.clone(),
            error: self.error.clone(),
        }
    }
}

struct Shared {
    state: Mutex<WorkerState>,
    changed: Condvar,
}

pub struct AntiAfkService {
    shared: Arc<Shared>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl AntiAfkService {
    pub fn new(
        platform: Arc<dyn AntiAfkPlatform>,
        interval_min: u32,
        action: AntiAfkAction,
    ) -> Self {
        let interval_min = interval_min.clamp(MIN_INTERVAL_MIN, MAX_INTERVAL_MIN);
        let random_state = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
        let shared = Arc::new(Shared {
            state: Mutex::new(WorkerState {
                enabled: false,
                target_id: None,
                interval_min,
                action,
                random_state,
                status: "off".to_string(),
                error: None,
                next_fire: None,
                shutdown: false,
            }),
            changed: Condvar::new(),
        });
        let worker_shared = shared.clone();
        let worker = std::thread::Builder::new()
            .name("anti-afk".to_string())
            .spawn(move || worker_loop(worker_shared, platform))
            .expect("anti-afk worker thread starts");
        Self {
            shared,
            worker: Mutex::new(Some(worker)),
        }
    }

    pub fn get(&self) -> AntiAfkSnapshot {
        self.shared.state.lock().unwrap().snapshot()
    }

    pub fn update(
        &self,
        target_id: Option<String>,
        interval_min: Option<u32>,
        action: Option<AntiAfkAction>,
        enabled: Option<bool>,
    ) -> Result<AntiAfkSnapshot, String> {
        if let Some(minutes) = interval_min {
            if !(MIN_INTERVAL_MIN..=MAX_INTERVAL_MIN).contains(&minutes) {
                return Err(format!(
                    "interval must be between {MIN_INTERVAL_MIN} and {MAX_INTERVAL_MIN} minutes"
                ));
            }
        }

        let now = Instant::now();
        let mut state = self.shared.state.lock().unwrap();
        if let Some(id) = target_id {
            state.target_id = Some(id);
            state.error = None;
            if state.enabled {
                state.status = "active".to_string();
            }
        }

        let interval_changed = interval_min.is_some_and(|minutes| minutes != state.interval_min);
        if let Some(minutes) = interval_min {
            state.interval_min = minutes;
        }
        if let Some(action) = action {
            state.action = action;
        }

        let newly_enabled = enabled == Some(true) && !state.enabled;
        if enabled == Some(true) && state.target_id.is_none() {
            return Err("select a game or app window first".to_string());
        }
        if let Some(value) = enabled {
            state.enabled = value;
            state.error = None;
            if value {
                state.status = "active".to_string();
            } else {
                state.status = "off".to_string();
                state.next_fire = None;
            }
        }

        if state.enabled {
            if newly_enabled {
                state.next_fire = Some(now);
            } else if interval_changed {
                state.next_fire = Some(now + interval_duration(state.interval_min));
            }
        }

        let snapshot = state.snapshot();
        drop(state);
        self.shared.changed.notify_all();
        Ok(snapshot)
    }
}

impl Drop for AntiAfkService {
    fn drop(&mut self) {
        {
            let mut state = self.shared.state.lock().unwrap();
            state.shutdown = true;
        }
        self.shared.changed.notify_all();
        if let Some(worker) = self.worker.get_mut().unwrap().take() {
            let _ = worker.join();
        }
    }
}

fn interval_duration(minutes: u32) -> Duration {
    Duration::from_secs(u64::from(minutes) * 60)
}

fn worker_loop(shared: Arc<Shared>, platform: Arc<dyn AntiAfkPlatform>) {
    loop {
        let (target, action) = {
            let mut state = shared.state.lock().unwrap();
            loop {
                if state.shutdown {
                    return;
                }
                match (state.enabled, state.target_id.clone(), state.next_fire) {
                    (true, Some(target), Some(due)) if due <= Instant::now() => {
                        state.next_fire = None;
                        state.status = "acting".to_string();
                        let configured = state.action;
                        let action = resolve_action(configured, &mut state.random_state);
                        break (target, action);
                    }
                    (true, Some(_), Some(due)) => {
                        let wait = due.saturating_duration_since(Instant::now());
                        let (next, _) = shared.changed.wait_timeout(state, wait).unwrap();
                        state = next;
                    }
                    _ => state = shared.changed.wait(state).unwrap(),
                }
            }
        };

        let result = perform_anti_afk(platform.as_ref(), &target, action);

        let mut state = shared.state.lock().unwrap();
        if state.enabled {
            state.next_fire = Some(Instant::now() + interval_duration(state.interval_min));
            match result {
                Ok(()) => {
                    state.status = "active".to_string();
                    state.error = None;
                }
                Err(ActionError::TargetUnavailable) => {
                    state.status = "target_unavailable".to_string();
                    state.error = Some("The selected window is no longer available.".to_string());
                }
                Err(ActionError::Operation(error)) => {
                    state.status = "error".to_string();
                    state.error = Some(error);
                }
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum ActionError {
    TargetUnavailable,
    Operation(String),
}

fn resolve_action(configured: AntiAfkAction, state: &mut u64) -> AntiAfkAction {
    if configured != AntiAfkAction::Random {
        return configured;
    }
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    match *state % 3 {
        0 => AntiAfkAction::Jump,
        1 => AntiAfkAction::Walk,
        _ => AntiAfkAction::Camera,
    }
}

fn perform_anti_afk(
    platform: &dyn AntiAfkPlatform,
    target: &str,
    action: AntiAfkAction,
) -> Result<(), ActionError> {
    if !platform.is_window(target) {
        return Err(ActionError::TargetUnavailable);
    }

    let previous = platform.foreground_window();
    let action_result = platform
        .focus_window(target)
        .map_err(ActionError::Operation)
        .and_then(|()| {
            platform
                .perform_action(action)
                .map_err(ActionError::Operation)
        });
    let restore_result = previous
        .filter(|id| id != target && platform.is_window(id))
        .map(|id| platform.focus_window(&id).map_err(ActionError::Operation))
        .unwrap_or(Ok(()));

    action_result.and(restore_result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::{self, Receiver, Sender};

    struct MockPlatform {
        events: Sender<String>,
        valid: bool,
        previous: Option<String>,
        action_error: bool,
        target_focus_error: bool,
    }

    impl AntiAfkPlatform for MockPlatform {
        fn is_window(&self, id: &str) -> bool {
            let _ = self.events.send(format!("valid:{id}"));
            self.valid
        }

        fn foreground_window(&self) -> Option<String> {
            let _ = self.events.send("foreground".to_string());
            self.previous.clone()
        }

        fn focus_window(&self, id: &str) -> Result<(), String> {
            let _ = self.events.send(format!("focus:{id}"));
            if self.target_focus_error && id == "target" {
                Err("focus failed".to_string())
            } else {
                Ok(())
            }
        }

        fn perform_action(&self, action: AntiAfkAction) -> Result<(), String> {
            let _ = self.events.send(format!("action:{}", action.as_config()));
            if self.action_error {
                Err("action failed".to_string())
            } else {
                Ok(())
            }
        }
    }

    fn mock(valid: bool, previous: Option<&str>) -> (Arc<MockPlatform>, Receiver<String>) {
        let (tx, rx) = mpsc::channel();
        (
            Arc::new(MockPlatform {
                events: tx,
                valid,
                previous: previous.map(str::to_string),
                action_error: false,
                target_focus_error: false,
            }),
            rx,
        )
    }

    #[test]
    fn enabling_requires_a_target() {
        let (platform, _) = mock(true, None);
        let service = AntiAfkService::new(platform, DEFAULT_INTERVAL_MIN, AntiAfkAction::Jump);

        let error = service
            .update(None, None, None, Some(true))
            .expect_err("enable without target is rejected");

        assert_eq!(error, "select a game or app window first");
        assert!(!service.get().enabled);
    }

    #[test]
    fn interval_is_validated() {
        let (platform, _) = mock(true, None);
        let service = AntiAfkService::new(platform, DEFAULT_INTERVAL_MIN, AntiAfkAction::Jump);

        assert!(service.update(None, Some(0), None, None).is_err());
        assert!(service.update(None, Some(21), None, None).is_err());
        assert_eq!(
            service
                .update(None, Some(7), None, None)
                .unwrap()
                .interval_min,
            7
        );
    }

    #[test]
    fn jump_focuses_target_presses_space_and_restores_foreground() {
        let (platform, events) = mock(true, Some("previous"));

        perform_anti_afk(platform.as_ref(), "target", AntiAfkAction::Jump).unwrap();

        let actual: Vec<String> = events.try_iter().collect();
        assert_eq!(
            actual,
            [
                "valid:target",
                "foreground",
                "focus:target",
                "action:jump",
                "valid:previous",
                "focus:previous"
            ]
        );
    }

    #[test]
    fn enabling_fires_immediately() {
        let (platform, events) = mock(true, Some("previous"));
        let service = AntiAfkService::new(platform, DEFAULT_INTERVAL_MIN, AntiAfkAction::Walk);

        service
            .update(Some("target".to_string()), None, None, Some(true))
            .unwrap();

        let mut actual = Vec::new();
        while actual.len() < 6 {
            actual.push(
                events
                    .recv_timeout(Duration::from_secs(1))
                    .expect("immediate jump event"),
            );
        }
        assert_eq!(actual[2], "focus:target");
        assert_eq!(actual[3], "action:walk");
        assert_eq!(actual[5], "focus:previous");
    }

    #[test]
    fn jump_failure_still_restores_the_previous_window() {
        let (events_tx, events_rx) = mpsc::channel();
        let platform = MockPlatform {
            events: events_tx,
            valid: true,
            previous: Some("previous".to_string()),
            action_error: true,
            target_focus_error: false,
        };

        assert_eq!(
            perform_anti_afk(&platform, "target", AntiAfkAction::Camera),
            Err(ActionError::Operation("action failed".to_string()))
        );
        assert_eq!(
            events_rx.try_iter().collect::<Vec<_>>().last(),
            Some(&"focus:previous".to_string())
        );
    }

    #[test]
    fn focus_failure_still_attempts_to_restore_the_previous_window() {
        let (events_tx, events_rx) = mpsc::channel();
        let platform = MockPlatform {
            events: events_tx,
            valid: true,
            previous: Some("previous".to_string()),
            action_error: false,
            target_focus_error: true,
        };

        assert_eq!(
            perform_anti_afk(&platform, "target", AntiAfkAction::Jump),
            Err(ActionError::Operation("focus failed".to_string()))
        );
        assert_eq!(
            events_rx.try_iter().collect::<Vec<_>>().last(),
            Some(&"focus:previous".to_string())
        );
    }

    #[test]
    fn missing_target_is_reported_without_input() {
        let (platform, events) = mock(false, None);

        assert_eq!(
            perform_anti_afk(platform.as_ref(), "closed", AntiAfkAction::Jump),
            Err(ActionError::TargetUnavailable)
        );
        assert_eq!(
            events.try_iter().collect::<Vec<_>>(),
            ["valid:closed".to_string()]
        );
    }

    #[test]
    fn random_mix_resolves_to_supported_actions() {
        let mut state = 0xA5A5_1234_8765_DCBA;
        for _ in 0..20 {
            assert!(matches!(
                resolve_action(AntiAfkAction::Random, &mut state),
                AntiAfkAction::Jump | AntiAfkAction::Walk | AntiAfkAction::Camera
            ));
        }
        assert_eq!(
            resolve_action(AntiAfkAction::Walk, &mut state),
            AntiAfkAction::Walk
        );
    }
}
