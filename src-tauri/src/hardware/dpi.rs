//! Per-Monitor-V2 DPI awareness: the whole app deals in PHYSICAL pixels at ANY
//! display scaling, held up by two layers.
//!
//! Layer 1 — process-wide: `run()` calls [`raise_process_to_per_monitor_v2`]
//! before anything else runs, so every thread the app spawns (the recorder's
//! hook thread, the playback thread, the guard engine, the vision loops…)
//! inherits Per-Monitor-V2 as its default context. `GetSystemMetrics`, the
//! `WH_MOUSE_LL` hook coordinates, `GetCursorPos`, and `SetCursorPos` /
//! absolute `SendInput` are ALL DPI-virtualized per the *calling thread*
//! unless that context is Per-Monitor aware; with the process raised, an
//! unaware worker thread cannot exist here, and a 2K panel at 125% records and
//! replays as 2560x1440 — never "counted as 100%", coordinates landing a
//! DPI-factor short of their target.
//!
//! Layer 2 — per-thread RAII guards: the coordinate hot spots (the recorder's
//! hook thread, the playback thread, cursor placement, the GDI grab, the
//! picker overlay) each hold a [`PerMonitorAware`] for as long as they touch
//! screen coordinates, so the pipeline stays physical even if something ever
//! lowered the process default, and so the invariant sits next to the code
//! that depends on it. A guard is a no-op where the thread is already aware
//! and exact where it isn't.
//!
//! Relative `SendInput` movement is separate from the screen-coordinate
//! pipeline: its values are raw counts relative to the previous mouse event,
//! not logical screen coordinates. They therefore must never be DPI-scaled.
//! Playback forwards each recorded delta unchanged for Raw Input consumers,
//! then reconciles the visible cursor to the physical target because Windows
//! can scale the resulting pointer path on a scaled monitor.
//!
//! [`report`] probes both layers on a fresh thread at startup (and on demand
//! via the `dpi_report` command), so a regression names itself in the log on
//! the machine it breaks on instead of being inferred from missed clicks.
//!
//! The embedded app manifest declares no `<dpiAwareness>`; tao raises the UI
//! path at event-loop creation, but that is a framework side effect this crate
//! does not lean on — layer 1 runs first and unconditionally.

use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    DPI_AWARENESS_CONTEXT_UNAWARE,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};

// windows-sys 0.59 ships the `DPI_AWARENESS_CONTEXT_*` constants but not these
// calls, so link them directly from user32.dll (already in the link set for
// `SetCursorPos`/`GetSystemMetrics`). A context is the same `*mut c_void` as
// `HANDLE`; the predicates return `BOOL` (`i32`, nonzero = true).
extern "system" {
    fn SetThreadDpiAwarenessContext(value: HANDLE) -> HANDLE;
    fn SetProcessDpiAwarenessContext(value: HANDLE) -> i32;
    fn GetThreadDpiAwarenessContext() -> HANDLE;
    fn AreDpiAwarenessContextsEqual(value1: HANDLE, value2: HANDLE) -> i32;
}

/// Raise the whole process to Per-Monitor-V2: layer 1 of the guarantee. Called
/// at the very top of `run()`, before any thread, window, or capture exists,
/// so every thread spawned afterwards inherits physical coordinates as its
/// default context. Best-effort by design: the call fails when a context is
/// already pinned (a manifest entry, or tao's own raise at event-loop
/// creation), and in every case where it fails the process is already at least
/// Per-Monitor aware — ignoring the result is correct, not sloppy.
pub(crate) fn raise_process_to_per_monitor_v2() {
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

/// Human-readable name for an awareness context.
fn context_name(ctx: HANDLE) -> &'static str {
    unsafe {
        if AreDpiAwarenessContextsEqual(ctx, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) != 0 {
            "PerMonitorV2"
        } else if AreDpiAwarenessContextsEqual(ctx, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE) != 0 {
            "PerMonitor"
        } else if AreDpiAwarenessContextsEqual(ctx, DPI_AWARENESS_CONTEXT_UNAWARE) != 0 {
            "Unaware"
        } else {
            "other"
        }
    }
}

/// Probe the live DPI state on a FRESH thread — the exact kind of thread the
/// recorder and player spawn — and report whether the two layers agree on
/// physical pixels. `(healthy, line)`: healthy when the probe thread inherits
/// Per-Monitor-V2 AND reads the same primary size bare and under the guard.
/// When something has regressed, the line names which layer (process context,
/// bare-thread metrics, guarded metrics) disagrees.
pub(crate) fn report() -> (bool, String) {
    let probe = std::thread::spawn(|| {
        // Before any guard on this thread: what a worker inherits by default.
        let ctx = unsafe { GetThreadDpiAwarenessContext() };
        let (bare_w, bare_h) =
            unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
        let (guard_w, guard_h) = {
            let _aware = PerMonitorAware::new();
            unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) }
        };
        (context_name(ctx), (bare_w, bare_h), (guard_w, guard_h))
    });
    match probe.join() {
        Ok((ctx, (bw, bh), (gw, gh))) => {
            let healthy = ctx == "PerMonitorV2" && bw > 0 && (bw, bh) == (gw, gh);
            let line = if healthy {
                format!(
                    "DPI ok: process={ctx}, primary {gw}x{gh} physical (bare thread reads {bw}x{bh}, guarded {gw}x{gh})"
                )
            } else {
                format!(
                    "DPI MISMATCH: process={ctx}, bare-thread metrics {bw}x{bh} vs guarded {gw}x{gh} — coordinates would drift by the scale factor"
                )
            };
            (healthy, line)
        }
        Err(_) => (
            false,
            "DPI probe: worker thread died before reporting".to_string(),
        ),
    }
}

/// RAII form of `SetThreadDpiAwarenessContext(PER_MONITOR_AWARE_V2)`: makes this
/// thread Per-Monitor-V2 DPI-aware on construction and restores the previous
/// context on drop. Hold one for as long as the thread reads or writes screen
/// coordinates; it restores the prior context (if any) when it goes out of
/// scope.
pub(crate) struct PerMonitorAware {
    previous: HANDLE,
}

impl PerMonitorAware {
    pub(crate) fn new() -> Self {
        // Returns the prior context, or NULL when unsupported (pre-Windows 10
        // 1607) — in which case awareness is unchanged and there's nothing to
        // restore on drop.
        let previous =
            unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        Self { previous }
    }
}

impl Drop for PerMonitorAware {
    fn drop(&mut self) {
        if !self.previous.is_null() {
            unsafe {
                SetThreadDpiAwarenessContext(self.previous);
            }
        }
    }
}

/// Recorded physical movement → raw `SendInput` relative counts.
///
/// `MOUSEINPUT::dx`/`dy` are relative counts, not screen coordinates, so DPI
/// scaling is invalid here. Keeping this conversion explicit makes that
/// boundary testable and prevents a display-scale correction from being added
/// to the relative playback path again.
pub(crate) fn relative_counts(dx: i32, dy: i32) -> (i32, i32) {
    (dx, dy)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// After the process raise, a freshly spawned thread must inherit
    /// Per-Monitor-V2 and read identical physical metrics with or without the
    /// guard — the regression the field report described would fail exactly
    /// here (bare thread reporting logical 2048x1152 against guarded 2560x1440).
    #[test]
    fn fresh_threads_inherit_per_monitor_v2() {
        raise_process_to_per_monitor_v2();
        let (healthy, line) = report();
        assert!(healthy, "{line}");
    }

    /// Regression: display scaling must not alter raw relative input counts.
    /// At 125% the old conversion turned this 200px recorded segment into 160,
    /// causing playback to drift from every later click.
    #[test]
    fn relative_counts_preserve_recorded_physical_deltas() {
        assert_eq!(relative_counts(200, 180), (200, 180));
        assert_eq!(relative_counts(-3, 1), (-3, 1));
    }
}
