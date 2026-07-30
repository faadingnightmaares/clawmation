//! Fullscreen GDI overlay: the Win32 scaffolding the screen pickers draw on.
//!
//! Ports the window/message/paint half that `region_picker.py`, `color_sampler.py`
//! and `surgical_picker.py` each hand-rolled through `ctypes`, three near-identical
//! times. A picker supplies a [`Scene`]: it reacts to [`Event`]s and paints through
//! a [`Painter`]; this module owns the topmost popup window, the message pump and
//! the off-screen double buffer that keeps the overlay from tearing mid-drag.
//!
//! Two departures from the Python originals, both strict improvements:
//!   * Mouse coordinates are sign-extended (`GET_X_LPARAM`) rather than masked with
//!     `& 0xFFFF`, so a cursor left of the origin reads as negative instead of
//!     ~65535. Unreachable on a primary-monitor overlay, wrong everywhere else.
//!   * Text is drawn in Segoe UI and centred by its measured extent, where Python
//!     used the stock bitmap font and guessed `len(text) * 8` pixels.

use std::ffi::c_void;
use std::iter::once;
use std::mem::size_of;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::Duration;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW, CreatePen,
    CreateSolidBrush, DeleteDC, DeleteObject, EndPaint, GetStockObject, GetTextExtentPoint32W,
    InvalidateRect, LineTo, MoveToEx, Rectangle, SelectObject, SetBkMode, SetTextColor,
    StretchDIBits, TextOutW, UpdateWindow, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CLEARTYPE_QUALITY,
    CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DIB_RGB_COLORS, HDC, HGDIOBJ, NULL_BRUSH, OUT_TT_PRECIS,
    PAINTSTRUCT, PS_SOLID, SRCCOPY, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_SHIFT};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    GetSystemMetrics, GetWindowLongPtrW, LoadCursorW, PostMessageW, PostQuitMessage,
    RegisterClassW, SetForegroundWindow, SetWindowLongPtrW, SetWindowTextW, ShowWindow,
    TranslateMessage, CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW, GWLP_USERDATA, IDC_CROSS, MSG,
    SM_CXSCREEN, SM_CYSCREEN, SW_SHOWNA, WM_CLOSE, WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCCREATE, WM_PAINT,
    WM_RBUTTONDOWN, WNDCLASSW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
};

/// Segoe UI at ~16px. Negative height is a *cell* height request, the usual way to
/// ask for a specific pixel size rather than a point size.
const FONT_HEIGHT: i32 = -16;
const FONT_WEIGHT: i32 = 400;

/// A colour in the `(b, g, r)` tuple order the Python pickers wrote their palettes
/// in, so the constants port across unchanged.
pub type Bgr = (u8, u8, u8);

/// `COLORREF` is `0x00bbggrr`, exactly what `_rgb()` built by hand.
fn colorref(bgr: Bgr) -> COLORREF {
    let (b, g, r) = bgr;
    COLORREF((u32::from(b) << 16) | (u32::from(g) << 8) | u32::from(r))
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(once(0)).collect()
}

/// Primary-display size in physical pixels, the overlay's full extent. Unaware
/// threads get the logical (scaled-down) metrics, which would size the overlay
/// short of the physical screen's edges and break the alignment between a drag
/// and the captured frame it selects from (issue #5).
pub fn screen_size() -> (i32, i32) {
    let _aware = crate::hardware::dpi::PerMonitorAware::new();
    unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) }
}

// ── Bitmaps ──────────────────────────────────────────────────────────────────

/// A 32-bit top-down BGRA image ready for `StretchDIBits`.
///
/// Python flipped every buffer vertically because it described the DIB with a
/// positive `biHeight` (bottom-up). A negative height says "top-down" and the flip
/// disappears: same pixels on screen, one less full-frame copy per build.
pub struct Dib {
    bits: Vec<u8>,
    width: i32,
    height: i32,
}

impl Dib {
    /// Widen tightly-packed BGR to BGRA with an opaque alpha channel.
    pub fn from_bgr(bgr: &[u8], width: u32, height: u32) -> Self {
        let mut bits = Vec::with_capacity(bgr.len() / 3 * 4);
        for px in bgr.chunks_exact(3) {
            bits.extend_from_slice(&[px[0], px[1], px[2], 255]);
        }
        Self {
            bits,
            width: width as i32,
            height: height as i32,
        }
    }

    #[cfg(test)]
    pub fn bytes(&self) -> &[u8] {
        &self.bits
    }

    fn header(&self) -> BITMAPINFO {
        BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: self.width,
                biHeight: -self.height, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        }
    }
}

// ── Painting ─────────────────────────────────────────────────────────────────

/// The drawing surface handed to [`Scene::paint`]. Every call targets the
/// off-screen buffer, never the visible DC.
pub struct Painter {
    hdc: HDC,
}

impl Painter {
    /// Blit a bitmap 1:1 with its top-left at `(x, y)`.
    pub fn image(&self, dib: &Dib, x: i32, y: i32) {
        let bmi = dib.header();
        unsafe {
            StretchDIBits(
                self.hdc,
                x,
                y,
                dib.width,
                dib.height,
                0,
                0,
                dib.width,
                dib.height,
                Some(dib.bits.as_ptr() as *const c_void),
                &bmi,
                DIB_RGB_COLORS,
                SRCCOPY,
            );
        }
    }

    /// Draw a batch of segments with one pen, the shape the pixel grid and the
    /// stroke list both want.
    pub fn lines(&self, segments: &[(i32, i32, i32, i32)], color: Bgr, width: i32) {
        if segments.is_empty() {
            return;
        }
        unsafe {
            let pen = CreatePen(PS_SOLID, width, colorref(color));
            let old = SelectObject(self.hdc, HGDIOBJ(pen.0));
            for &(x1, y1, x2, y2) in segments {
                let _ = MoveToEx(self.hdc, x1, y1, None);
                let _ = LineTo(self.hdc, x2, y2);
            }
            SelectObject(self.hdc, old);
            let _ = DeleteObject(HGDIOBJ(pen.0));
        }
    }

    pub fn line(&self, x1: i32, y1: i32, x2: i32, y2: i32, color: Bgr, width: i32) {
        self.lines(&[(x1, y1, x2, y2)], color, width);
    }

    /// A rectangle outline; `(x2, y2)` is exclusive, like GDI's `Rectangle`.
    pub fn frame_rect(&self, x1: i32, y1: i32, x2: i32, y2: i32, color: Bgr, width: i32) {
        unsafe {
            let pen = CreatePen(PS_SOLID, width, colorref(color));
            let old_pen = SelectObject(self.hdc, HGDIOBJ(pen.0));
            let old_brush = SelectObject(self.hdc, GetStockObject(NULL_BRUSH));
            let _ = Rectangle(self.hdc, x1, y1, x2, y2);
            SelectObject(self.hdc, old_brush);
            SelectObject(self.hdc, old_pen);
            let _ = DeleteObject(HGDIOBJ(pen.0));
        }
    }

    pub fn filled_rect(&self, x1: i32, y1: i32, x2: i32, y2: i32, color: Bgr) {
        unsafe {
            let brush = CreateSolidBrush(colorref(color));
            let pen = CreatePen(PS_SOLID, 1, colorref(color));
            let old_brush = SelectObject(self.hdc, HGDIOBJ(brush.0));
            let old_pen = SelectObject(self.hdc, HGDIOBJ(pen.0));
            let _ = Rectangle(self.hdc, x1, y1, x2, y2);
            SelectObject(self.hdc, old_pen);
            SelectObject(self.hdc, old_brush);
            let _ = DeleteObject(HGDIOBJ(pen.0));
            let _ = DeleteObject(HGDIOBJ(brush.0));
        }
    }

    /// Rendered size of `text` in the overlay font.
    pub fn text_size(&self, text: &str) -> (i32, i32) {
        let buf: Vec<u16> = text.encode_utf16().collect();
        let mut size = SIZE::default();
        unsafe {
            let _ = GetTextExtentPoint32W(self.hdc, &buf, &mut size);
        }
        (size.cx, size.cy)
    }

    pub fn text(&self, x: i32, y: i32, text: &str, color: Bgr) {
        let buf: Vec<u16> = text.encode_utf16().collect();
        unsafe {
            SetBkMode(self.hdc, TRANSPARENT);
            SetTextColor(self.hdc, colorref(color));
            let _ = TextOutW(self.hdc, x, y, &buf);
        }
    }

    /// Draw `text` horizontally centred on `cx`, clamped away from the left edge.
    pub fn text_centered(&self, cx: i32, y: i32, text: &str, color: Bgr) {
        let (w, _) = self.text_size(text);
        self.text((cx - w / 2).max(8), y, text, color);
    }
}

// ── Scenes ───────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    MouseMove {
        x: i32,
        y: i32,
    },
    LeftDown {
        x: i32,
        y: i32,
    },
    LeftUp {
        x: i32,
        y: i32,
    },
    RightDown {
        x: i32,
        y: i32,
    },
    /// Notches, positive when the wheel is pushed away from the user.
    Wheel {
        delta: i32,
    },
    Key {
        vk: u32,
        shift: bool,
    },
}

/// What the overlay does after a scene handled an event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    /// Repaint the whole overlay.
    Repaint,
    /// Nothing changed on screen.
    Idle,
    /// The scene is finished; tear the window down.
    Close,
}

/// The live overlay window, for the few things a scene drives directly.
pub struct Window {
    hwnd: HWND,
}

impl Window {
    /// Retitle the window. Invisible on a borderless popup, but it is what shows
    /// in the taskbar preview and in accessibility tooling; the Python pickers
    /// used it as their phase indicator, so the calls are kept.
    pub fn set_title(&self, title: &str) {
        unsafe {
            let _ = SetWindowTextW(self.hwnd, PCWSTR(wide(title).as_ptr()));
        }
    }

    /// A handle attached to nothing, so a scene's event logic can be exercised
    /// without a message pump. Retitling is all the scenes ask of a `Window`,
    /// and on a null handle that is a Win32 no-op the callers already ignore.
    #[cfg(test)]
    pub fn detached() -> Self {
        Self {
            hwnd: HWND(std::ptr::null_mut()),
        }
    }
}

/// A picker's interaction model: state, event handling, and how it paints.
pub trait Scene {
    /// What the picker yields once the user commits.
    type Output;

    fn event(&mut self, ev: Event, win: &Window) -> Flow;
    fn paint(&mut self, p: &Painter, sw: i32, sh: i32);
    /// The committed value, moved out after the pump stops. `None` = cancelled.
    fn take(&mut self) -> Option<Self::Output>;
}

// ── The pump ─────────────────────────────────────────────────────────────────

/// Show `scene` fullscreen and block until the user commits or cancels.
///
/// The window lives on its own thread with its own message queue, exactly as the
/// Python pickers did; the caller is a Tauri command thread that must not start
/// pumping Win32 messages. `timeout` bounds the wait: unlike Python, which
/// abandoned a stuck overlay on screen forever, expiry posts `WM_CLOSE` so the
/// window actually goes away.
pub fn run<S>(class: &str, title: &str, scene: S, timeout: Duration) -> Option<S::Output>
where
    S: Scene + Send + 'static,
    S::Output: Send + 'static,
{
    let (tx, rx) = mpsc::channel();
    let hwnd_slot = Arc::new(AtomicIsize::new(0));
    let slot = Arc::clone(&hwnd_slot);
    let (class, title) = (class.to_string(), title.to_string());

    thread::spawn(move || {
        // DPI awareness is per-thread, so it must be forced on *this* thread —
        // the one that creates the window and pumps its messages. Without it,
        // CreateWindowExW interprets the physical screen extent in logical
        // units (at 125% scaling the window comes out a quarter of the screen
        // short on each axis) and WM_MOUSEMOVE reports coordinates in a space
        // the physical-pixel captured frame does not share, so the region a
        // user drags is not the region that gets picked (issue #5). Held for
        // the whole pump: creation, sizing, and input all agree on physical
        // pixels.
        let _aware = crate::hardware::dpi::PerMonitorAware::new();
        let mut scene = scene;
        let hwnd = unsafe { create_window::<S>(&class, &title, &mut scene as *mut S) };
        if let Some(hwnd) = hwnd {
            slot.store(hwnd.0 as isize, Ordering::SeqCst);
            unsafe { pump() };
        }
        let _ = tx.send(scene.take());
    });

    match rx.recv_timeout(timeout) {
        Ok(picked) => picked,
        Err(_) => {
            let raw = hwnd_slot.load(Ordering::SeqCst);
            if raw != 0 {
                unsafe {
                    let _ = PostMessageW(
                        Some(HWND(raw as *mut c_void)),
                        WM_CLOSE,
                        WPARAM(0),
                        LPARAM(0),
                    );
                }
            }
            rx.recv_timeout(Duration::from_secs(2)).unwrap_or(None)
        }
    }
}

/// The overlay font, created once and kept for the process. GDI font handles are
/// cheap to keep and expensive to rebuild on every `WM_PAINT`.
fn ui_font() -> HGDIOBJ {
    static FONT: OnceLock<isize> = OnceLock::new();
    let raw = *FONT.get_or_init(|| unsafe {
        CreateFontW(
            FONT_HEIGHT,
            0,
            0,
            0,
            FONT_WEIGHT,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_TT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            0,
            PCWSTR(wide("Segoe UI").as_ptr()),
        )
        .0 as isize
    });
    HGDIOBJ(raw as *mut c_void)
}

unsafe fn create_window<S: Scene>(class: &str, title: &str, state: *mut S) -> Option<HWND> {
    let hinstance = GetModuleHandleW(None).ok()?;
    let class_w = wide(class);
    let title_w = wide(title);

    let wc = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(wndproc::<S>),
        hInstance: hinstance.into(),
        hCursor: LoadCursorW(None, IDC_CROSS).unwrap_or_default(),
        hbrBackground: windows::Win32::Graphics::Gdi::HBRUSH(GetStockObject(NULL_BRUSH).0),
        lpszClassName: PCWSTR(class_w.as_ptr()),
        ..Default::default()
    };
    // Re-registering an existing class fails harmlessly; the pickers reopen often.
    RegisterClassW(&wc);

    let (sw, sh) = screen_size();
    let hwnd = CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
        PCWSTR(class_w.as_ptr()),
        PCWSTR(title_w.as_ptr()),
        WS_POPUP | WS_VISIBLE,
        0,
        0,
        sw,
        sh,
        None,
        None,
        Some(hinstance.into()),
        Some(state as *const c_void),
    )
    .ok()?;

    let _ = ShowWindow(hwnd, SW_SHOWNA);
    let _ = UpdateWindow(hwnd);
    // Needed for keyboard input to reach the overlay at all.
    let _ = SetForegroundWindow(hwnd);
    Some(hwnd)
}

unsafe fn pump() {
    let mut msg = MSG::default();
    // `GetMessageW` returns -1 on error, which `as_bool()` would read as "keep
    // going". Compare against 0 explicitly.
    while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
        let _ = TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
}

fn lparam_point(lp: LPARAM) -> (i32, i32) {
    let x = (lp.0 & 0xFFFF) as u16 as i16;
    let y = ((lp.0 >> 16) & 0xFFFF) as u16 as i16;
    (i32::from(x), i32::from(y))
}

fn translate(msg: u32, wp: WPARAM, lp: LPARAM) -> Option<Event> {
    let shift = || unsafe { GetKeyState(VK_SHIFT.0 as i32) & 0x8000u16 as i16 != 0 };
    let (x, y) = lparam_point(lp);
    match msg {
        WM_MOUSEMOVE => Some(Event::MouseMove { x, y }),
        WM_LBUTTONDOWN => Some(Event::LeftDown { x, y }),
        WM_LBUTTONUP => Some(Event::LeftUp { x, y }),
        WM_RBUTTONDOWN => Some(Event::RightDown { x, y }),
        WM_MOUSEWHEEL => {
            let delta = ((wp.0 >> 16) & 0xFFFF) as u16 as i16;
            Some(Event::Wheel {
                delta: i32::from(delta),
            })
        }
        WM_KEYDOWN => Some(Event::Key {
            vk: wp.0 as u32,
            shift: shift(),
        }),
        _ => None,
    }
}

unsafe extern "system" fn wndproc<S: Scene>(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
) -> LRESULT {
    if msg == WM_NCCREATE {
        let cs = lp.0 as *const CREATESTRUCTW;
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, (*cs).lpCreateParams as isize);
        return DefWindowProcW(hwnd, msg, wp, lp);
    }

    let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut S;
    if state.is_null() {
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    let scene = &mut *state;

    match msg {
        // We repaint every pixel through the back buffer, so the system erase is
        // pure white flicker.
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            paint(hwnd, scene);
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => match translate(msg, wp, lp) {
            None => DefWindowProcW(hwnd, msg, wp, lp),
            Some(ev) => {
                match scene.event(ev, &Window { hwnd }) {
                    Flow::Repaint => {
                        let _ = InvalidateRect(Some(hwnd), None, false);
                    }
                    Flow::Idle => {}
                    Flow::Close => {
                        let _ = DestroyWindow(hwnd);
                    }
                }
                LRESULT(0)
            }
        },
    }
}

/// Double-buffered paint: the scene draws into a memory DC, which lands on screen
/// in a single `BitBlt`. Drawing straight onto the visible DC lets the monitor
/// sample a half-painted frame, the flicker the Python pickers fixed the same way.
unsafe fn paint<S: Scene>(hwnd: HWND, scene: &mut S) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);
    let (sw, sh) = screen_size();

    let mem = CreateCompatibleDC(Some(hdc));
    let bmp = CreateCompatibleBitmap(hdc, sw, sh);
    let old_bmp = SelectObject(mem, HGDIOBJ(bmp.0));
    let old_font = SelectObject(mem, ui_font());

    scene.paint(&Painter { hdc: mem }, sw, sh);
    let _ = BitBlt(hdc, 0, 0, sw, sh, Some(mem), 0, 0, SRCCOPY);

    SelectObject(mem, old_font);
    SelectObject(mem, old_bmp);
    let _ = DeleteObject(HGDIOBJ(bmp.0));
    let _ = DeleteDC(mem);
    let _ = EndPaint(hwnd, &ps);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colorref_packs_bgr_the_way_win32_reads_it() {
        // 0x00bbggrr, the layout `_rgb()` built by hand in every Python picker.
        assert_eq!(colorref((0x11, 0x22, 0x33)).0, 0x0011_2233);
        assert_eq!(colorref((0, 0, 255)).0, 0x0000_00ff); // pure red
        assert_eq!(colorref((255, 0, 0)).0, 0x00ff_0000); // pure blue
    }

    #[test]
    fn lparam_point_sign_extends_negative_coordinates() {
        // Cursor at (10, 20).
        assert_eq!(lparam_point(LPARAM((20 << 16) | 10)), (10, 20));
        // Cursor at (-3, -1): masking with 0xFFFF would have read 65533 / 65535.
        let packed = ((0xFFFFi64 & -1) << 16) | (0xFFFF & -3);
        assert_eq!(lparam_point(LPARAM(packed as isize)), (-3, -1));
    }

    #[test]
    fn wheel_delta_is_signed() {
        let up = translate(WM_MOUSEWHEEL, WPARAM(120 << 16), LPARAM(0));
        assert_eq!(up, Some(Event::Wheel { delta: 120 }));
        let down = translate(WM_MOUSEWHEEL, WPARAM((0xFF88u32 as usize) << 16), LPARAM(0));
        assert_eq!(down, Some(Event::Wheel { delta: -120 }));
    }

    #[test]
    fn unhandled_messages_translate_to_nothing() {
        assert_eq!(translate(WM_PAINT, WPARAM(0), LPARAM(0)), None);
    }

    #[test]
    fn dib_widens_bgr_to_opaque_bgra_top_down() {
        let dib = Dib::from_bgr(&[1, 2, 3, 4, 5, 6], 2, 1);
        assert_eq!(dib.bits, vec![1, 2, 3, 255, 4, 5, 6, 255]);
        // Negative height is what tells GDI the rows are already top-down.
        assert_eq!(dib.header().bmiHeader.biHeight, -1);
        assert_eq!(dib.header().bmiHeader.biWidth, 2);
    }
}
