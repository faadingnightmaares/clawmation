//! Shared semantic verification for autonomous Vision actions.
//!
//! Hardware-specific input and detection stay injected. This module owns the
//! one rule Watch, Loops, and guards must agree on: an OS-accepted gesture is
//! retried when a later screen observation still contains the same target.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock, TryLockError};
use std::time::{Duration, Instant};

use super::sleep_interruptible;

pub const MAX_ACTION_ATTEMPTS: usize = 3;
pub const RETRY_GAP: Duration = Duration::from_millis(16);
pub const FRESH_FRAME_FALLBACK_AFTER: Duration = Duration::from_millis(80);
pub const MIN_REACTION_SETTLE: Duration = Duration::from_millis(16);
const ACTION_SESSION_POLL: Duration = Duration::from_millis(2);

static ACTION_SESSION: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameStamp {
    pub sample: u64,
    pub generation: u64,
    pub fresh: bool,
    pub captured_at: Instant,
}

/// Result of looking at the target after a delivered action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetReaction {
    Changed,
    StillVisible,
    Unavailable(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActionReceipt {
    pub attempts: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetBox {
    pub x: i64,
    pub y: i64,
    pub w: i64,
    pub h: i64,
}

impl TargetBox {
    pub fn new(x: i64, y: i64, w: i64, h: i64) -> Self {
        Self {
            x,
            y,
            w: w.max(0),
            h: h.max(0),
        }
    }

    /// Hover candidates can change colour and size by a few pixels. Treat them
    /// as the same actionable control when their centres remain inside the
    /// larger target footprint; unrelated identical buttons elsewhere do not
    /// block acceptance.
    pub fn matches(self, candidate: Self) -> bool {
        let tolerance_x = self.w.max(candidate.w).max(16) / 2;
        let tolerance_y = self.h.max(candidate.h).max(16) / 2;
        (self.x - candidate.x).abs() <= tolerance_x && (self.y - candidate.y).abs() <= tolerance_y
    }
}

/// A sample is usable for semantic verification when it was taken after the
/// action and contains a newly sampled desktop frame. A DXGI cache reuse cannot
/// validate or trigger another press; production forces one GDI sample after a
/// short settle when the desktop has not presented.
pub fn observation_is_settled(
    baseline: FrameStamp,
    observed: FrameStamp,
    elapsed_since_action: Duration,
) -> bool {
    if observed.sample <= baseline.sample {
        return false;
    }
    elapsed_since_action >= MIN_REACTION_SETTLE
        && observed.fresh
        && observed.generation > baseline.generation
        && observed.captured_at > baseline.captured_at
}

/// Serialize a delivery-only autonomous gesture with verified actions. This is
/// used by nudges and drags, whose whole down/move/up sequence must not land
/// inside another action's delivery-to-verification window.
pub fn execute_serialized<T>(
    running: &AtomicBool,
    perform: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let _session = lock_action_session(running)?;
    if !running.load(Ordering::SeqCst) {
        return Err("action cancelled before input delivery".to_string());
    }
    perform()
}

fn lock_action_session(running: &AtomicBool) -> Result<std::sync::MutexGuard<'static, ()>, String> {
    let session = ACTION_SESSION.get_or_init(|| Mutex::new(()));
    loop {
        if !running.load(Ordering::SeqCst) {
            return Err("action cancelled while waiting for input session".to_string());
        }
        match session.try_lock() {
            Ok(guard) => return Ok(guard),
            Err(TryLockError::Poisoned(error)) => return Ok(error.into_inner()),
            Err(TryLockError::WouldBlock) => {
                sleep_interruptible(running, ACTION_SESSION_POLL);
            }
        }
    }
}

/// Deliver and verify one autonomous action. A still-visible target causes a
/// complete new gesture, never a duplicated down edge. Cancellation prevents a
/// new attempt but release recovery remains the actuator's responsibility.
pub fn execute_verified(
    running: &AtomicBool,
    mut perform: impl FnMut() -> Result<(), String>,
    mut verify: impl FnMut(&AtomicBool) -> Result<TargetReaction, String>,
) -> Result<ActionReceipt, String> {
    let _session = lock_action_session(running)?;
    for attempt in 1..=MAX_ACTION_ATTEMPTS {
        if !running.load(Ordering::SeqCst) {
            return Err("action cancelled before input delivery".to_string());
        }
        if let Err(error) = perform() {
            return Err(if attempt == 1 {
                error
            } else {
                format!("delivery attempt {attempt} failed: {error}")
            });
        }
        if !running.load(Ordering::SeqCst) {
            return Err("action cancelled after input delivery".to_string());
        }
        let reaction = verify(running)?;
        if !running.load(Ordering::SeqCst) {
            return Err("action cancelled during target verification".to_string());
        }
        match reaction {
            TargetReaction::Changed => return Ok(ActionReceipt { attempts: attempt }),
            TargetReaction::StillVisible if attempt < MAX_ACTION_ATTEMPTS => {
                sleep_interruptible(running, RETRY_GAP);
            }
            TargetReaction::StillVisible => {
                return Err(format!(
                    "target did not react after {MAX_ACTION_ATTEMPTS} verified input attempts"
                ));
            }
            TargetReaction::Unavailable(reason) => {
                return Err(format!("could not verify target reaction: {reason}"));
            }
        }
    }
    unreachable!("the bounded attempt loop always returns")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn a_changed_target_accepts_the_first_delivery() {
        let running = AtomicBool::new(true);
        let deliveries = AtomicUsize::new(0);
        let receipt = execute_verified(
            &running,
            || {
                deliveries.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
            |_| Ok(TargetReaction::Changed),
        )
        .unwrap();
        assert_eq!(receipt.attempts, 1);
        assert_eq!(deliveries.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn an_unchanged_target_retries_until_a_fresh_observation_reacts() {
        let running = AtomicBool::new(true);
        let deliveries = AtomicUsize::new(0);
        let observations = AtomicUsize::new(0);
        let receipt = execute_verified(
            &running,
            || {
                deliveries.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
            |_| {
                let pass = observations.fetch_add(1, Ordering::SeqCst);
                Ok(if pass < 2 {
                    TargetReaction::StillVisible
                } else {
                    TargetReaction::Changed
                })
            },
        )
        .unwrap();
        assert_eq!(receipt.attempts, 3);
        assert_eq!(deliveries.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn a_permanently_visible_target_is_bounded() {
        let running = AtomicBool::new(true);
        let deliveries = AtomicUsize::new(0);
        let error = execute_verified(
            &running,
            || {
                deliveries.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
            |_| Ok(TargetReaction::StillVisible),
        )
        .unwrap_err();
        assert!(error.contains("3 verified input attempts"));
        assert_eq!(deliveries.load(Ordering::SeqCst), MAX_ACTION_ATTEMPTS);
    }

    #[test]
    fn cancellation_prevents_the_next_retry() {
        let running = AtomicBool::new(true);
        let deliveries = AtomicUsize::new(0);
        let error = execute_verified(
            &running,
            || {
                deliveries.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
            |_| {
                running.store(false, Ordering::SeqCst);
                Ok(TargetReaction::Changed)
            },
        )
        .unwrap_err();
        assert!(error.contains("cancelled"));
        assert_eq!(deliveries.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn only_new_or_settled_later_samples_can_verify_an_action() {
        let captured = Instant::now();
        let baseline = FrameStamp {
            sample: 7,
            generation: 4,
            fresh: true,
            captured_at: captured,
        };
        assert!(!observation_is_settled(
            baseline,
            baseline,
            Duration::from_secs(1)
        ));
        assert!(!observation_is_settled(
            baseline,
            FrameStamp {
                sample: 8,
                generation: 5,
                fresh: true,
                captured_at: captured + Duration::from_millis(1),
            },
            Duration::ZERO
        ));
        assert!(observation_is_settled(
            baseline,
            FrameStamp {
                sample: 8,
                generation: 5,
                fresh: true,
                captured_at: captured + Duration::from_millis(1),
            },
            MIN_REACTION_SETTLE
        ));
        assert!(!observation_is_settled(
            baseline,
            FrameStamp {
                sample: 8,
                generation: 4,
                fresh: false,
                captured_at: captured + Duration::from_millis(1),
            },
            FRESH_FRAME_FALLBACK_AFTER - Duration::from_millis(1)
        ));
        assert!(!observation_is_settled(
            baseline,
            FrameStamp {
                sample: 9,
                generation: 4,
                fresh: false,
                captured_at: captured + Duration::from_millis(1),
            },
            FRESH_FRAME_FALLBACK_AFTER
        ));
    }

    #[test]
    fn target_identity_allows_hover_resize_but_rejects_another_button() {
        let target = TargetBox::new(500, 300, 180, 40);
        assert!(target.matches(TargetBox::new(503, 301, 184, 42)));
        assert!(!target.matches(TargetBox::new(900, 300, 180, 40)));
    }

    #[test]
    fn concurrent_verified_actions_cannot_interleave_delivery_and_acceptance() {
        let log = std::sync::Arc::new(Mutex::new(Vec::<String>::new()));
        let workers = (0..2)
            .map(|id| {
                let perform_log = log.clone();
                let verify_log = log.clone();
                std::thread::spawn(move || {
                    let running = AtomicBool::new(true);
                    execute_verified(
                        &running,
                        || {
                            perform_log.lock().unwrap().push(format!("act:{id}"));
                            std::thread::sleep(Duration::from_millis(5));
                            Ok(())
                        },
                        |_| {
                            verify_log.lock().unwrap().push(format!("verify:{id}"));
                            Ok(TargetReaction::Changed)
                        },
                    )
                    .unwrap();
                })
            })
            .collect::<Vec<_>>();
        for worker in workers {
            worker.join().unwrap();
        }
        let log = log.lock().unwrap();
        assert_eq!(log.len(), 4);
        for pair in log.chunks_exact(2) {
            assert_eq!(
                pair[0].strip_prefix("act:"),
                pair[1].strip_prefix("verify:")
            );
        }
    }

    #[test]
    fn a_delivery_only_transaction_cannot_split_a_verified_action() {
        let log = std::sync::Arc::new(Mutex::new(Vec::<&'static str>::new()));
        let (verified_started_tx, verified_started_rx) = std::sync::mpsc::channel();
        let verified_log = log.clone();
        let verified = std::thread::spawn(move || {
            let running = AtomicBool::new(true);
            execute_verified(
                &running,
                || {
                    verified_log.lock().unwrap().push("verified:act");
                    verified_started_tx.send(()).unwrap();
                    std::thread::sleep(Duration::from_millis(5));
                    Ok(())
                },
                |_| {
                    verified_log.lock().unwrap().push("verified:verify");
                    Ok(TargetReaction::Changed)
                },
            )
            .unwrap();
        });
        verified_started_rx.recv().unwrap();
        let delivery_log = log.clone();
        let delivery = std::thread::spawn(move || {
            let running = AtomicBool::new(true);
            execute_serialized(&running, || {
                delivery_log.lock().unwrap().push("delivery");
                Ok(())
            })
            .unwrap();
        });

        verified.join().unwrap();
        delivery.join().unwrap();

        assert_eq!(
            *log.lock().unwrap(),
            vec!["verified:act", "verified:verify", "delivery"]
        );
    }

    #[test]
    fn concurrent_action_soak_preserves_every_delivery_verification_pair() {
        const WORKERS: usize = 8;
        const ACTIONS_PER_WORKER: usize = 200;
        let start = std::sync::Arc::new(std::sync::Barrier::new(WORKERS));
        let log = std::sync::Arc::new(Mutex::new(Vec::<(usize, usize, bool)>::new()));
        let workers = (0..WORKERS)
            .map(|worker| {
                let start = start.clone();
                let perform_log = log.clone();
                let verify_log = log.clone();
                std::thread::spawn(move || {
                    let running = AtomicBool::new(true);
                    start.wait();
                    for action in 0..ACTIONS_PER_WORKER {
                        execute_verified(
                            &running,
                            || {
                                perform_log.lock().unwrap().push((worker, action, false));
                                Ok(())
                            },
                            |_| {
                                verify_log.lock().unwrap().push((worker, action, true));
                                Ok(TargetReaction::Changed)
                            },
                        )
                        .unwrap();
                    }
                })
            })
            .collect::<Vec<_>>();
        for worker in workers {
            worker.join().unwrap();
        }

        let log = log.lock().unwrap();
        assert_eq!(log.len(), WORKERS * ACTIONS_PER_WORKER * 2);
        for pair in log.chunks_exact(2) {
            assert_eq!((pair[0].0, pair[0].1), (pair[1].0, pair[1].1));
            assert!(!pair[0].2);
            assert!(pair[1].2);
        }
    }

    #[test]
    fn cancellation_releases_a_waiter_before_the_active_session_finishes() {
        let (holder_entered_tx, holder_entered_rx) = std::sync::mpsc::channel();
        let (release_holder_tx, release_holder_rx) = std::sync::mpsc::channel();
        let holder = std::thread::spawn(move || {
            let running = AtomicBool::new(true);
            execute_serialized(&running, || {
                holder_entered_tx.send(()).unwrap();
                release_holder_rx.recv().unwrap();
                Ok(())
            })
            .unwrap();
        });
        holder_entered_rx.recv().unwrap();

        let running = std::sync::Arc::new(AtomicBool::new(true));
        let waiter_running = running.clone();
        let (waiter_started_tx, waiter_started_rx) = std::sync::mpsc::channel();
        let (result_tx, result_rx) = std::sync::mpsc::channel();
        let waiter = std::thread::spawn(move || {
            waiter_started_tx.send(()).unwrap();
            result_tx
                .send(execute_serialized(&waiter_running, || Ok(())))
                .unwrap();
        });
        waiter_started_rx.recv().unwrap();
        running.store(false, Ordering::SeqCst);

        let result_before_release = result_rx.recv_timeout(Duration::from_millis(100));
        release_holder_tx.send(()).unwrap();
        holder.join().unwrap();
        waiter.join().unwrap();

        let result = result_before_release.expect("cancelled waiter stayed blocked on the session");
        assert!(result.unwrap_err().contains("cancelled"));
    }

    #[test]
    fn action_session_recovers_after_a_panicking_caller() {
        let panicked = std::thread::spawn(|| {
            let running = AtomicBool::new(true);
            let _: Result<(), String> = execute_serialized(&running, || {
                panic!("simulated actuator panic");
            });
        })
        .join();
        assert!(panicked.is_err());

        let running = AtomicBool::new(true);
        assert_eq!(execute_serialized(&running, || Ok(42)).unwrap(), 42);
    }
}
