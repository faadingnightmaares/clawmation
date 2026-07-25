//! Keeping Clawmation's own windows out of Clawmation's own screen captures.
//!
//! Every grab the app makes wants the desktop *without* us in it: the pickers
//! ask the user to point at the game, a guard's Test button matches against what
//! the game is showing, and a trigger watching while a macro plays must not find
//! its own answer painted on our overlay. The blunt way to get that is to hide
//! the window across the capture, which is what this replaced, because a window
//! that vanishes from the screen and the taskbar for a third of a second reads
//! as a crash and a relaunch, not as a screenshot.
//!
//! Windows 10 2004 added the right answer. `SetWindowDisplayAffinity` with
//! `WDA_EXCLUDEFROMCAPTURE` has DWM compose the capture path as though the
//! window were not there (desktop duplication, BitBlt and PrintWindow all see
//! straight through to whatever is behind it), while the user carries on seeing
//! it normally. Nothing moves, nothing blinks.
//!
//! It is a property of the window rather than of the capture, so it holds no
//! matter which backend does the grabbing, and it costs nothing to leave on.

use std::ffi::c_void;

use windows_sys::Win32::UI::WindowsAndMessaging::{
    SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE, WDA_NONE,
};

/// Take `hwnd` out of every screen-capture path, or put it back.
///
/// `false` means the platform refused. Windows 10 before build 2004 is the
/// documented case, and a caller that needs the guarantee has to fall back to
/// hiding the window for real.
pub fn set_excluded(hwnd: *mut c_void, excluded: bool) -> bool {
    if hwnd.is_null() {
        return false;
    }
    let affinity = if excluded { WDA_EXCLUDEFROMCAPTURE } else { WDA_NONE };
    unsafe { SetWindowDisplayAffinity(hwnd, affinity) != 0 }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    use super::super::capture::ScreenCapture;
    use super::super::overlay::{self, Bgr, Event, Flow, Painter, Scene, Window};
    use super::*;

    const CLASS: &str = "ClawmationShieldTest";
    /// Nothing on a desktop is this colour, so "did the capture see the window"
    /// is a single pixel comparison.
    const MAGENTA: Bgr = (255, 0, 255);

    /// Fills the screen and answers nothing; the pump closes it on timeout.
    struct SolidScene;

    impl Scene for SolidScene {
        type Output = ();
        fn event(&mut self, _ev: Event, _win: &Window) -> Flow {
            Flow::Idle
        }
        fn paint(&mut self, p: &Painter, sw: i32, sh: i32) {
            p.filled_rect(-1, -1, sw + 1, sh + 1, MAGENTA);
        }
        fn take(&mut self) -> Option<()> {
            None
        }
    }

    /// The centre pixel of one grab through `backend`.
    fn centre_pixel(backend: &str) -> Option<(u8, u8, u8)> {
        let mut cap = ScreenCapture::new(backend, None);
        let frame = cap.grab();
        cap.close();
        let f = frame?;
        let i = ((f.height as usize / 2) * f.width as usize + f.width as usize / 2) * 3;
        Some((f.bgr[i], f.bgr[i + 1], f.bgr[i + 2]))
    }

    fn find_overlay() -> *mut std::ffi::c_void {
        let class: Vec<u16> = CLASS.encode_utf16().chain(std::iter::once(0)).collect();
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::FindWindowW(
                class.as_ptr(),
                std::ptr::null(),
            )
        }
    }

    /// The whole point of the module, against the real capture backends: a
    /// window the user can see, that a screen grab cannot.
    ///
    /// Ignored because it covers the screen with a magenta sheet for two
    /// seconds. Run by hand with
    /// `cargo test --lib shield -- --ignored --test-threads=1`.
    #[test]
    #[ignore = "covers the real screen with a fullscreen window and grabs it"]
    fn an_excluded_window_is_missing_from_every_backend() {
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(900));
            let hwnd = find_overlay();
            let before: Vec<_> = ["mss", "dxcam"].iter().map(|b| centre_pixel(b)).collect();
            let ok = set_excluded(hwnd, true);
            thread::sleep(Duration::from_millis(400));
            let after: Vec<_> = ["mss", "dxcam"].iter().map(|b| centre_pixel(b)).collect();
            let _ = set_excluded(hwnd, false);
            // As an integer: a raw pointer is not `Send`, and this one is only
            // ever read back as "was there a window at all".
            let _ = tx.send((hwnd as usize, ok, before, after));
        });

        overlay::run(CLASS, "Clawmation shield test", SolidScene, Duration::from_secs(2));
        let (hwnd, ok, before, after) = rx.recv_timeout(Duration::from_secs(5)).expect("driver ran");

        assert!(hwnd != 0, "the overlay window was never found");
        assert!(ok, "SetWindowDisplayAffinity was refused on this build of Windows");
        for (i, backend) in ["mss", "dxcam"].iter().enumerate() {
            assert_eq!(
                before[i],
                Some(MAGENTA),
                "{backend} did not see the window before it was excluded, so the test proves nothing"
            );
            assert_ne!(after[i], Some(MAGENTA), "{backend} still captured the excluded window");
        }
    }
}
