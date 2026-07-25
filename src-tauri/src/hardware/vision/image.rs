//! The OpenCV operations the detector is built from, ported to Rust.
//!
//! Every function here replaces one `cv2.*` call the Python sidecar made. They
//! are transcriptions, not equivalents: the guard thresholds users have already
//! tuned were produced by OpenCV's exact arithmetic, so a colour that quantises
//! one step differently, or a contour area computed off pixel counts instead of
//! the border polygon, is a guard that silently stops firing.
//!
//! Where a transcription was not practical the departure is named at the call
//! site, with what it costs.

use std::sync::OnceLock;

/// A single-channel 8-bit image, tightly packed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gray {
    pub w: usize,
    pub h: usize,
    pub data: Vec<u8>,
}

impl Gray {
    pub fn new(w: usize, h: usize) -> Self {
        Self { w, h, data: vec![0; w * h] }
    }

    pub fn from_vec(w: usize, h: usize, data: Vec<u8>) -> Self {
        debug_assert_eq!(data.len(), w * h);
        Self { w, h, data }
    }

    #[inline]
    pub fn at(&self, x: usize, y: usize) -> u8 {
        self.data[y * self.w + x]
    }

    #[inline]
    pub fn set(&mut self, x: usize, y: usize, v: u8) {
        self.data[y * self.w + x] = v;
    }

    pub fn is_empty(&self) -> bool {
        self.w == 0 || self.h == 0
    }

    /// `cv2.countNonZero`.
    pub fn count_nonzero(&self) -> usize {
        self.data.iter().filter(|&&v| v != 0).count()
    }

    /// A tight sub-image, clamped to bounds. `None` when the rectangle is
    /// degenerate or lands entirely outside.
    pub fn crop(&self, x: i32, y: i32, w: i32, h: i32) -> Option<Gray> {
        let (fw, fh) = (self.w as i32, self.h as i32);
        let x0 = x.clamp(0, fw);
        let y0 = y.clamp(0, fh);
        let x1 = (x + w).clamp(0, fw);
        let y1 = (y + h).clamp(0, fh);
        let (cw, ch) = ((x1 - x0) as usize, (y1 - y0) as usize);
        if cw == 0 || ch == 0 {
            return None;
        }
        let mut data = Vec::with_capacity(cw * ch);
        for row in 0..ch {
            let start = (y0 as usize + row) * self.w + x0 as usize;
            data.extend_from_slice(&self.data[start..start + cw]);
        }
        Some(Gray::from_vec(cw, ch, data))
    }
}

// ── Colour conversion ────────────────────────────────────────────────────────

/// `cv2.cvtColor(..., cv2.COLOR_BGR2GRAY)`: fixed-point luma with 14 fractional
/// bits, the coefficients straight out of OpenCV's `RGB2Gray`.
const B2Y: i32 = 1868;
const G2Y: i32 = 9617;
const R2Y: i32 = 4899;
const Y_SHIFT: i32 = 14;

pub fn bgr_to_gray(bgr: &[u8], w: usize, h: usize) -> Gray {
    let mut data = Vec::with_capacity(w * h);
    for px in bgr.chunks_exact(3).take(w * h) {
        let (b, g, r) = (i32::from(px[0]), i32::from(px[1]), i32::from(px[2]));
        let y = (b * B2Y + g * G2Y + r * R2Y + (1 << (Y_SHIFT - 1))) >> Y_SHIFT;
        data.push(y as u8);
    }
    Gray::from_vec(w, h, data)
}

/// OpenCV's `RGB2HSV_b`: fixed-point, 12 fractional bits.
const HSV_SHIFT: i32 = 12;
const HSV_HALF: i32 = 1 << (HSV_SHIFT - 1);

/// `cvRound(255 * 2^12 / i)` — the reciprocal table saturation is scaled by.
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

/// `cvRound(180 * 2^12 / (6 * i))` — hue is expressed on 0..179, and the raw
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

/// `cvRound` — nearest, ties to even.
pub fn cv_round(v: f64) -> i32 {
    v.round_ties_even() as i32
}

/// `cvFloor`.
pub fn cv_floor(v: f32) -> i32 {
    v.floor() as i32
}

/// `saturate_cast<uchar>` from a float — rounds, then clamps to 0..255.
pub fn saturate_u8(v: f32) -> u8 {
    cv_round(f64::from(v)).clamp(0, 255) as u8
}

/// One pixel through `cv2.cvtColor(..., cv2.COLOR_BGR2HSV)`.
///
/// Reproduced as integer arithmetic rather than converted through floats because
/// the guard thresholds the detector compares against were derived from this
/// exact table-driven rounding; a value off by one at a boundary is a guard that
/// silently stops matching.
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

/// `cv2.inRange(cv2.cvtColor(bgr, COLOR_BGR2HSV), lower, upper)`, fused so the
/// intermediate HSV image is never materialised — on a full 2560x1440 grab that
/// is 11 MB of allocation the Python path paid and this one does not.
pub fn hsv_in_range(bgr: &[u8], w: usize, h: usize, lower: [u8; 3], upper: [u8; 3]) -> Gray {
    let mut data = Vec::with_capacity(w * h);
    for px in bgr.chunks_exact(3).take(w * h) {
        let (hh, ss, vv) = bgr_to_hsv(px[0], px[1], px[2]);
        let hit = hh >= lower[0]
            && hh <= upper[0]
            && ss >= lower[1]
            && ss <= upper[1]
            && vv >= lower[2]
            && vv <= upper[2];
        data.push(if hit { 255 } else { 0 });
    }
    Gray::from_vec(w, h, data)
}

// ── Morphology ───────────────────────────────────────────────────────────────

/// Erode with a 3x3 rectangular kernel. Out-of-bounds neighbours are the
/// `+inf` of OpenCV's default morphology border, i.e. they never lower the min.
fn erode3(src: &Gray) -> Gray {
    neighbourhood3(src, u8::min, u8::MAX)
}

/// Dilate with a 3x3 rectangular kernel; out-of-bounds is `-inf` for the max.
fn dilate3(src: &Gray) -> Gray {
    neighbourhood3(src, u8::max, u8::MIN)
}

fn neighbourhood3(src: &Gray, pick: fn(u8, u8) -> u8, outside: u8) -> Gray {
    let mut out = Gray::new(src.w, src.h);
    for y in 0..src.h {
        for x in 0..src.w {
            let mut acc = outside;
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    let nx = x as i32 + dx;
                    let ny = y as i32 + dy;
                    let v = if nx < 0 || ny < 0 || nx >= src.w as i32 || ny >= src.h as i32 {
                        outside
                    } else {
                        src.at(nx as usize, ny as usize)
                    };
                    acc = pick(acc, v);
                }
            }
            out.set(x, y, acc);
        }
    }
    out
}

/// `cv2.morphologyEx(..., cv2.MORPH_OPEN, 3x3 rect, iterations)`.
pub fn morph_open(src: &Gray, iterations: usize) -> Gray {
    let mut m = src.clone();
    for _ in 0..iterations {
        m = erode3(&m);
    }
    for _ in 0..iterations {
        m = dilate3(&m);
    }
    m
}

/// `cv2.morphologyEx(..., cv2.MORPH_CLOSE, kernel, iterations)`. OpenCV applies
/// all dilations before all erosions for iterated CLOSE, which is *not* the same
/// as repeating a full close.
pub fn morph_close(src: &Gray, iterations: usize) -> Gray {
    let mut m = src.clone();
    for _ in 0..iterations {
        m = dilate3(&m);
    }
    for _ in 0..iterations {
        m = erode3(&m);
    }
    m
}

/// `cv2.dilate` with a 5x5 elliptical kernel — the one `_auto_mask` uses. At 5x5
/// OpenCV's `MORPH_ELLIPSE` is the plus-shape-with-corners-filled below.
const ELLIPSE5: [[bool; 5]; 5] = [
    [false, false, true, false, false],
    [true, true, true, true, true],
    [true, true, true, true, true],
    [true, true, true, true, true],
    [false, false, true, false, false],
];

pub fn dilate_ellipse5(src: &Gray, iterations: usize) -> Gray {
    let mut m = src.clone();
    for _ in 0..iterations {
        m = ellipse5_pass(&m, u8::max, u8::MIN);
    }
    m
}

/// `cv2.morphologyEx(..., MORPH_CLOSE, 5x5 ellipse, iterations)`.
pub fn morph_close_ellipse5(src: &Gray, iterations: usize) -> Gray {
    let mut m = src.clone();
    for _ in 0..iterations {
        m = ellipse5_pass(&m, u8::max, u8::MIN);
    }
    for _ in 0..iterations {
        m = ellipse5_pass(&m, u8::min, u8::MAX);
    }
    m
}

fn ellipse5_pass(src: &Gray, pick: fn(u8, u8) -> u8, outside: u8) -> Gray {
    let mut out = Gray::new(src.w, src.h);
    for y in 0..src.h {
        for x in 0..src.w {
            let mut acc = outside;
            for (ky, row) in ELLIPSE5.iter().enumerate() {
                for (kx, &on) in row.iter().enumerate() {
                    if !on {
                        continue;
                    }
                    let nx = x as i32 + kx as i32 - 2;
                    let ny = y as i32 + ky as i32 - 2;
                    let v = if nx < 0 || ny < 0 || nx >= src.w as i32 || ny >= src.h as i32 {
                        outside
                    } else {
                        src.at(nx as usize, ny as usize)
                    };
                    acc = pick(acc, v);
                }
            }
            out.set(x, y, acc);
        }
    }
    out
}

// ── Flood fill ───────────────────────────────────────────────────────────────

/// `cv2.floodFill(img, mask, seed, new_val)` with the default 4-connectivity and
/// zero tolerance: repaints the connected run of pixels equal to the seed value.
/// No-op when the seed already holds `new_val`, matching OpenCV.
pub fn flood_fill(img: &mut Gray, seed: (usize, usize), new_val: u8) {
    let target = img.at(seed.0, seed.1);
    if target == new_val {
        return;
    }
    let mut stack = vec![seed];
    img.set(seed.0, seed.1, new_val);
    while let Some((x, y)) = stack.pop() {
        let neighbours = [
            (x.wrapping_sub(1), y),
            (x + 1, y),
            (x, y.wrapping_sub(1)),
            (x, y + 1),
        ];
        for (nx, ny) in neighbours {
            if nx >= img.w || ny >= img.h {
                continue;
            }
            if img.at(nx, ny) == target {
                img.set(nx, ny, new_val);
                stack.push((nx, ny));
            }
        }
    }
}

// ── Contours ─────────────────────────────────────────────────────────────────

/// One traced border, as the chain of pixel centres OpenCV's border following
/// visits. `CHAIN_APPROX_SIMPLE` drops the collinear interior of each run, which
/// changes neither the shoelace area nor the bounding box, so the chain is kept
/// whole and the approximation is skipped.
#[derive(Debug, Clone)]
pub struct Contour {
    pub points: Vec<(i32, i32)>,
}

impl Contour {
    /// `cv2.contourArea` — the shoelace formula over the border polygon. Note
    /// this is *not* the pixel count: a solid `w`x`h` blob traces its border
    /// pixel centres and so measures `(w-1)*(h-1)`.
    pub fn area(&self) -> f64 {
        let n = self.points.len();
        if n < 3 {
            return 0.0;
        }
        let mut a = 0.0f64;
        let mut prev = self.points[n - 1];
        for &p in &self.points {
            a += f64::from(prev.0) * f64::from(p.1) - f64::from(p.0) * f64::from(prev.1);
            prev = p;
        }
        (a / 2.0).abs()
    }

    /// `cv2.boundingRect` — `(x, y, w, h)`, inclusive of both extremes.
    pub fn bounding_rect(&self) -> (i32, i32, i32, i32) {
        let mut x0 = i32::MAX;
        let mut y0 = i32::MAX;
        let mut x1 = i32::MIN;
        let mut y1 = i32::MIN;
        for &(x, y) in &self.points {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
        (x0, y0, x1 - x0 + 1, y1 - y0 + 1)
    }
}

/// The eight neighbour offsets in OpenCV's `icvCodeDeltas` order: 0 is east and
/// the sequence runs counter-clockwise in image coordinates (y grows down).
const DELTAS: [(i32, i32); 8] =
    [(1, 0), (1, -1), (0, -1), (-1, -1), (-1, 0), (-1, 1), (0, 1), (1, 1)];

/// `cv2.findContours(mask, cv2.RETR_EXTERNAL, cv2.CHAIN_APPROX_SIMPLE)`.
///
/// `RETR_EXTERNAL` keeps exactly the outer boundaries of the 8-connected
/// foreground components that are not enclosed by another component, so the
/// components are found by labelling and the "not enclosed" test is
/// 8-adjacency to the background region reachable from the image border. Each
/// kept component's boundary is then traced with OpenCV's own border-following
/// walk, from the same start pixel its raster scan would have found.
pub fn find_external_contours(mask: &Gray) -> Vec<Contour> {
    if mask.is_empty() {
        return Vec::new();
    }
    let (w, h) = (mask.w, mask.h);
    let outside = outer_background(mask);

    let mut label = vec![u32::MAX; w * h];
    let mut out = Vec::new();
    let mut next_label = 0u32;

    for y in 0..h {
        for x in 0..w {
            if mask.at(x, y) == 0 || label[y * w + x] != u32::MAX {
                continue;
            }
            let id = next_label;
            next_label += 1;
            // Flood the component (8-connected) and note whether any of its
            // pixels touches the outer background — that is what makes it
            // external rather than a blob sitting inside someone else's hole.
            let mut external = false;
            let mut stack = vec![(x, y)];
            label[y * w + x] = id;
            while let Some((cx, cy)) = stack.pop() {
                for (dx, dy) in DELTAS {
                    let nx = cx as i32 + dx;
                    let ny = cy as i32 + dy;
                    if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                        external = true;
                        continue;
                    }
                    let (nx, ny) = (nx as usize, ny as usize);
                    if mask.at(nx, ny) == 0 {
                        external |= outside[ny * w + nx];
                        continue;
                    }
                    if label[ny * w + nx] == u32::MAX {
                        label[ny * w + nx] = id;
                        stack.push((nx, ny));
                    }
                }
            }
            if external {
                out.push(trace_border(mask, (x as i32, y as i32)));
            }
        }
    }
    out
}

/// The background pixels 4-connected to the image border — everything OpenCV
/// would consider "outside" all contours.
fn outer_background(mask: &Gray) -> Vec<bool> {
    let (w, h) = (mask.w, mask.h);
    let mut seen = vec![false; w * h];
    let mut stack: Vec<(usize, usize)> = Vec::new();
    let seed = |x: usize, y: usize, seen: &mut Vec<bool>, stack: &mut Vec<(usize, usize)>| {
        if mask.at(x, y) == 0 && !seen[y * w + x] {
            seen[y * w + x] = true;
            stack.push((x, y));
        }
    };
    for x in 0..w {
        seed(x, 0, &mut seen, &mut stack);
        seed(x, h - 1, &mut seen, &mut stack);
    }
    for y in 0..h {
        seed(0, y, &mut seen, &mut stack);
        seed(w - 1, y, &mut seen, &mut stack);
    }
    while let Some((x, y)) = stack.pop() {
        for (dx, dy) in [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)] {
            let nx = x as i32 + dx;
            let ny = y as i32 + dy;
            if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                continue;
            }
            let (nx, ny) = (nx as usize, ny as usize);
            if mask.at(nx, ny) == 0 && !seen[ny * w + nx] {
                seen[ny * w + nx] = true;
                stack.push((nx, ny));
            }
        }
    }
    seen
}

/// OpenCV's `icvFetchContour` for an outer border, transcribed. The walk starts
/// looking west (`s = 4`, the direction the raster scan arrived from) and turns
/// clockwise until it finds foreground, then follows the boundary until it
/// returns to the start edge.
fn trace_border(mask: &Gray, origin: (i32, i32)) -> Contour {
    let (w, h) = (mask.w as i32, mask.h as i32);
    let get = |p: (i32, i32)| -> bool {
        p.0 >= 0 && p.1 >= 0 && p.0 < w && p.1 < h && mask.at(p.0 as usize, p.1 as usize) != 0
    };

    let i0 = origin;
    let mut s = 4usize;
    let s_end = 4usize;
    let mut i1 = i0;
    let mut found = false;
    loop {
        s = (s + 7) & 7;
        let cand = (i0.0 + DELTAS[s].0, i0.1 + DELTAS[s].1);
        if get(cand) {
            i1 = cand;
            found = true;
            break;
        }
        if s == s_end {
            break;
        }
    }
    if !found {
        return Contour { points: vec![i0] };
    }

    let mut points = Vec::new();
    let mut i3 = i0;
    loop {
        let s_start = s;
        let mut i4 = i3;
        loop {
            s = (s + 1) & 7;
            let cand = (i3.0 + DELTAS[s].0, i3.1 + DELTAS[s].1);
            if get(cand) {
                i4 = cand;
                break;
            }
            if s == s_start {
                // Fully isolated pixel — cannot happen once `found` is true,
                // but a runaway inner loop is worse than an early exit.
                break;
            }
        }
        points.push(i3);
        if i4 == i0 && i3 == i1 {
            break;
        }
        i3 = i4;
        s = (s + 4) & 7;
    }
    Contour { points }
}

// ── Resampling ───────────────────────────────────────────────────────────────

/// Which `cv2.resize` interpolation to use.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Interp {
    Nearest,
    Linear,
    Area,
}

/// `cv2.resize`.
///
/// The one place this is not a transcription: OpenCV resamples 8-bit images
/// through fixed-point weights, and these run in `f32`. The coordinate mapping —
/// the half-pixel shift that decides *alignment* — is OpenCV's; only the weight
/// rounding differs, which moves a correlation score in the third decimal and
/// never moves a match.
pub fn resize(src: &Gray, dw: usize, dh: usize, interp: Interp) -> Gray {
    if dw == 0 || dh == 0 || src.is_empty() {
        return Gray::new(dw, dh);
    }
    if dw == src.w && dh == src.h {
        return src.clone();
    }
    match interp {
        Interp::Nearest => resize_nearest(src, dw, dh),
        Interp::Linear => resize_linear(src, dw, dh),
        // OpenCV silently falls back to bilinear when INTER_AREA is asked to
        // *upscale*; the sweep relies on that for scales above 1.
        Interp::Area if dw >= src.w && dh >= src.h => resize_linear(src, dw, dh),
        Interp::Area => resize_area(src, dw, dh),
    }
}

fn resize_nearest(src: &Gray, dw: usize, dh: usize) -> Gray {
    let sx_ratio = src.w as f64 / dw as f64;
    let sy_ratio = src.h as f64 / dh as f64;
    let mut out = Gray::new(dw, dh);
    for y in 0..dh {
        let sy = ((y as f64 * sy_ratio) as usize).min(src.h - 1);
        for x in 0..dw {
            let sx = ((x as f64 * sx_ratio) as usize).min(src.w - 1);
            out.set(x, y, src.at(sx, sy));
        }
    }
    out
}

/// The per-axis source index and weight OpenCV's bilinear path computes, with
/// its `(d + 0.5) * scale - 0.5` mapping and its edge clamping.
fn linear_taps(dst_len: usize, src_len: usize) -> Vec<(usize, usize, f32)> {
    let scale = src_len as f32 / dst_len as f32;
    (0..dst_len)
        .map(|d| {
            let f = (d as f32 + 0.5) * scale - 0.5;
            let mut i = cv_floor(f);
            let mut frac = f - i as f32;
            if i < 0 {
                i = 0;
                frac = 0.0;
            }
            if i >= src_len as i32 - 1 {
                i = src_len as i32 - 1;
                frac = 0.0;
            }
            let i = i as usize;
            (i, (i + 1).min(src_len - 1), frac)
        })
        .collect()
}

fn resize_linear(src: &Gray, dw: usize, dh: usize) -> Gray {
    let xs = linear_taps(dw, src.w);
    let ys = linear_taps(dh, src.h);
    let mut out = Gray::new(dw, dh);
    for (y, &(y0, y1, fy)) in ys.iter().enumerate() {
        for (x, &(x0, x1, fx)) in xs.iter().enumerate() {
            let a = f32::from(src.at(x0, y0));
            let b = f32::from(src.at(x1, y0));
            let c = f32::from(src.at(x0, y1));
            let d = f32::from(src.at(x1, y1));
            let top = a + (b - a) * fx;
            let bot = c + (d - c) * fx;
            out.set(x, y, saturate_u8(top + (bot - top) * fy));
        }
    }
    out
}

/// The source span each output pixel averages, as `(index, weight)` pairs —
/// OpenCV's `INTER_AREA` decimation weights, where partially covered edge pixels
/// contribute their overlap fraction.
fn area_taps(dst_len: usize, src_len: usize) -> Vec<Vec<(usize, f32)>> {
    let scale = src_len as f64 / dst_len as f64;
    (0..dst_len)
        .map(|d| {
            let start = d as f64 * scale;
            let end = ((d + 1) as f64 * scale).min(src_len as f64);
            let first = start.floor() as usize;
            let last = ((end.ceil() as usize).max(first + 1)).min(src_len);
            let mut taps = Vec::with_capacity(last - first);
            let mut total = 0.0f64;
            for i in first..last {
                let cover = (end.min((i + 1) as f64) - start.max(i as f64)).max(0.0);
                if cover > 0.0 {
                    taps.push((i, cover as f32));
                    total += cover;
                }
            }
            if total > 0.0 {
                for tap in &mut taps {
                    tap.1 = (f64::from(tap.1) / total) as f32;
                }
            }
            taps
        })
        .collect()
}

fn resize_area(src: &Gray, dw: usize, dh: usize) -> Gray {
    let xs = area_taps(dw, src.w);
    let ys = area_taps(dh, src.h);
    let mut out = Gray::new(dw, dh);
    for (y, ytaps) in ys.iter().enumerate() {
        for (x, xtaps) in xs.iter().enumerate() {
            let mut acc = 0.0f32;
            for &(sy, wy) in ytaps {
                let row = sy * src.w;
                let mut rowacc = 0.0f32;
                for &(sx, wx) in xtaps {
                    rowacc += f32::from(src.data[row + sx]) * wx;
                }
                acc += rowacc * wy;
            }
            out.set(x, y, saturate_u8(acc));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gray_matches_opencvs_fixed_point_luma() {
        // Values read off cv2.cvtColor for the primaries and a mid grey.
        let bgr = [0u8, 0, 255, 0, 255, 0, 255, 0, 0, 128, 128, 128];
        let g = bgr_to_gray(&bgr, 4, 1);
        assert_eq!(g.data, vec![76, 150, 29, 128]);
    }

    #[test]
    fn hsv_lands_on_opencvs_integer_triples() {
        assert_eq!(bgr_to_hsv(0, 0, 255), (0, 255, 255)); // pure red
        assert_eq!(bgr_to_hsv(0, 255, 0), (60, 255, 255)); // pure green
        assert_eq!(bgr_to_hsv(255, 0, 0), (120, 255, 255)); // pure blue
        assert_eq!(bgr_to_hsv(7, 7, 7), (0, 0, 7)); // grey: no hue, no saturation
    }

    #[test]
    fn in_range_keeps_only_pixels_inside_every_channel_band() {
        // Red passes a red band; green does not, despite equal S and V.
        let bgr = [0u8, 0, 255, 0, 255, 0];
        let m = hsv_in_range(&bgr, 2, 1, [0, 200, 200], [10, 255, 255]);
        assert_eq!(m.data, vec![255, 0]);
    }

    #[test]
    fn opening_removes_a_lone_speck_and_closing_fills_a_pinhole() {
        let mut speck = Gray::new(5, 5);
        speck.set(2, 2, 255);
        assert_eq!(morph_open(&speck, 1).count_nonzero(), 0);

        let mut holed = Gray::from_vec(5, 5, vec![255; 25]);
        holed.set(2, 2, 0);
        assert_eq!(morph_close(&holed, 1).count_nonzero(), 25);
    }

    #[test]
    fn a_solid_rectangle_measures_its_border_polygon_not_its_pixels() {
        // cv2.contourArea over a traced 10x6 block is (10-1)*(6-1) = 45, while
        // the block holds 60 pixels. Confidence is a ratio of the former.
        let mut m = Gray::new(20, 20);
        for y in 4..10 {
            for x in 3..13 {
                m.set(x, y, 255);
            }
        }
        let cs = find_external_contours(&m);
        assert_eq!(cs.len(), 1);
        assert_eq!(cs[0].bounding_rect(), (3, 4, 10, 6));
        assert_eq!(cs[0].area(), 45.0);
    }

    #[test]
    fn retr_external_skips_a_blob_sitting_inside_anothers_hole() {
        // A ring with a dot in the middle: OpenCV returns the ring only.
        let mut m = Gray::new(21, 21);
        for y in 2..19 {
            for x in 2..19 {
                let on = y < 5 || y >= 16 || x < 5 || x >= 16;
                if on {
                    m.set(x, y, 255);
                }
            }
        }
        m.set(10, 10, 255);
        let cs = find_external_contours(&m);
        assert_eq!(cs.len(), 1);
        assert_eq!(cs[0].bounding_rect(), (2, 2, 17, 17));
    }

    #[test]
    fn two_separate_blobs_come_back_in_raster_order() {
        let mut m = Gray::new(30, 12);
        for y in 1..4 {
            for x in 1..4 {
                m.set(x, y, 255);
            }
        }
        for y in 6..11 {
            for x in 20..27 {
                m.set(x, y, 255);
            }
        }
        let cs = find_external_contours(&m);
        assert_eq!(cs.len(), 2);
        assert_eq!(cs[0].bounding_rect(), (1, 1, 3, 3));
        assert_eq!(cs[1].bounding_rect(), (20, 6, 7, 5));
    }

    #[test]
    fn flood_fill_repaints_only_the_connected_run() {
        let mut g = Gray::new(5, 3);
        g.set(2, 0, 7); // a wall down the middle
        g.set(2, 1, 7);
        g.set(2, 2, 7);
        flood_fill(&mut g, (0, 0), 128);
        assert_eq!(g.at(1, 2), 128);
        assert_eq!(g.at(3, 0), 0, "the far side of the wall is untouched");
    }

    #[test]
    fn halving_a_block_averages_it_and_doubling_it_interpolates() {
        let src = Gray::from_vec(2, 2, vec![0, 100, 200, 255]);
        let small = resize(&src, 1, 1, Interp::Area);
        assert_eq!(small.data, vec![139]); // (0+100+200+255)/4 = 138.75

        let big = resize(&src, 4, 4, Interp::Nearest);
        assert_eq!(big.w, 4);
        assert_eq!(big.at(0, 0), 0);
        assert_eq!(big.at(3, 3), 255);
    }

    #[test]
    fn bilinear_keeps_a_flat_image_flat() {
        let src = Gray::from_vec(8, 8, vec![91; 64]);
        let out = resize(&src, 5, 3, Interp::Linear);
        assert!(out.data.iter().all(|&v| v == 91));
    }
}
