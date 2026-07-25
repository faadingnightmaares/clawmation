//! Screen capture: DXGI Desktop Duplication (primary), GDI BitBlt (fallback).
//!
//! Faithful port of `anime_macro/capture.py::ScreenCapture`. Python names its
//! backends after the libraries it wrapped: `dxcam` (DXGI Desktop Duplication,
//! 120+ FPS) and `mss` (a GDI `BitBlt` grabber, 30-60 FPS). We hand-roll both
//! against Win32 so the resolved-backend names stay `"dxcam"` / `"mss"` exactly
//! as the UI reports them; the pixels differ from Python's (a different capture
//! path) but every *observable* behavior below is preserved.
//!
//! Backend selection mirrors `_init_backend` precisely:
//!   * `"dxcam"` → try Desktop Duplication, fall back to GDI (`"mss"`) on failure.
//!   * `"mss"` (and any unknown value) → GDI.
//!   * `"wgc"` → GDI. Python warns and falls back here because the Windows
//!     Graphics Capture path *segfaults the process*; its `_init_wgc` is dead
//!     code never reached by `_init_backend`, so it is omitted entirely.
//!
//! Two deliberate fidelity decisions, documented in MIGRATION-NOTES "Capture":
//!   * Python's `dxcam` runs a background thread (`video_mode=True`) that keeps
//!     capturing and reuses the last frame when the screen is static, so
//!     `get_latest_frame()` never returns `None`. We reproduce those *semantics*
//!     synchronously: each `grab()` does one non-blocking `AcquireNextFrame`, and
//!     on `WAIT_TIMEOUT` (nothing new) returns the cached last frame. Same
//!     observable result (a fresh frame while the screen moves, the last frame
//!     while it is still, never `None` after the first) without a capture thread
//!     the clean codebase does not need.
//!   * `fps` is Python's exact metric: a rolling mean over the last 60 samples of
//!     `1 / (time to read one frame)`. It measures read throughput, not the
//!     backend's true capture rate, so the absolute number differs from Python's
//!     buffer-read timing, but it is cosmetic telemetry (shown in the UI, never
//!     a decision input), and the *formula* is identical.
//!
//! Intentionally not ported yet (no live consumer; see MIGRATION-NOTES):
//!   * `grab_rgb` (BGR→RGB): lands with the first RGB consumer (a picker /
//!     color sampler slice); every current frame consumer reads BGR.
//!   * the `AppState` lazy-`get_capture` wiring and the `get_status` resolved
//!     backend / real-fps report: non-observable until a frame consumer warms
//!     capture (until then `Api._capture is None`, and the status command already
//!     faithfully reports the config backend and 0.0 fps). They attach with the
//!     detection slice, which also validates cross-thread use behind the mutex.

use std::collections::VecDeque;
use std::ffi::c_void;
use std::mem::size_of;
use std::time::{Duration, Instant};

use windows::core::Interface;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, D3D11_CPU_ACCESS_READ,
    D3D11_CREATE_DEVICE_FLAG, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_SDK_VERSION,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC;
use windows::Win32::Graphics::Dxgi::{
    IDXGIAdapter, IDXGIDevice, IDXGIOutput, IDXGIOutput1, IDXGIOutputDuplication, IDXGIResource,
    DXGI_ERROR_ACCESS_DENIED, DXGI_ERROR_ACCESS_LOST, DXGI_OUTDUPL_FRAME_INFO,
};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, GetDC,
    ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HGDIOBJ, SRCCOPY,
};
use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};

/// A captured frame: tightly-packed BGR bytes plus its dimensions. Deliberately
/// a plain data carrier (not an `image` crate type) so the deferred vision-stack
/// decision (pure-Rust `image`/`imageproc` vs the `opencv` crate) stays open.
#[derive(Clone)]
pub struct Frame {
    /// `width * height * 3` bytes, row-major, BGR order (as OpenCV/Python expect).
    pub bgr: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

impl Frame {
    /// A tight sub-frame of `self`, clamped to its bounds.
    ///
    /// `None` when the rectangle is degenerate or lands entirely outside; the
    /// caller then has nothing to search, which is not an error. Lets one
    /// full-screen grab serve several region-scoped consumers per cycle instead
    /// of re-grabbing per region.
    pub fn crop(&self, x: i32, y: i32, w: i32, h: i32) -> Option<Frame> {
        let (fw, fh) = (self.width as i32, self.height as i32);
        let x0 = x.clamp(0, fw);
        let y0 = y.clamp(0, fh);
        let x1 = (x + w).clamp(0, fw);
        let y1 = (y + h).clamp(0, fh);
        let (cw, ch) = (x1 - x0, y1 - y0);
        if cw <= 0 || ch <= 0 {
            return None;
        }
        let src_stride = self.width as usize * 3;
        let dst_stride = cw as usize * 3;
        let mut bgr = Vec::with_capacity(dst_stride * ch as usize);
        for row in 0..ch {
            let start = (y0 + row) as usize * src_stride + x0 as usize * 3;
            bgr.extend_from_slice(&self.bgr[start..start + dst_stride]);
        }
        Some(Frame { bgr, width: cw as u32, height: ch as u32 })
    }
}

/// Capture region as `(x1, y1, x2, y2)` pixels, matching Python's `region`
/// tuple. `None` = the whole primary screen.
type Region = Option<(i32, i32, i32, i32)>;

// ── Pure helpers (unit-tested without hardware) ──────────────────────────────

/// Rolling FPS window, a direct port of `_track_fps` / the `fps` property:
/// each read pushes `1/dt`, the window is capped at 60 samples, and `fps()` is
/// the mean (0.0 when empty). Non-positive `dt` is dropped, like Python's
/// `if dt > 0`.
struct FpsWindow {
    samples: VecDeque<f64>,
}

impl FpsWindow {
    const CAP: usize = 60;

    fn new() -> Self {
        Self { samples: VecDeque::new() }
    }

    fn record(&mut self, dt: f64) {
        if dt <= 0.0 {
            return;
        }
        self.samples.push_back(1.0 / dt);
        if self.samples.len() > Self::CAP {
            self.samples.pop_front();
        }
    }

    fn fps(&self) -> f64 {
        if self.samples.is_empty() {
            0.0
        } else {
            self.samples.iter().sum::<f64>() / self.samples.len() as f64
        }
    }
}

/// `(x1, y1, x2, y2)` region → `(left, top, width, height)`. `None` covers the
/// whole screen. Matches `mss`'s monitor dict (`width = x2 - x1`) and dxcam's
/// region crop; negative extents are floored at 0 so a degenerate region yields
/// an empty grab rather than a panic.
fn region_rect(region: Region, screen_w: i32, screen_h: i32) -> (i32, i32, i32, i32) {
    match region {
        Some((x1, y1, x2, y2)) => (x1, y1, (x2 - x1).max(0), (y2 - y1).max(0)),
        None => (0, 0, screen_w, screen_h),
    }
}

/// Copy a BGRA source into a tight BGR buffer, cropping to `(left, top, w, h)`.
///
/// `row_pitch` is the source's bytes-per-row: a D3D11 staging texture (and any
/// DIB) pads each row wider than `width * 4`, so we must index by `row_pitch`,
/// never by `width * 4`; a tight-pack assumption skews the image diagonally
/// (and the result is non-zero, so it slips past a naive "not all black" check).
/// Rows/columns past the source edge are left black instead of read
/// out-of-bounds, so an over-large region clamps cleanly.
fn bgra_to_bgr_crop(
    src: &[u8],
    row_pitch: usize,
    src_w: usize,
    src_h: usize,
    left: usize,
    top: usize,
    w: usize,
    h: usize,
) -> Vec<u8> {
    let mut out = vec![0u8; w * h * 3];
    for row in 0..h {
        let sy = top + row;
        if sy >= src_h {
            continue;
        }
        let src_row = sy * row_pitch;
        let dst_row = row * w * 3;
        for col in 0..w {
            let sx = left + col;
            if sx >= src_w {
                continue;
            }
            let si = src_row + sx * 4;
            let di = dst_row + col * 3;
            if si + 2 >= src.len() {
                continue;
            }
            out[di] = src[si]; // B
            out[di + 1] = src[si + 1]; // G
            out[di + 2] = src[si + 2]; // R
        }
    }
    out
}

// ── DXGI Desktop Duplication backend ("dxcam") ───────────────────────────────

/// A live Desktop Duplication session. Created once; `grab` reuses it across
/// calls and transparently rebuilds it after `DXGI_ERROR_ACCESS_LOST` (raised on
/// every fullscreen transition, resolution change, and secure-desktop/UAC switch,
/// all routine for a game macro). COM objects release themselves when this drops.
struct DxgiCapture {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    /// Kept so the duplication can be recreated after it is lost.
    output: IDXGIOutput1,
    /// `None` between an access-loss and the next successful recreate.
    duplication: Option<IDXGIOutputDuplication>,
    /// CPU-readable copy target, cached and rebuilt only when the size changes.
    staging: Option<ID3D11Texture2D>,
    staging_dims: (u32, u32),
    /// Last good frame, reused on `WAIT_TIMEOUT` (static screen) and while the
    /// duplication is being rebuilt; this is what makes `grab` never-`None`.
    last_frame: Option<Frame>,
}

impl DxgiCapture {
    fn new() -> Option<Self> {
        unsafe {
            let mut device: Option<ID3D11Device> = None;
            let mut context: Option<ID3D11DeviceContext> = None;
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_FLAG(0),
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                Some(std::ptr::null_mut::<D3D_FEATURE_LEVEL>()),
                Some(&mut context),
            )
            .ok()?;
            let device = device?;
            let context = context?;

            let dxgi_device: IDXGIDevice = device.cast().ok()?;
            let adapter: IDXGIAdapter = dxgi_device.GetAdapter().ok()?;
            let output: IDXGIOutput = adapter.EnumOutputs(0).ok()?;
            let output1: IDXGIOutput1 = output.cast().ok()?;
            let duplication = output1.DuplicateOutput(&device).ok()?;

            Some(Self {
                device,
                context,
                output: output1,
                duplication: Some(duplication),
                staging: None,
                staging_dims: (0, 0),
                last_frame: None,
            })
        }
    }

    fn grab(&mut self, region: Region) -> Option<Frame> {
        if self.duplication.is_none() {
            self.duplication = unsafe { self.output.DuplicateOutput(&self.device).ok() };
            if self.duplication.is_none() {
                return self.last_frame.clone();
            }
        }
        // Clone the COM handle (a cheap AddRef) so `self` stays free to mutate.
        let dup = self.duplication.clone().unwrap();

        // Copy only frames that carry an actual desktop present. The first
        // `AcquireNextFrame` after `DuplicateOutput` (and any mouse-only update)
        // returns `LastPresentTime == 0` with a *blank* surface, so we skip those
        // and keep the last good frame; dxcam's capture loop does the same. Wait
        // up to a second for that first real frame; once one is cached, never
        // block: a static screen falls straight through to the reused frame.
        let deadline =
            Instant::now() + Duration::from_millis(if self.last_frame.is_none() { 1000 } else { 0 });

        loop {
            let timeout = deadline.saturating_duration_since(Instant::now()).as_millis() as u32;
            let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut resource: Option<IDXGIResource> = None;
            match unsafe { dup.AcquireNextFrame(timeout, &mut info, &mut resource) } {
                Ok(()) => {
                    let frame = if info.LastPresentTime != 0 {
                        unsafe { self.copy_frame(resource, region) }
                    } else {
                        None // metadata-only update: the surface may be blank.
                    };
                    unsafe {
                        let _ = dup.ReleaseFrame();
                    }
                    if let Some(f) = frame {
                        self.last_frame = Some(f.clone());
                        return Some(f);
                    }
                    if self.last_frame.is_some() || Instant::now() >= deadline {
                        return self.last_frame.clone();
                    }
                }
                Err(e) => {
                    let code = e.code();
                    if code == DXGI_ERROR_ACCESS_LOST || code == DXGI_ERROR_ACCESS_DENIED {
                        // Duplication invalidated (fullscreen/mode switch): drop
                        // it so the next grab rebuilds; reuse the cache meanwhile.
                        self.duplication = None;
                        return self.last_frame.clone();
                    }
                    // WAIT_TIMEOUT (static screen) or a transient error: reuse the
                    // cache, else keep waiting for the first frame until deadline.
                    if self.last_frame.is_some() || Instant::now() >= deadline {
                        return self.last_frame.clone();
                    }
                }
            }
        }
    }

    /// Copy the acquired desktop texture into a BGR `Frame`. Caller releases the
    /// duplication frame; this only touches our own staging copy.
    unsafe fn copy_frame(&mut self, resource: Option<IDXGIResource>, region: Region) -> Option<Frame> {
        let texture: ID3D11Texture2D = resource?.cast().ok()?;
        let mut tdesc = D3D11_TEXTURE2D_DESC::default();
        texture.GetDesc(&mut tdesc);

        if self.staging.is_none() || self.staging_dims != (tdesc.Width, tdesc.Height) {
            let sdesc = D3D11_TEXTURE2D_DESC {
                Width: tdesc.Width,
                Height: tdesc.Height,
                MipLevels: 1,
                ArraySize: 1,
                Format: tdesc.Format,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                MiscFlags: 0,
            };
            let mut st: Option<ID3D11Texture2D> = None;
            self.device.CreateTexture2D(&sdesc, None, Some(&mut st)).ok()?;
            self.staging = st;
            self.staging_dims = (tdesc.Width, tdesc.Height);
        }
        let staging = self.staging.clone()?;

        self.context.CopyResource(&staging, &texture);
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        self.context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped)).ok()?;

        let src = std::slice::from_raw_parts(
            mapped.pData as *const u8,
            mapped.RowPitch as usize * tdesc.Height as usize,
        );
        let (left, top, w, h) = region_rect(region, tdesc.Width as i32, tdesc.Height as i32);
        let bgr = bgra_to_bgr_crop(
            src,
            mapped.RowPitch as usize,
            tdesc.Width as usize,
            tdesc.Height as usize,
            left.max(0) as usize,
            top.max(0) as usize,
            w.max(0) as usize,
            h.max(0) as usize,
        );
        self.context.Unmap(&staging, 0);

        Some(Frame { bgr, width: w.max(0) as u32, height: h.max(0) as u32 })
    }
}

// ── GDI BitBlt backend ("mss") ───────────────────────────────────────────────

/// One-shot GDI screen grab, the faithful equivalent of `mss.grab`. 32-bit DIBs
/// are stored BGRA, so dropping the 4th byte yields BGR exactly as `mss`'s
/// `np.array(shot)[:, :, :3]` does. A negative `biHeight` requests a top-down
/// buffer, matching the DXGI path's row order.
fn grab_gdi(region: Region) -> Option<Frame> {
    let (screen_w, screen_h) = unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
    let (left, top, w, h) = region_rect(region, screen_w, screen_h);
    if w <= 0 || h <= 0 {
        return None;
    }

    unsafe {
        let screen = GetDC(None);
        if screen.0.is_null() {
            return None;
        }
        let mem = CreateCompatibleDC(Some(screen));
        let bmp = CreateCompatibleBitmap(screen, w, h);
        let old = SelectObject(mem, HGDIOBJ(bmp.0));

        let blt = BitBlt(mem, 0, 0, w, h, Some(screen), left, top, SRCCOPY);

        let mut bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0 as u32,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut buf = vec![0u8; (w * h * 4) as usize];
        let lines = GetDIBits(
            mem,
            bmp,
            0,
            h as u32,
            Some(buf.as_mut_ptr() as *mut c_void),
            &mut bmi,
            DIB_RGB_COLORS,
        );

        SelectObject(mem, old);
        let _ = DeleteObject(HGDIOBJ(bmp.0));
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);

        if blt.is_err() || lines == 0 {
            return None;
        }
        let bgr = bgra_to_bgr_crop(&buf, (w * 4) as usize, w as usize, h as usize, 0, 0, w as usize, h as usize);
        Some(Frame { bgr, width: w as u32, height: h as u32 })
    }
}

// ── Capture facade ───────────────────────────────────────────────────────────

enum Backend {
    Dxgi(DxgiCapture),
    Gdi,
    Closed,
}

/// Screen capture with dxcam→mss fallback, mirroring `ScreenCapture`. Construct
/// with the configured backend name; `backend()` reports the *resolved* one
/// after any fallback, exactly as the Python UI reads `self._capture.backend`.
pub struct ScreenCapture {
    backend: Backend,
    /// The resolved backend name (`"dxcam"` or `"mss"`), never the requested one.
    name: &'static str,
    region: Region,
    fps: FpsWindow,
}

impl ScreenCapture {
    /// `requested` is the config value (`"dxcam"` | `"mss"` | `"wgc"`); anything
    /// unrecognized falls through to GDI, like Python's `else` branch.
    pub fn new(requested: &str, region: Region) -> Self {
        match requested.to_ascii_lowercase().as_str() {
            "dxcam" => match DxgiCapture::new() {
                Some(dxgi) => Self::with(Backend::Dxgi(dxgi), "dxcam", region),
                None => Self::with(Backend::Gdi, "mss", region), // fallback
            },
            // "wgc" segfaults (see module docs) and is treated as "mss"; "mss"
            // and any unknown value land here too.
            _ => Self::with(Backend::Gdi, "mss", region),
        }
    }

    fn with(backend: Backend, name: &'static str, region: Region) -> Self {
        Self { backend, name, region, fps: FpsWindow::new() }
    }

    /// Grab the current screen (or the configured region) as BGR. Returns `None`
    /// only on a hard failure with no frame ever cached; a static screen returns
    /// the last frame. Records an FPS sample on every call, like `_track_fps`.
    pub fn grab(&mut self) -> Option<Frame> {
        let t0 = Instant::now();
        let frame = match &mut self.backend {
            Backend::Dxgi(d) => d.grab(self.region),
            Backend::Gdi => grab_gdi(self.region),
            Backend::Closed => None,
        };
        self.fps.record(t0.elapsed().as_secs_f64());
        frame
    }

    /// Rolling-average read throughput (see the module note on the FPS metric).
    pub fn fps(&self) -> f64 {
        self.fps.fps()
    }

    /// The resolved backend name (`"dxcam"` | `"mss"`).
    pub fn backend(&self) -> &str {
        self.name
    }

    /// Release the capture resources (Python's `close()` / `__exit__`). Dropping
    /// the struct does the same automatically via COM refcounting.
    pub fn close(&mut self) {
        self.backend = Backend::Closed;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fps_window_averages_and_caps_at_60() {
        let mut w = FpsWindow::new();
        assert_eq!(w.fps(), 0.0); // empty → 0.0
        w.record(0.5); // 2 fps
        w.record(0.25); // 4 fps
        assert!((w.fps() - 3.0).abs() < 1e-9); // mean(2, 4)
        // Non-positive dt is ignored (Python's `if dt > 0`).
        w.record(0.0);
        w.record(-1.0);
        assert_eq!(w.samples.len(), 2);
        // Window never grows past 60 samples.
        for _ in 0..100 {
            w.record(0.1);
        }
        assert_eq!(w.samples.len(), 60);
        assert!((w.fps() - 10.0).abs() < 1e-9); // all remaining are 1/0.1
    }

    #[test]
    fn region_rect_full_screen_and_crop() {
        // None → the whole screen.
        assert_eq!(region_rect(None, 2560, 1440), (0, 0, 2560, 1440));
        // (x1,y1,x2,y2) → (left, top, width, height).
        assert_eq!(region_rect(Some((100, 50, 400, 250)), 2560, 1440), (100, 50, 300, 200));
        // Inverted / degenerate region floors the extent at 0.
        assert_eq!(region_rect(Some((400, 250, 100, 50)), 2560, 1440), (400, 250, 0, 0));
    }

    #[test]
    fn bgra_to_bgr_respects_row_pitch() {
        // 2×2 BGRA source with a padded pitch of 12 bytes (8 used + 4 pad). If
        // the copy assumed a tight 8-byte pitch, row 1 would read the padding
        // and the image would skew; this pins that it doesn't.
        let pitch = 12;
        let mut src = vec![0u8; pitch * 2];
        // (0,0) B,G,R = 1,2,3 ; (1,0) = 4,5,6
        src[0..4].copy_from_slice(&[1, 2, 3, 255]);
        src[4..8].copy_from_slice(&[4, 5, 6, 255]);
        // padding bytes 8..12 stay 0 (must be skipped)
        // (0,1) = 7,8,9 ; (1,1) = 10,11,12
        src[12..16].copy_from_slice(&[7, 8, 9, 255]);
        src[16..20].copy_from_slice(&[10, 11, 12, 255]);

        let out = bgra_to_bgr_crop(&src, pitch, 2, 2, 0, 0, 2, 2);
        assert_eq!(out, vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
    }

    #[test]
    fn bgra_to_bgr_crops_a_sub_rect() {
        // 3×2 tight BGRA source; crop the right 2×1 corner at (1,1).
        let pitch = 12; // 3 px * 4
        let mut src = vec![0u8; pitch * 2];
        for px in 0..6 {
            let base = px * 4;
            let v = (px as u8 + 1) * 10;
            src[base..base + 4].copy_from_slice(&[v, v + 1, v + 2, 255]);
        }
        // Pixel index layout: row0 = 0,1,2 ; row1 = 3,4,5. Crop left=1,top=1,w=2,h=1
        // → source pixels 4 and 5.
        let out = bgra_to_bgr_crop(&src, pitch, 3, 2, 1, 1, 2, 1);
        assert_eq!(out, vec![50, 51, 52, 60, 61, 62]);
    }

    #[test]
    fn bgra_to_bgr_clamps_oversized_region() {
        // A region larger than the source leaves the overflow black, no panic.
        let src = vec![9u8, 8, 7, 255]; // single 1×1 pixel
        let out = bgra_to_bgr_crop(&src, 4, 1, 1, 0, 0, 2, 2);
        assert_eq!(out, vec![9, 8, 7, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    }

    /// Captures the real screen, so it is ignored by default. Run explicitly with
    /// `cargo test -- --ignored dxcam_or_mss_grabs_the_primary_screen`.
    #[test]
    #[ignore = "captures the real screen"]
    fn dxcam_or_mss_grabs_the_primary_screen() {
        let (sw, sh) = unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
        let mut cap = ScreenCapture::new("dxcam", None);
        eprintln!("resolved backend: {}", cap.backend());

        let frame = cap.grab().expect("grab returned a frame");
        // Dimensions must equal the primary output; a RowPitch skew or wrong
        // crop would still be non-zero, so asserting size is the real check.
        assert_eq!(frame.width, sw as u32, "frame width");
        assert_eq!(frame.height, sh as u32, "frame height");
        assert_eq!(frame.bgr.len(), (sw * sh * 3) as usize, "buffer length");
        // A real desktop is never uniformly black.
        assert!(frame.bgr.iter().any(|&b| b != 0), "frame was entirely black");
        // fps records a sample per grab.
        assert!(cap.fps() > 0.0, "fps should be populated after a grab");
    }
}
