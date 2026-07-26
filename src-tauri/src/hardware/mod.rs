//! Hardware layer: the OS-touching primitives the engines drive.
//!
//! Where `engine/` is deliberately hardware-free (managers with injected
//! callbacks), these modules call straight into Win32, mirroring the Python
//! `input.py` / `capture.py` modules. `vision` is the exception in the other
//! direction: it touches no OS API at all, being a transcription of the OpenCV
//! primitives the detection path used to reach for.

pub mod capture;
pub mod dpi;
pub mod input;
pub mod ocr;
pub mod overlay;
pub mod picker;
pub mod player;
pub mod preview;
pub mod recorder;
pub mod shield;
pub mod snap;
pub mod vision;
pub mod window;

use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};

use crate::hardware::dpi::PerMonitorAware;

/// Primary-display pixel size via Win32 `GetSystemMetrics`, the raw half of
/// Python's `_get_screen_resolution` (the caller supplies the config fallback).
///
/// Queried under a Per-Monitor-V2 thread context so the result is PHYSICAL
/// pixels regardless of the display's scaling. `GetSystemMetrics` is
/// DPI-virtualized per the *calling thread*: an unaware thread on a 2K panel at
/// 125% reports the scaled-down size, which would mismatch the physical
/// coordinates the recorder's hook captures and the physical placement
/// `move_to` performs. This is the single source for both the recorded
/// resolution and the playback target (`core::resolve_screen`), so guarding it
/// keeps record→play at scale 1.0 on the recording display at any DPI.
pub fn screen_size() -> (u32, u32) {
    let _aware = PerMonitorAware::new();
    let (w, h) = unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
    (w.max(0) as u32, h.max(0) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Read-only `GetSystemMetrics` under the DPI guard: returns a real size on
    /// any display, scaling included. Safe to run un-ignored — it moves nothing.
    #[test]
    fn screen_size_reports_the_primary_display() {
        let (w, h) = screen_size();
        assert!(w > 0 && h > 0, "primary monitor reports a size, got {w}x{h}");
    }
}
