//! Per-thread DPI awareness: forcing Per-Monitor-V2 on every thread that reads
//! or writes screen coordinates, so recording and playback agree on physical
//! pixels at ANY display scaling.
//!
//! The coordinate pipeline spans three threads that nothing guarantees share a
//! DPI context: the low-level mouse hook (its own spawned thread), the screen
//! resolution query (a command-pool thread), and cursor placement (the playback
//! thread). `GetSystemMetrics`, the `WH_MOUSE_LL` coordinates, and
//! `SetCursorPos` / absolute `SendInput` are ALL DPI-virtualized unless the
//! *calling thread* is Per-Monitor aware. The embedded app manifest declares no
//! `<dpiAwareness>` and tao's runtime raise to Per-Monitor-V2 reaches the UI
//! path but cannot be relied upon to govern these worker threads — when it
//! doesn't, a 2K panel at 125% gets recorded and replayed as if it were 100%
//! (coordinates land a DPI-factor short of their target). Forcing the context
//! per-thread makes the whole pipeline physical by construction rather than by
//! whatever awareness the threads happened to inherit; it is a no-op where a
//! thread is already aware and exact where it isn't.

use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2;

// windows-sys 0.59 ships the `DPI_AWARENESS_CONTEXT_*` constants but not this
// call, so link it directly from user32.dll (already in the link set for
// `SetCursorPos`/`GetSystemMetrics`). Takes and returns a `DPI_AWARENESS_CONTEXT`
// — the same `*mut c_void` as `HANDLE`.
extern "system" {
    fn SetThreadDpiAwarenessContext(value: HANDLE) -> HANDLE;
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
