//! Hardware layer: the OS-touching primitives the engines drive.
//!
//! Where `engine/` is deliberately hardware-free (managers with injected
//! callbacks), these modules call straight into Win32, mirroring the Python
//! `input.py` / `capture.py` modules. `vision` is the exception in the other
//! direction: it touches no OS API at all, being a transcription of the OpenCV
//! primitives the detection path used to reach for.

pub mod capture;
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

use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};

/// Primary-display pixel size via Win32 `GetSystemMetrics`, the raw half of
/// Python's `_get_screen_resolution` (the caller supplies the config fallback).
pub fn screen_size() -> (u32, u32) {
    let (w, h) = unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
    (w.max(0) as u32, h.max(0) as u32)
}
