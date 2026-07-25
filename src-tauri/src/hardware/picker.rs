//! The interactive screen pickers, in process.
//!
//! Ports `color_sampler.py`, `region_picker.py` and `surgical_picker.py`, the
//! three sidecar modules that were pure `ctypes` Win32 with cv2 used only at the
//! very end, to convert a colour and to write a PNG. [`overlay`] supplies the
//! window and the message pump, [`preview`] the PNG/thumbnail encoding, and
//! [`bgr_to_hsv`] below reproduces `cv2.cvtColor(..., COLOR_BGR2HSV)` exactly, so
//! a sampled colour lands on the same integer HSV triple the detector matches
//! against. Nothing here crosses a process boundary any more.
//!
//! Behaviour is a 1:1 port down to the pixel offsets and the key bindings; the
//! departures are listed where they occur, and all four are cases where Python
//! reported success after a failure it had ignored.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use super::capture::{Frame, ScreenCapture};
use super::overlay::{self, Bgr, Dib, Event, Flow, Painter, Scene, Window};
use super::preview;
use super::snap;
use super::vision;

/// Brightness multiplier applied outside the selection.
const DARKEN: f32 = 0.42;

const GOLD: Bgr = (74, 168, 192);
const GRID: Bgr = (60, 60, 60);
const CROSS: Bgr = (74, 168, 192);
const WHITE: Bgr = (255, 255, 255);
/// The dim-cyan the readout lines are drawn in.
const READOUT: Bgr = (120, 200, 220);
const ZOOM_TEXT: Bgr = (100, 180, 200);
/// Dark ink on the gold DONE button.
const BUTTON_INK: Bgr = (20, 18, 12);

/// How long each overlay may stay open, matching the `pick(timeout=…)` defaults.
const COLOR_TIMEOUT: Duration = Duration::from_secs(60);
const REGION_TIMEOUT: Duration = Duration::from_secs(120);
const SURGICAL_TIMEOUT: Duration = Duration::from_secs(180);

/// Titles for the two box pickers. Both take a click as well as a drag now, and
/// a lasso nobody knows is there is a lasso nobody uses.
const REGION_HINT: &str = "Clawmation: Click an element, or drag a box (Esc cancels)";
const SURGICAL_HINT: &str = "Clawmation: Click an element, or drag a box around it";

const VK_BACK: u32 = 0x08;
const VK_RETURN: u32 = 0x0D;
const VK_ESCAPE: u32 = 0x1B;
const VK_LEFT: u32 = 0x25;
const VK_UP: u32 = 0x26;
const VK_RIGHT: u32 = 0x27;
const VK_DOWN: u32 = 0x28;
const VK_D: u32 = 0x44;
const VK_G: u32 = 0x47;

// ── cv2-exact BGR → HSV ──────────────────────────────────────────────────────

/// OpenCV's `RGB2HSV_b`: fixed-point, 12 fractional bits.
const HSV_SHIFT: i32 = 12;
const HSV_HALF: i32 = 1 << (HSV_SHIFT - 1);

/// `cvRound(255 * 2^12 / i)`, the reciprocal table saturation is scaled by.
fn sdiv_table() -> &'static [i32; 256] {
    static TABLE: OnceLock<[i32; 256]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = [0i32; 256];
        for (i, slot) in t.iter_mut().enumerate().skip(1) {
            *slot = cv_round(f64::from(255 << HSV_SHIFT) / i as f64);
        }
        t
    })
}

/// `cvRound(180 * 2^12 / (6 * i))`: hue is expressed on 0..179, and the raw
/// numerator spans six 60-degree sectors.
fn hdiv_table() -> &'static [i32; 256] {
    static TABLE: OnceLock<[i32; 256]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = [0i32; 256];
        for (i, slot) in t.iter_mut().enumerate().skip(1) {
            *slot = cv_round(f64::from(180 << HSV_SHIFT) / (6.0 * i as f64));
        }
        t
    })
}

/// `cvRound`: nearest, ties to even.
fn cv_round(v: f64) -> i32 {
    v.round_ties_even() as i32
}

/// One pixel through `cv2.cvtColor(..., cv2.COLOR_BGR2HSV)`.
///
/// Reproduced as integer arithmetic rather than converted through floats because
/// the guard thresholds the sidecar's detector compares against were derived from
/// this exact table-driven rounding; a value off by one at a boundary is a guard
/// that silently stops matching.
pub fn bgr_to_hsv(b: u8, g: u8, r: u8) -> (u8, u8, u8) {
    let (b, g, r) = (i32::from(b), i32::from(g), i32::from(r));
    let v = b.max(g).max(r);
    let vmin = b.min(g).min(r);
    let diff = v - vmin;

    // Branch-free "which channel is the max", as masks of all-ones or all-zeros.
    let vr = if v == r { -1 } else { 0 };
    let vg = if v == g { -1 } else { 0 };

    let s = (diff * sdiv_table()[v as usize] + HSV_HALF) >> HSV_SHIFT;
    let h = (vr & (g - b)) + (!vr & ((vg & (b - r + 2 * diff)) + (!vg & (r - g + 4 * diff))));
    let h = (h * hdiv_table()[diff as usize] + HSV_HALF) >> HSV_SHIFT;
    let h = if h < 0 { h + 180 } else { h };

    (h as u8, s as u8, v as u8)
}

// ── Shared geometry ──────────────────────────────────────────────────────────

/// A drag normalised to `(x, y, w, h)`, whichever way it was drawn.
fn rect_of(sx: i32, sy: i32, cx: i32, cy: i32) -> (i32, i32, i32, i32) {
    (sx.min(cx), sy.min(cy), (cx - sx).abs(), (cy - sy).abs())
}

/// The frame darkened to [`DARKEN`], the "outside the selection" backdrop.
fn darkened(frame: &Frame) -> Dib {
    let bgr: Vec<u8> = frame.bgr.iter().map(|&v| (f32::from(v) * DARKEN) as u8).collect();
    Dib::from_bgr(&bgr, frame.width, frame.height)
}

fn dib_of(frame: &Frame) -> Dib {
    Dib::from_bgr(&frame.bgr, frame.width, frame.height)
}

/// Capture once through the same backend the Python pickers hardcoded.
fn grab_screen() -> Option<Frame> {
    let mut cap = ScreenCapture::new("mss", None);
    let frame = cap.grab();
    cap.close();
    frame
}

fn unix_seconds() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

// ── Colour sampler ───────────────────────────────────────────────────────────

/// Click a point; the HSV under it (5x5 averaged) becomes the guard's range.
struct ColorScene {
    frame: Frame,
    bits: Dib,
    mx: i32,
    my: i32,
    picked: Option<Value>,
}

impl ColorScene {
    fn new(frame: Frame) -> Self {
        let bits = dib_of(&frame);
        Self { frame, bits, mx: 0, my: 0, picked: None }
    }

    /// HSV at `(x, y)`, averaged over the surrounding 5x5 so a single stray
    /// subpixel on an anti-aliased edge cannot define the whole guard.
    fn sample(&mut self, x: i32, y: i32) {
        let (w, h) = (self.frame.width as i32, self.frame.height as i32);
        let x = x.clamp(2, w - 3);
        let y = y.clamp(2, h - 3);

        let (mut hs, mut ss, mut vs) = (0u32, 0u32, 0u32);
        for row in y - 2..=y + 2 {
            for col in x - 2..=x + 2 {
                let i = (row as usize * w as usize + col as usize) * 3;
                let px = &self.frame.bgr[i..i + 3];
                let (ph, ps, pv) = bgr_to_hsv(px[0], px[1], px[2]);
                hs += u32::from(ph);
                ss += u32::from(ps);
                vs += u32::from(pv);
            }
        }
        // `int(np.mean(...))` truncates, and every term is non-negative.
        let (h_avg, s_avg, v_avg) = ((hs / 25) as i32, (ss / 25) as i32, (vs / 25) as i32);

        let i = (y as usize * w as usize + x as usize) * 3;
        let (b, g, r) = (self.frame.bgr[i], self.frame.bgr[i + 1], self.frame.bgr[i + 2]);

        let (h_tol, s_tol, v_tol) = (10, 30, 30);
        self.picked = Some(json!({
            "x": x,
            "y": y,
            "hsv": [h_avg, s_avg, v_avg],
            "hsv_low": [
                (h_avg - h_tol).max(0),
                (s_avg - s_tol).max(0),
                (v_avg - v_tol).max(0),
            ],
            "hsv_high": [
                (h_avg + h_tol).min(179),
                (s_avg + s_tol).min(255),
                (v_avg + v_tol).min(255),
            ],
            "bgr": [b, g, r],
            "hex": format!("#{r:02x}{g:02x}{b:02x}"),
        }));
    }
}

impl Scene for ColorScene {
    type Output = Value;

    fn event(&mut self, ev: Event, _win: &Window) -> Flow {
        match ev {
            Event::MouseMove { x, y } => {
                self.mx = x;
                self.my = y;
                Flow::Repaint
            }
            Event::LeftDown { x, y } => {
                self.sample(x, y);
                Flow::Close
            }
            Event::Key { vk: VK_ESCAPE, .. } => Flow::Close,
            _ => Flow::Idle,
        }
    }

    fn paint(&mut self, p: &Painter, _sw: i32, _sh: i32) {
        p.image(&self.bits, 0, 0);
        p.lines(
            &[
                (self.mx - 15, self.my, self.mx + 15, self.my),
                (self.mx, self.my - 15, self.mx, self.my + 15),
            ],
            WHITE,
            1,
        );
        p.text(20, 20, "Click to sample color · Esc to cancel", WHITE);
    }

    fn take(&mut self) -> Option<Value> {
        self.picked.take()
    }
}

// ── Region picker ────────────────────────────────────────────────────────────

/// Drag a rectangle; everything outside it stays dimmed.
struct RegionScene {
    frame: Frame,
    dark: Dib,
    dragging: bool,
    sx: i32,
    sy: i32,
    cx: i32,
    cy: i32,
    /// The element under the cursor, offered in place of a drawn box.
    hover: snap::Hover,
    picked: Option<(i32, i32, i32, i32)>,
}

impl RegionScene {
    fn new(frame: Frame) -> Self {
        let dark = darkened(&frame);
        Self {
            frame,
            dark,
            dragging: false,
            sx: 0,
            sy: 0,
            cx: 0,
            cy: 0,
            hover: snap::Hover::default(),
            picked: None,
        }
    }

    fn rect(&self) -> (i32, i32, i32, i32) {
        rect_of(self.sx, self.sy, self.cx, self.cy)
    }
}

impl Scene for RegionScene {
    type Output = (i32, i32, i32, i32);

    fn event(&mut self, ev: Event, _win: &Window) -> Flow {
        match ev {
            Event::LeftDown { x, y } => {
                self.sx = x;
                self.cx = x;
                self.sy = y;
                self.cy = y;
                self.dragging = true;
                // Cheap where the cursor was already hovering; this is only real
                // work when the press is the first the overlay has heard of the
                // cursor, which is exactly when a click still has to snap.
                self.hover.aim(&self.frame, x, y);
                Flow::Repaint
            }
            Event::MouseMove { x, y } if self.dragging => {
                self.cx = x;
                self.cy = y;
                Flow::Repaint
            }
            Event::MouseMove { x, y } => {
                if self.hover.aim(&self.frame, x, y) {
                    Flow::Repaint
                } else {
                    Flow::Idle
                }
            }
            Event::LeftUp { x, y } if self.dragging => {
                self.dragging = false;
                self.cx = x;
                self.cy = y;
                let (rx, ry, w, h) = self.rect();
                if w >= 8 && h >= 8 {
                    self.picked = Some((rx, ry, w, h));
                    return Flow::Close;
                }
                // Too small to be a box the user meant to draw, so it was a
                // click: take the lasso's offer if it has one.
                match self.hover.rect() {
                    Some(r) => {
                        self.picked = Some(r);
                        Flow::Close
                    }
                    None => Flow::Repaint,
                }
            }
            Event::Key { vk: VK_ESCAPE, .. } => Flow::Close,
            _ => Flow::Idle,
        }
    }

    fn paint(&mut self, p: &Painter, _sw: i32, _sh: i32) {
        p.image(&self.dark, 0, 0);
        // Mid-drag the box being drawn wins; otherwise the lasso shows what a
        // click would take. Both are drawn the same, because they are the same
        // offer, undarkened, so the pixels can be read against the backdrop.
        let shown = match self.rect() {
            (x, y, w, h) if self.dragging && w > 0 && h > 0 => Some((x, y, w, h)),
            // A press that has not travelled yet is still a click in progress,
            // so the offer stays up instead of blinking off under the cursor.
            _ => self.hover.rect(),
        };
        let Some((x, y, w, h)) = shown else { return };
        if let Some(crop) = self.frame.crop(x, y, w, h) {
            p.image(&dib_of(&crop), x, y);
        }
        p.frame_rect(x, y, x + w, y + h, GOLD, 2);
    }

    fn take(&mut self) -> Option<Self::Output> {
        self.picked.take()
    }
}

// ── Surgical picker ──────────────────────────────────────────────────────────

#[derive(PartialEq, Eq, Clone, Copy)]
enum Mode {
    /// Drawing the box from scratch.
    Drag,
    /// The box exists; corners resize it, the inside moves it, arrows nudge it.
    Adjust,
}

/// Which corner a resize drag has hold of.
const CORNERS: usize = 4;

/// `(0=TL, 1=TR, 2=BL, 3=BR)` if the cursor is within `margin` of a corner.
fn hit_corner(rect: (i32, i32, i32, i32), cx: i32, cy: i32, margin: i32) -> i32 {
    let (x, y, w, h) = rect;
    let corners = [(x, y), (x + w, y), (x, y + h), (x + w, y + h)];
    for i in 0..CORNERS {
        let (hx, hy) = corners[i];
        if (cx - hx).abs() <= margin && (cy - hy).abs() <= margin {
            return i as i32;
        }
    }
    -1
}

/// Keep the selection at least 6x6 and entirely inside the frame.
fn clamped(rect: (i32, i32, i32, i32), fw: i32, fh: i32) -> (i32, i32, i32, i32) {
    let (x, y, w, h) = rect;
    let w = w.min(fw).max(6);
    let h = h.min(fh).max(6);
    (x.clamp(0, fw - w), y.clamp(0, fh - h), w, h)
}

/// Magnification that fits a `w x h` crop into ~72% of the screen, 4x..48x.
fn zoom_for(w: i32, h: i32, sw: i32, sh: i32) -> i32 {
    let max_w = (f64::from(sw) * 0.72) as i32;
    let max_h = (f64::from(sh) * 0.72) as i32;
    (max_w / w.max(1)).min(max_h / h.max(1)).max(4).min(48)
}

/// Screen cursor to a `(col, row)` offset inside the zoomed crop.
fn offset_at(
    mx: i32,
    my: i32,
    ox: i32,
    oy: i32,
    zoom: i32,
    cw: i32,
    ch: i32,
) -> Option<(i32, i32)> {
    if mx < 0 {
        return None;
    }
    // Floor division: truncation would fold the row just above the crop onto its
    // first row, letting a click land one pixel outside the template.
    let px = (mx - ox).div_euclid(zoom);
    let py = (my - oy).div_euclid(zoom);
    (px >= 0 && px < cw && py >= 0 && py < ch).then_some((px, py))
}

/// Nearest-neighbour magnification. `zoom` is an integer factor, so cv2's
/// `INTER_NEAREST` (`src = floor(dst / factor)`) is exact pixel replication.
fn magnified(crop: &Frame, zoom: i32) -> Dib {
    let (cw, ch, z) = (crop.width as usize, crop.height as usize, zoom as usize);
    let mut bgr = Vec::with_capacity(cw * ch * z * z * 3);
    for row in 0..ch * z {
        let src = (row / z) * cw * 3;
        for col in 0..cw * z {
            let i = src + (col / z) * 3;
            bgr.extend_from_slice(&crop.bgr[i..i + 3]);
        }
    }
    Dib::from_bgr(&bgr, (cw * z) as u32, (ch * z) as u32)
}

/// Two phases: box the element, then draw the exact pixels a click must land on.
struct SurgicalScene {
    frame: Frame,
    dark: Dib,
    w: i32,
    h: i32,

    mode: Mode,
    dragging: bool,
    /// The element under the cursor while the box is still being drawn.
    hover: snap::Hover,
    moving: bool,
    move_dx: i32,
    move_dy: i32,
    resize_corner: i32,
    sx: i32,
    sy: i32,
    cx: i32,
    cy: i32,

    /// `Some` once phase 2 is entered; the crop and everything derived from it.
    target: Option<Target>,
    mx: i32,
    my: i32,
    strokes: Vec<[i32; 4]>,
    drawing: bool,
    line_start: Option<(i32, i32)>,
    line_end: Option<(i32, i32)>,
    /// Recomputed on every paint, then hit-tested on the next click.
    done_rect: Option<(i32, i32, i32, i32)>,
    show_grid: bool,

    picked: Option<(i32, i32, i32, i32, Vec<[i32; 4]>)>,
}

/// Phase-2 state: the chosen crop and its on-screen magnification.
struct Target {
    crop: Frame,
    rect: (i32, i32, i32, i32),
    zoom: i32,
    disp_w: i32,
    disp_h: i32,
    ox: i32,
    oy: i32,
    bits: Dib,
}

impl SurgicalScene {
    fn new(frame: Frame) -> Self {
        let dark = darkened(&frame);
        let (w, h) = (frame.width as i32, frame.height as i32);
        Self {
            frame,
            dark,
            w,
            h,
            mode: Mode::Drag,
            dragging: false,
            hover: snap::Hover::default(),
            moving: false,
            move_dx: 0,
            move_dy: 0,
            resize_corner: -1,
            sx: 0,
            sy: 0,
            cx: 0,
            cy: 0,
            target: None,
            mx: -1,
            my: -1,
            strokes: Vec::new(),
            drawing: false,
            line_start: None,
            line_end: None,
            done_rect: None,
            show_grid: true,
            picked: None,
        }
    }

    fn rect(&self) -> (i32, i32, i32, i32) {
        rect_of(self.sx, self.sy, self.cx, self.cy)
    }

    fn set_rect(&mut self, (x, y, w, h): (i32, i32, i32, i32)) {
        self.sx = x;
        self.sy = y;
        self.cx = x + w;
        self.cy = y + h;
    }

    fn clamp_selection(&mut self) {
        let r = clamped(self.rect(), self.w, self.h);
        self.set_rect(r);
    }

    fn nudge(&mut self, dx: i32, dy: i32) {
        let (x, y, w, h) = self.rect();
        self.set_rect(((x + dx).clamp(0, self.w - w), (y + dy).clamp(0, self.h - h), w, h));
    }

    fn enter_target_phase(&mut self, win: &Window) {
        let (x, y, w, h) = self.rect();
        let x = x.clamp(0, self.w - 1);
        let y = y.clamp(0, self.h - 1);
        let (w, h) = (w.min(self.w - x), h.min(self.h - y));
        let Some(crop) = self.frame.crop(x, y, w, h) else { return };

        let (sw, sh) = overlay::screen_size();
        let zoom = zoom_for(w, h, sw, sh);
        self.target = Some(Target {
            bits: magnified(&crop, zoom),
            crop,
            rect: (x, y, w, h),
            zoom,
            disp_w: w * zoom,
            disp_h: h * zoom,
            ox: (sw - w * zoom) / 2,
            oy: (sh - h * zoom) / 2,
        });
        win.set_title("Clawmation: DRAG to draw the exact pixels to hit (Esc cancels)");
    }

    fn back_to_adjust(&mut self, win: &Window) {
        self.mode = Mode::Adjust;
        self.strokes.clear();
        self.drawing = false;
        self.line_start = None;
        self.line_end = None;
        self.target = None;
        win.set_title("Clawmation: Enter to use it (Esc cancels)");
    }

    fn rebuild_zoom(&mut self) {
        let (sw, sh) = overlay::screen_size();
        if let Some(t) = &mut self.target {
            t.disp_w = t.crop.width as i32 * t.zoom;
            t.disp_h = t.crop.height as i32 * t.zoom;
            t.ox = (sw - t.disp_w) / 2;
            t.oy = (sh - t.disp_h) / 2;
            t.bits = magnified(&t.crop, t.zoom);
        }
    }

    fn cursor_offset(&self) -> Option<(i32, i32)> {
        let t = self.target.as_ref()?;
        offset_at(
            self.mx,
            self.my,
            t.ox,
            t.oy,
            t.zoom,
            t.crop.width as i32,
            t.crop.height as i32,
        )
    }

    /// Close on `rect`, with whatever strokes have been drawn on it.
    ///
    /// Nothing drawn means the click lands on the middle of the element, which
    /// is what picking one already says: the lasso found the bounds, and the
    /// centre of a button is the part of it you press. Downstream still gets a
    /// stroke list (a single degenerate one) because `click_lines` is the
    /// contract the template carries.
    fn commit_rect(&mut self, (x, y, w, h): (i32, i32, i32, i32)) -> Flow {
        if w < 1 || h < 1 {
            return Flow::Idle;
        }
        let mut strokes = std::mem::take(&mut self.strokes);
        if strokes.is_empty() {
            let (mx, my) = (w / 2, h / 2);
            strokes.push([mx, my, mx, my]);
        }
        self.picked = Some((x, y, w, h, strokes));
        Flow::Close
    }

    /// Phase-2 commit: the crop is already fixed, so only the strokes are new.
    fn commit(&mut self) -> Flow {
        let Some(t) = &self.target else { return Flow::Idle };
        self.commit_rect(t.rect)
    }

    // ── Phase 1 ──────────────────────────────────────────────────────────────

    fn select_event(&mut self, ev: Event, win: &Window) -> Flow {
        match ev {
            Event::LeftDown { x, y } => {
                if self.mode == Mode::Drag {
                    self.sx = x;
                    self.cx = x;
                    self.sy = y;
                    self.cy = y;
                    self.dragging = true;
                    self.hover.aim(&self.frame, x, y);
                } else {
                    let corner = hit_corner(self.rect(), x, y, 10);
                    if corner >= 0 {
                        self.resize_corner = corner;
                        self.dragging = true;
                    } else {
                        let (rx, ry, w, h) = self.rect();
                        if x >= rx && x <= rx + w && y >= ry && y <= ry + h {
                            self.moving = true;
                            self.move_dx = x - rx;
                            self.move_dy = y - ry;
                        }
                    }
                }
                Flow::Repaint
            }

            Event::MouseMove { x, y } => {
                if self.mode == Mode::Drag {
                    if !self.dragging {
                        return if self.hover.aim(&self.frame, x, y) {
                            Flow::Repaint
                        } else {
                            Flow::Idle
                        };
                    }
                    self.cx = x;
                    self.cy = y;
                } else if self.resize_corner >= 0 && self.dragging {
                    // The corner opposite the one being dragged stays put.
                    let (rx, ry, w, h) = self.rect();
                    let (sx, sy, cx, cy) = match self.resize_corner {
                        0 => (x, y, rx + w, ry + h),
                        1 => (rx, y, x, ry + h),
                        2 => (x, ry, rx + w, y),
                        _ => (rx, ry, x, y),
                    };
                    self.sx = sx;
                    self.sy = sy;
                    self.cx = cx;
                    self.cy = cy;
                    self.clamp_selection();
                } else if self.moving {
                    let (_, _, w, h) = self.rect();
                    let nx = (x - self.move_dx).clamp(0, self.w - w);
                    let ny = (y - self.move_dy).clamp(0, self.h - h);
                    self.set_rect((nx, ny, w, h));
                } else {
                    return Flow::Idle;
                }
                Flow::Repaint
            }

            Event::LeftUp { x, y } => {
                if self.mode == Mode::Drag {
                    if !self.dragging {
                        return Flow::Idle;
                    }
                    self.dragging = false;
                    self.cx = x;
                    self.cy = y;
                    let (_, _, w, h) = self.rect();
                    // Too small to be a box the user meant to draw, so it was a
                    // click: take the lasso's offer if it has one, and land in
                    // Adjust either way so the snap can still be corrected.
                    if w < 6 || h < 6 {
                        match self.hover.rect() {
                            Some(r) => self.set_rect(r),
                            None => return Flow::Repaint,
                        }
                    }
                    self.mode = Mode::Adjust;
                    self.clamp_selection();
                    win.set_title("Clawmation: Enter to use it (Esc cancels)");
                } else {
                    self.dragging = false;
                    self.resize_corner = -1;
                    self.moving = false;
                    self.clamp_selection();
                }
                Flow::Repaint
            }

            Event::RightDown { .. } => {
                if self.mode != Mode::Adjust {
                    return Flow::Idle;
                }
                self.mode = Mode::Drag;
                self.dragging = false;
                self.resize_corner = -1;
                self.moving = false;
                // Nothing has been aimed at since the box was drawn; leaving the
                // old outline up would offer a box the cursor is no longer on.
                self.hover.clear();
                win.set_title(SURGICAL_HINT);
                Flow::Repaint
            }

            Event::Key { vk, shift } => {
                if vk == VK_ESCAPE {
                    return Flow::Close;
                }
                if self.mode != Mode::Adjust {
                    return Flow::Idle;
                }
                if vk == VK_RETURN {
                    let r = self.rect();
                    if r.2 >= 6 && r.3 >= 6 {
                        return self.commit_rect(r);
                    }
                    return Flow::Idle;
                }
                // The pixel editor is no longer on the way out: the middle of
                // the element is the click point unless someone says otherwise,
                // and this is how they say otherwise.
                if vk == VK_D {
                    let (_, _, w, h) = self.rect();
                    if w >= 6 && h >= 6 {
                        self.enter_target_phase(win);
                        return Flow::Repaint;
                    }
                    return Flow::Idle;
                }
                let step = if shift { 10 } else { 1 };
                match vk {
                    VK_LEFT => self.nudge(-step, 0),
                    VK_RIGHT => self.nudge(step, 0),
                    VK_UP => self.nudge(0, -step),
                    VK_DOWN => self.nudge(0, step),
                    _ => return Flow::Idle,
                }
                Flow::Repaint
            }

            Event::Wheel { .. } => Flow::Idle,
        }
    }

    // ── Phase 2 ──────────────────────────────────────────────────────────────

    fn target_event(&mut self, ev: Event, win: &Window) -> Flow {
        match ev {
            Event::LeftDown { x, y } => {
                if let Some((bx0, by0, bx1, by1)) = self.done_rect {
                    if x >= bx0 && x <= bx1 && y >= by0 && y <= by1 {
                        return self.commit();
                    }
                }
                // The stroke starts under the *tracked* cursor, not the click
                // point, so a click and a drag begin on the same pixel.
                if let Some(off) = self.cursor_offset() {
                    self.drawing = true;
                    self.line_start = Some(off);
                    self.line_end = Some(off);
                    return Flow::Repaint;
                }
                Flow::Idle
            }

            Event::MouseMove { x, y } => {
                self.mx = x;
                self.my = y;
                if self.drawing {
                    if let Some(off) = self.cursor_offset() {
                        self.line_end = Some(off);
                    }
                }
                Flow::Repaint
            }

            Event::LeftUp { .. } => {
                if !self.drawing {
                    return Flow::Idle;
                }
                self.drawing = false;
                if let Some(off) = self.cursor_offset() {
                    self.line_end = Some(off);
                }
                if let Some(start) = self.line_start {
                    let end = self.line_end.unwrap_or(start);
                    self.strokes.push([start.0, start.1, end.0, end.1]);
                }
                self.line_start = None;
                self.line_end = None;
                Flow::Repaint
            }

            Event::RightDown { .. } => {
                if self.drawing {
                    self.drawing = false;
                    self.line_start = None;
                    self.line_end = None;
                } else {
                    self.strokes.pop();
                }
                Flow::Repaint
            }

            Event::Key { vk, .. } => match vk {
                VK_ESCAPE => Flow::Close,
                VK_RETURN => self.commit(),
                VK_BACK => {
                    if self.strokes.pop().is_none() {
                        self.back_to_adjust(win);
                    }
                    Flow::Repaint
                }
                VK_G => {
                    self.show_grid = !self.show_grid;
                    Flow::Repaint
                }
                _ => Flow::Idle,
            },

            Event::Wheel { delta } => {
                let Some(t) = &mut self.target else { return Flow::Idle };
                let step = (t.zoom / 4).max(1);
                t.zoom = if delta > 0 { (t.zoom + step).min(48) } else { (t.zoom - step).max(4) };
                self.rebuild_zoom();
                Flow::Repaint
            }
        }
    }

    // ── Painting ─────────────────────────────────────────────────────────────

    fn paint_select(&mut self, p: &Painter, sw: i32, _sh: i32) {
        p.image(&self.dark, 0, 0);
        let (x, y, w, h) = self.rect();
        let drawn = (self.dragging && w > 0 && h > 0) || self.mode == Mode::Adjust;
        // Before there is a box, the lasso shows what a click would take.
        let shown = if drawn { Some((x, y, w, h)) } else { self.hover.rect() };
        let Some((x, y, w, h)) = shown else { return };
        if let Some(crop) = self.frame.crop(x, y, w, h) {
            p.image(&dib_of(&crop), x, y);
        }
        p.frame_rect(x, y, x + w, y + h, GOLD, 2);

        if self.mode != Mode::Adjust {
            return;
        }
        let hs = 5;
        for (hx, hy) in [(x, y), (x + w, y), (x, y + h), (x + w, y + h)] {
            p.filled_rect(hx - hs, hy - hs, hx + hs, hy + hs, GOLD);
        }
        p.text(x, y + h + 6, &format!("{w} x {h}"), READOUT);
        p.text_centered(
            sw / 2,
            10,
            "Enter = use it   arrows = nudge (Shift=10)   right-click = redo   D = pick exact pixels   Esc = cancel",
            READOUT,
        );
    }

    fn paint_target(&mut self, p: &Painter, sw: i32, _sh: i32) {
        p.image(&self.dark, 0, 0);
        let Some(t) = &self.target else { return };
        let (ox, oy, zoom, disp_w, disp_h) = (t.ox, t.oy, t.zoom, t.disp_w, t.disp_h);
        let (cw, ch) = (t.crop.width as i32, t.crop.height as i32);

        p.image(&t.bits, ox, oy);

        if self.show_grid && zoom >= 8 {
            let mut segs = Vec::with_capacity((cw + ch + 2) as usize);
            for i in 0..=cw {
                segs.push((ox + i * zoom, oy, ox + i * zoom, oy + disp_h));
            }
            for j in 0..=ch {
                segs.push((ox, oy + j * zoom, ox + disp_w, oy + j * zoom));
            }
            p.lines(&segs, GRID, 1);
        }

        p.frame_rect(ox, oy, ox + disp_w, oy + disp_h, GOLD, 2);

        // Every committed stroke, plus the one being dragged, as a thick line
        // between two pixel centres with an outlined square on each end.
        let pen = (zoom / 3).max(2);
        let r = (zoom / 2).max(3);
        let centre = |cx: i32, cy: i32| (ox + cx * zoom + zoom / 2, oy + cy * zoom + zoom / 2);
        let live = self
            .drawing
            .then(|| self.line_start.map(|s| [s.0, s.1, self.line_end.unwrap_or(s).0, self.line_end.unwrap_or(s).1]))
            .flatten();
        for s in self.strokes.iter().copied().chain(live) {
            let (x1, y1) = centre(s[0], s[1]);
            let (x2, y2) = centre(s[2], s[3]);
            p.line(x1, y1, x2, y2, CROSS, pen);
            p.frame_rect(x1 - r, y1 - r, x1 + r, y1 + r, WHITE, 2);
            p.frame_rect(x2 - r, y2 - r, x2 + r, y2 + r, WHITE, 2);
        }

        if !self.drawing {
            if let Some((px, py)) = self.cursor_offset() {
                let (midx, midy) = centre(px, py);
                p.lines(
                    &[(midx, oy, midx, oy + disp_h), (ox, midy, ox + disp_w, midy)],
                    CROSS,
                    1,
                );
                p.text(midx + 10, midy - 18, &format!("{px},{py}"), WHITE);
            }
        }

        p.text(ox + disp_w - 36, oy + disp_h + 4, &format!("{zoom}x"), ZOOM_TEXT);
        self.paint_label(p, sw, oy, disp_h, cw, ch);
    }

    /// The instruction line above the crop and, once a stroke exists, the DONE
    /// button below it.
    fn paint_label(&mut self, p: &Painter, sw: i32, oy: i32, disp_h: i32, cw: i32, ch: i32) {
        let n = self.strokes.len();
        let text = if self.drawing {
            format!("drawing stroke {}...   release to add it", n + 1)
        } else if n == 0 {
            format!("crop {cw}x{ch}   click = point  ·  drag = line  ·  scroll = zoom  ·  G = grid  ·  Enter = use the middle  ·  Backspace = back")
        } else {
            let plural = if n == 1 { "" } else { "s" };
            format!("{n} stroke{plural}   right-click/Backspace = undo  ·  Enter = done  ·  Esc = cancel")
        };
        p.text_centered(sw / 2, (oy - 26).max(8), &text, READOUT);

        self.done_rect = None;
        if n == 0 || self.drawing {
            return;
        }
        let label = "DONE  (Enter)";
        let (tw, th) = p.text_size(label);
        let (bw, bh) = (tw + 28, 34);
        let bx = (sw - bw) / 2;
        let by = oy + disp_h + 18;
        self.done_rect = Some((bx, by, bx + bw, by + bh));
        p.filled_rect(bx, by, bx + bw, by + bh, GOLD);
        p.text(bx + 14, by + (bh - th) / 2, label, BUTTON_INK);
    }
}

impl Scene for SurgicalScene {
    type Output = (i32, i32, i32, i32, Vec<[i32; 4]>);

    fn event(&mut self, ev: Event, win: &Window) -> Flow {
        if self.target.is_none() {
            self.select_event(ev, win)
        } else {
            self.target_event(ev, win)
        }
    }

    fn paint(&mut self, p: &Painter, sw: i32, sh: i32) {
        if self.target.is_none() {
            self.paint_select(p, sw, sh);
        } else {
            self.paint_target(p, sw, sh);
        }
    }

    fn take(&mut self) -> Option<Self::Output> {
        self.picked.take()
    }
}

// ── Entry points ─────────────────────────────────────────────────────────────
//
// Each returns the result dict the guard/trigger editor already renders, so the
// frontend contract is unchanged. One departure throughout: Python folded a
// failed screen capture into `{"ok": false, "error": "cancelled"}`, which the
// editor silently swallows; a capture that could not happen is reported as
// itself, and so is a template that could not be written to disk.

fn cancelled() -> Value {
    json!({ "ok": false, "error": "cancelled" })
}

fn no_capture() -> Value {
    json!({ "ok": false, "error": "Could not capture screen" })
}

/// Click a point on screen; returns its HSV range, BGR and hex.
pub fn sample_color() -> Value {
    let Some(frame) = grab_screen() else { return no_capture() };
    match overlay::run(
        "ClawmationColorSampler",
        "Clawmation: Pick Color",
        ColorScene::new(frame),
        COLOR_TIMEOUT,
    ) {
        Some(Value::Object(map)) => {
            let mut out = serde_json::Map::new();
            out.insert("ok".into(), Value::Bool(true));
            out.extend(map);
            Value::Object(out)
        }
        _ => cancelled(),
    }
}

/// Drag a rectangle; returns it as `{x, y, w, h}` clamped to the screen.
pub fn pick_region() -> Value {
    let Some(frame) = grab_screen() else { return no_capture() };
    let (fw, fh) = (frame.width as i32, frame.height as i32);
    match run_region(frame) {
        None => cancelled(),
        Some((x, y, w, h)) => {
            let (x, y) = (x.clamp(0, fw - 1), y.clamp(0, fh - 1));
            json!({ "ok": true, "x": x, "y": y, "w": w.min(fw - x), "h": h.min(fh - y) })
        }
    }
}

fn run_region(frame: Frame) -> Option<(i32, i32, i32, i32)> {
    overlay::run("ClawmationRegionPicker", REGION_HINT, RegionScene::new(frame), REGION_TIMEOUT)
}

/// Drag a rectangle and save it as a template PNG.
pub fn capture_template(templates_dir: &Path) -> Value {
    let Some(frame) = grab_screen() else { return no_capture() };
    let (fw, fh) = (frame.width as i32, frame.height as i32);
    let Some((x, y, w, h)) = run_region(frame.clone()) else { return cancelled() };

    let (x, y) = (x.clamp(0, fw - 1), y.clamp(0, fh - 1));
    let (w, h) = (w.min(fw - x), h.min(fh - y));
    if w < 8 || h < 8 {
        return json!({ "ok": false, "error": "Selection too small" });
    }
    let Some(crop) = frame.crop(x, y, w, h) else {
        return json!({ "ok": false, "error": "Selection too small" });
    };

    match write_template(templates_dir, "template", &crop) {
        Err(e) => json!({ "ok": false, "error": e }),
        Ok(path) => json!({
            "ok": true,
            "path": path.display().to_string(),
            "w": w,
            "h": h,
            "thumb": preview::thumbnail(&crop, 64),
        }),
    }
}

/// Box an element, then draw the exact pixels a click must land on.
pub fn surgical_capture(templates_dir: &Path) -> Value {
    let Some(frame) = grab_screen() else { return no_capture() };
    let picked = overlay::run(
        "ClawmationSurgicalPicker",
        SURGICAL_HINT,
        SurgicalScene::new(frame.clone()),
        SURGICAL_TIMEOUT,
    );
    let Some((x, y, w, h, strokes)) = picked else { return cancelled() };
    let Some(crop) = frame.crop(x, y, w, h) else {
        return json!({ "ok": false, "error": "Empty surgical crop" });
    };

    let path = match write_template(templates_dir, "surgical", &crop) {
        Err(e) => return json!({ "ok": false, "error": e }),
        Ok(p) => p,
    };
    let first = strokes[0];
    json!({
        "ok": true,
        "path": path.display().to_string(),
        "click_lines": strokes,
        "click_line": first,
        "offset": [first[0], first[1]],
        "w": w,
        "h": h,
        "thumb": preview::stroke_thumbnail(&crop, 96, &strokes),
    })
}

/// Import an image file the user already has as a template, the compute half of
/// `_add_template_image`, with the file dialog left to the caller.
///
/// Whatever format the source was, it is re-encoded to PNG, so everything under
/// `templates/` decodes the same way. Anything under 8px on a side is refused:
/// the matcher's scale sweep has nothing to work with, and a stray icon picked by
/// accident is the usual cause.
pub fn import_template(templates_dir: &Path, file: &Path) -> Value {
    let image = match vision::read_frame(file) {
        Ok(f) => f,
        Err(e) => return json!({ "ok": false, "error": e.to_string() }),
    };
    let (w, h) = (image.width, image.height);
    if w < 8 || h < 8 {
        return json!({ "ok": false, "error": "Image too small" });
    }
    match write_template(templates_dir, "template", &image) {
        Err(e) => json!({ "ok": false, "error": e }),
        Ok(path) => json!({
            "ok": true,
            "path": path.display().to_string(),
            "w": w,
            "h": h,
            "thumb": preview::thumbnail(&image, 64),
        }),
    }
}

/// Save `crop` as `<prefix>_<unix>_<w>x<h>.png` under `dir`.
fn write_template(dir: &Path, prefix: &str, crop: &Frame) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
    let name = format!("{prefix}_{}_{}x{}.png", unix_seconds(), crop.width, crop.height);
    let path = dir.join(name);
    preview::save_png(&path, crop)
        .map(|()| path)
        .map_err(|e| format!("Could not save template: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The six corners of the RGB cube, as cv2 reports them.
    #[test]
    fn hsv_matches_opencv_on_the_primaries() {
        assert_eq!(bgr_to_hsv(0, 0, 255), (0, 255, 255)); // red
        assert_eq!(bgr_to_hsv(0, 255, 0), (60, 255, 255)); // green
        assert_eq!(bgr_to_hsv(255, 0, 0), (120, 255, 255)); // blue
        assert_eq!(bgr_to_hsv(0, 255, 255), (30, 255, 255)); // yellow
        assert_eq!(bgr_to_hsv(255, 255, 0), (90, 255, 255)); // cyan
        assert_eq!(bgr_to_hsv(255, 0, 255), (150, 255, 255)); // magenta
    }

    #[test]
    fn hsv_greys_have_no_hue_and_no_saturation() {
        assert_eq!(bgr_to_hsv(0, 0, 0), (0, 0, 0));
        assert_eq!(bgr_to_hsv(128, 128, 128), (0, 0, 128));
        assert_eq!(bgr_to_hsv(255, 255, 255), (0, 0, 255));
    }

    #[test]
    fn hsv_hue_wraps_below_red_instead_of_going_negative() {
        // Reds leaning magenta put the raw hue just under zero; cv2 adds 180.
        assert_eq!(bgr_to_hsv(5, 0, 255).0, 179);
        assert_eq!(bgr_to_hsv(40, 0, 255).0, 175);
        // 0..179 is the whole range 8-bit hue is allowed to take.
        for b in 0..=255u8 {
            for g in [0u8, 77, 200, 255] {
                assert!(bgr_to_hsv(b, g, 255).0 < 180);
            }
        }
    }

    #[test]
    fn hsv_tables_are_the_opencv_reciprocals() {
        assert_eq!(sdiv_table()[0], 0); // black divides by nothing
        assert_eq!(sdiv_table()[255], 4096); // 255<<12 / 255
        assert_eq!(hdiv_table()[0], 0);
        assert_eq!(hdiv_table()[255], 482); // cvRound(180<<12 / (6*255))
    }

    #[test]
    fn drag_normalises_whichever_way_it_was_drawn() {
        assert_eq!(rect_of(10, 10, 40, 30), (10, 10, 30, 20));
        assert_eq!(rect_of(40, 30, 10, 10), (10, 10, 30, 20));
        assert_eq!(rect_of(5, 5, 5, 5), (5, 5, 0, 0));
    }

    #[test]
    fn corner_hits_are_ordered_tl_tr_bl_br() {
        let r = (100, 100, 50, 50);
        assert_eq!(hit_corner(r, 100, 100, 10), 0);
        assert_eq!(hit_corner(r, 150, 100, 10), 1);
        assert_eq!(hit_corner(r, 100, 150, 10), 2);
        assert_eq!(hit_corner(r, 150, 150, 10), 3);
        assert_eq!(hit_corner(r, 109, 109, 10), 0); // inside the margin
        assert_eq!(hit_corner(r, 125, 125, 10), -1); // dead centre
    }

    #[test]
    fn clamping_keeps_the_box_inside_the_frame_and_at_least_six_wide() {
        assert_eq!(clamped((10, 10, 20, 20), 100, 100), (10, 10, 20, 20));
        assert_eq!(clamped((0, 0, 2, 2), 100, 100), (0, 0, 6, 6));
        assert_eq!(clamped((95, 95, 20, 20), 100, 100), (80, 80, 20, 20));
        assert_eq!(clamped((-5, -5, 20, 20), 100, 100), (0, 0, 20, 20));
        assert_eq!(clamped((0, 0, 500, 500), 100, 100), (0, 0, 100, 100));
    }

    #[test]
    fn zoom_fits_the_crop_into_most_of_the_screen() {
        assert_eq!(zoom_for(100, 100, 1920, 1080), 7); // height binds: 777/100
        assert_eq!(zoom_for(1, 1, 1920, 1080), 48); // capped
        assert_eq!(zoom_for(2000, 2000, 1920, 1080), 4); // floored
        assert_eq!(zoom_for(0, 0, 1920, 1080), 48); // degenerate, no divide by zero
    }

    #[test]
    fn cursor_maps_to_crop_pixels_and_rejects_the_surrounding_screen() {
        // 10x zoom, crop 8x8, drawn at (100, 100).
        assert_eq!(offset_at(100, 100, 100, 100, 10, 8, 8), Some((0, 0)));
        assert_eq!(offset_at(109, 100, 100, 100, 10, 8, 8), Some((0, 0)));
        assert_eq!(offset_at(110, 100, 100, 100, 10, 8, 8), Some((1, 0)));
        assert_eq!(offset_at(179, 179, 100, 100, 10, 8, 8), Some((7, 7)));
        assert_eq!(offset_at(180, 100, 100, 100, 10, 8, 8), None); // past the right edge
        // Truncating division would fold this onto row 0 instead of rejecting it.
        assert_eq!(offset_at(105, 99, 100, 100, 10, 8, 8), None);
        assert_eq!(offset_at(-1, 100, 100, 100, 10, 8, 8), None); // no cursor yet
    }

    #[test]
    fn magnification_replicates_pixels_exactly() {
        // Two pixels side by side, blown up 3x.
        let crop = Frame { bgr: vec![1, 2, 3, 4, 5, 6], width: 2, height: 1 };
        let dib = magnified(&crop, 3);
        // 6 wide, 3 tall, BGRA: first three columns are pixel 0, next three are 1.
        let row: Vec<u8> = (0..6).flat_map(|c| if c < 3 { [1, 2, 3, 255] } else { [4, 5, 6, 255] }).collect();
        let expected: Vec<u8> = row.repeat(3);
        assert_eq!(dib.bytes(), expected.as_slice());
    }

    #[test]
    fn darkening_scales_every_channel_toward_black() {
        let f = Frame { bgr: vec![0, 100, 255], width: 1, height: 1 };
        // 0.42 * 100 = 42.0, 0.42 * 255 = 107.1 -> truncated like numpy's astype.
        assert_eq!(darkened(&f).bytes(), &[0, 42, 107, 255]);
    }

    // ── Lasso, through the scenes ────────────────────────────────────────────
    // Driven as event sequences against a detached window, so the click-versus-
    // drag decision is covered without taking over the screen.

    /// A dark field with one button-coloured rectangle on it, and that
    /// rectangle, which is what the lasso is expected to return.
    fn framed_button() -> (Frame, (i32, i32, i32, i32)) {
        let (fw, fh) = (400i32, 300i32);
        let mut bgr = vec![40u8; (fw * fh * 3) as usize];
        let button = (100, 80, 120, 36);
        let (bx, by, bw, bh) = button;
        for y in by..by + bh {
            for x in bx..bx + bw {
                let i = ((y * fw + x) * 3) as usize;
                bgr[i..i + 3].copy_from_slice(&[200, 160, 90]);
            }
        }
        (Frame { bgr, width: fw as u32, height: fh as u32 }, button)
    }

    #[test]
    fn hovering_a_control_offers_it_and_a_click_takes_it() {
        let (frame, button) = framed_button();
        let mut scene = RegionScene::new(frame);
        let win = Window::detached();

        assert_eq!(scene.event(Event::MouseMove { x: 150, y: 95 }, &win), Flow::Repaint);
        assert_eq!(scene.hover.rect(), Some(button), "nothing was offered to click");

        scene.event(Event::LeftDown { x: 150, y: 95 }, &win);
        assert_eq!(scene.event(Event::LeftUp { x: 150, y: 95 }, &win), Flow::Close);
        assert_eq!(scene.take(), Some(button));
    }

    #[test]
    fn a_drawn_box_still_wins_over_the_offer() {
        let (frame, button) = framed_button();
        let mut scene = RegionScene::new(frame);
        let win = Window::detached();

        scene.event(Event::MouseMove { x: 150, y: 95 }, &win);
        scene.event(Event::LeftDown { x: 150, y: 95 }, &win);
        scene.event(Event::MouseMove { x: 260, y: 200 }, &win);
        assert_eq!(scene.event(Event::LeftUp { x: 260, y: 200 }, &win), Flow::Close);
        let picked = scene.take().expect("the drag");
        assert_eq!(picked, (150, 95, 110, 105));
        assert_ne!(picked, button);
    }

    #[test]
    fn clicking_where_there_is_no_control_commits_nothing() {
        let (frame, _) = framed_button();
        let mut scene = RegionScene::new(frame);
        let win = Window::detached();

        scene.event(Event::MouseMove { x: 20, y: 20 }, &win);
        assert_eq!(scene.hover.rect(), None);
        scene.event(Event::LeftDown { x: 20, y: 20 }, &win);
        // Repaint, not Close: the overlay stays up so the user can try again.
        assert_eq!(scene.event(Event::LeftUp { x: 20, y: 20 }, &win), Flow::Repaint);
        assert_eq!(scene.take(), None);
    }

    #[test]
    fn the_surgical_picker_snaps_a_click_straight_into_adjust() {
        let (frame, button) = framed_button();
        let mut scene = SurgicalScene::new(frame);
        let win = Window::detached();

        scene.event(Event::MouseMove { x: 150, y: 95 }, &win);
        scene.event(Event::LeftDown { x: 150, y: 95 }, &win);
        assert_eq!(scene.event(Event::LeftUp { x: 150, y: 95 }, &win), Flow::Repaint);

        // The drag phase is skipped entirely, and the box is the control's.
        assert!(scene.mode == Mode::Adjust);
        assert_eq!(scene.rect(), button);
    }

    #[test]
    fn confirming_a_snap_commits_the_middle_without_the_pixel_editor() {
        let (frame, button) = framed_button();
        let mut scene = SurgicalScene::new(frame);
        let win = Window::detached();

        scene.event(Event::MouseMove { x: 150, y: 95 }, &win);
        scene.event(Event::LeftDown { x: 150, y: 95 }, &win);
        scene.event(Event::LeftUp { x: 150, y: 95 }, &win);
        assert_eq!(scene.event(Event::Key { vk: VK_RETURN, shift: false }, &win), Flow::Close);

        assert!(scene.target.is_none(), "the pixel editor was opened anyway");
        let (x, y, w, h, strokes) = scene.take().expect("the snap");
        assert_eq!((x, y, w, h), button);
        // Crop-relative, and a point rather than a line.
        let (mx, my) = (button.2 / 2, button.3 / 2);
        assert_eq!(strokes, vec![[mx, my, mx, my]]);
    }

    #[test]
    fn d_still_opens_the_pixel_editor_for_an_off_centre_click() {
        let (frame, button) = framed_button();
        let mut scene = SurgicalScene::new(frame);
        let win = Window::detached();

        scene.event(Event::MouseMove { x: 150, y: 95 }, &win);
        scene.event(Event::LeftDown { x: 150, y: 95 }, &win);
        scene.event(Event::LeftUp { x: 150, y: 95 }, &win);
        assert_eq!(scene.event(Event::Key { vk: VK_D, shift: false }, &win), Flow::Repaint);

        let target = scene.target.as_ref().expect("the pixel editor");
        assert_eq!(target.rect, button);
    }

    // ── Live overlay ─────────────────────────────────────────────────────────
    // The tests above are pure math; these four are the only thing that proves
    // the Win32 half (window class, message pump, painting, hit handling)
    // actually runs, because they open the real overlay and drive it with the
    // app's own `SendInput` controller. They take over the screen, the cursor and
    // the keyboard, so they stay `#[ignore]`d and are run by hand with
    // `cargo test --no-default-features --lib live_ -- --ignored --test-threads=1`.

    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use crate::hardware::input::InputController;

    /// A driver thread that acts on the overlay while this thread pumps it.
    struct Driver {
        done: Arc<AtomicBool>,
        handle: std::thread::JoinHandle<()>,
    }

    impl Driver {
        /// Tell the driver the picker returned, then wait for it to stop.
        fn finish(self) {
            self.done.store(true, Ordering::SeqCst);
            self.handle.join().unwrap();
        }
    }

    /// Run `steps` once the overlay is up. If the picker has not returned a few
    /// seconds later the driver presses Esc, so a broken scene can never leave a
    /// fullscreen window sitting on the user's screen for the whole timeout.
    fn drive(steps: impl FnOnce(&InputController) + Send + 'static) -> Driver {
        let done = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&done);
        let handle = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(1200));
            let input = InputController::new();
            steps(&input);
            for _ in 0..30 {
                if flag.load(Ordering::SeqCst) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            input.key_press("escape");
        });
        Driver { done, handle }
    }

    /// Press the left button down, walk to the far corner, release: the drag
    /// both region scenes are built around. The intermediate steps are what make
    /// `WM_MOUSEMOVE` arrive while the button is held.
    fn drag(input: &InputController, x0: i32, y0: i32, x1: i32, y1: i32) {
        input.move_to(x0, y0);
        std::thread::sleep(Duration::from_millis(150));
        input.mouse_down(None, "left");
        for i in 1..=4 {
            let t = f64::from(i) / 4.0;
            let x = x0 + ((f64::from(x1 - x0)) * t).round_ties_even() as i32;
            let y = y0 + ((f64::from(y1 - y0)) * t).round_ties_even() as i32;
            input.move_to(x, y);
            std::thread::sleep(Duration::from_millis(60));
        }
        std::thread::sleep(Duration::from_millis(100));
        input.mouse_up(None, "left");
    }

    /// A directory of our own for the two template writers, so a live run never
    /// drops files into the user's templates folder.
    fn temp_templates(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("clawmation_live_{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    #[ignore = "opens a real fullscreen overlay and clicks with the user's cursor"]
    fn live_color_sampler_reports_the_clicked_pixel() {
        let (sw, sh) = overlay::screen_size();
        let (px, py) = (sw / 2, sh / 3);
        let driver = drive(move |input| {
            input.move_to(px, py);
            std::thread::sleep(Duration::from_millis(200));
            input.click(px, py, "left");
        });

        let res = sample_color();
        driver.finish();

        assert_eq!(res["ok"], true, "overlay returned {res}");
        assert_eq!(res["x"], px);
        assert_eq!(res["y"], py);
        let hex = res["hex"].as_str().expect("a #rrggbb string");
        assert_eq!(hex.len(), 7, "hex was {hex}");
        assert!(hex.starts_with('#'));
        // The sampled triple has to be the cv2 conversion of the sampled pixel's
        // neighbourhood average, so the low/high band brackets it.
        let hsv = res["hsv"].as_array().unwrap();
        let low = res["hsv_low"].as_array().unwrap();
        let high = res["hsv_high"].as_array().unwrap();
        for i in 0..3 {
            assert!(low[i].as_i64().unwrap() <= hsv[i].as_i64().unwrap());
            assert!(hsv[i].as_i64().unwrap() <= high[i].as_i64().unwrap());
        }
    }

    #[test]
    #[ignore = "opens a real fullscreen overlay and drags with the user's cursor"]
    fn live_region_picker_returns_the_dragged_rectangle() {
        let (sw, sh) = overlay::screen_size();
        let (x0, y0) = (sw / 4, sh / 4);
        let (x1, y1) = (x0 + 160, y0 + 120);
        let driver = drive(move |input| drag(input, x0, y0, x1, y1));

        let res = pick_region();
        driver.finish();

        assert_eq!(res["ok"], true, "overlay returned {res}");
        assert_eq!(res["x"], x0);
        assert_eq!(res["y"], y0);
        assert_eq!(res["w"], 160);
        assert_eq!(res["h"], 120);
    }

    #[test]
    #[ignore = "opens a real fullscreen overlay, drags, and writes a PNG"]
    fn live_capture_template_saves_the_dragged_crop() {
        let (sw, sh) = overlay::screen_size();
        let (x0, y0) = (sw / 4, sh / 4);
        let driver = drive(move |input| drag(input, x0, y0, x0 + 120, y0 + 90));

        let dir = temp_templates("template");
        let res = capture_template(&dir);
        driver.finish();

        assert_eq!(res["ok"], true, "overlay returned {res}");
        assert_eq!(res["w"], 120);
        assert_eq!(res["h"], 90);
        let path = PathBuf::from(res["path"].as_str().unwrap());
        assert!(path.exists(), "{} was not written", path.display());
        assert_eq!(path.parent(), Some(dir.as_path()));
        assert!(
            path.file_name().unwrap().to_string_lossy().ends_with("_120x90.png"),
            "unexpected name {}",
            path.display()
        );
        assert!(!res["thumb"].as_str().unwrap().is_empty(), "a base64 PNG thumbnail");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    #[ignore = "opens a real fullscreen overlay, drags, draws, and writes a PNG"]
    fn live_surgical_capture_saves_the_crop_and_the_stroke() {
        let (sw, sh) = overlay::screen_size();
        let (x0, y0) = (sw / 4, sh / 4);
        let driver = drive(move |input| {
            // Phase 1: drag the box, then Enter to accept it and zoom in.
            drag(input, x0, y0, x0 + 160, y0 + 120);
            std::thread::sleep(Duration::from_millis(300));
            input.key_press("enter");
            std::thread::sleep(Duration::from_millis(600));
            // Phase 2: the crop is magnified around the screen centre, so a short
            // drag from there lands inside it. The stroke starts under the
            // *tracked* cursor, hence the move-then-pause before pressing.
            drag(input, sw / 2, sh / 2, sw / 2 + 40, sh / 2 + 30);
            std::thread::sleep(Duration::from_millis(300));
            input.key_press("enter");
        });

        let dir = temp_templates("surgical");
        let res = surgical_capture(&dir);
        driver.finish();

        assert_eq!(res["ok"], true, "overlay returned {res}");
        assert_eq!(res["w"], 160);
        assert_eq!(res["h"], 120);
        let path = PathBuf::from(res["path"].as_str().unwrap());
        assert!(path.exists(), "{} was not written", path.display());
        assert_eq!(path.parent(), Some(dir.as_path()));

        let strokes = res["click_lines"].as_array().unwrap();
        assert_eq!(strokes.len(), 1, "one drawn stroke");
        let stroke = strokes[0].as_array().unwrap();
        for (i, v) in stroke.iter().enumerate() {
            let bound = if i % 2 == 0 { 160 } else { 120 };
            let n = v.as_i64().unwrap();
            assert!((0..bound).contains(&n), "stroke coord {n} outside the crop");
        }
        assert_eq!(res["click_line"], strokes[0]);
        assert_eq!(res["offset"], json!([stroke[0], stroke[1]]));
        assert!(!res["thumb"].as_str().unwrap().is_empty(), "a base64 PNG thumbnail");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
