//! Raster output for the UI: annotated Test-button screenshots, picker
//! thumbnails, and saved template PNGs.
//!
//! Every Test-button preview is drawn here, whether the trigger was read by
//! `hardware::vision` or `hardware::ocr`. The shapes and colours deliberately
//! match `server.py::_guard_test_on_frame` and `_ai_test_step_on_frame` so the
//! editor looks the same as it did under cv2: a region box, a ring on every
//! match, and a ringed crosshair on the best one. The same canvas then serves
//! `hardware::picker`, which replaced the cv2 `imwrite`/`imencode` calls the
//! Python pickers ended on.

use std::fs::File;
use std::io::{BufWriter, Cursor};
use std::path::Path;

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;

use super::capture::Frame;
use super::vision::Detection;

/// cv2 draws in BGR; these are the same three colours in RGB, the order a PNG
/// wants. `REGION` is the dusty blue box, `MATCH` the green ring, `BEST` the red
/// crosshair.
const REGION: [u8; 3] = [192, 168, 74];
const MATCH: [u8; 3] = [95, 174, 95];
const BEST: [u8; 3] = [255, 95, 95];

/// The step editor's region box: cyan, deliberately distinct from the guard
/// box above so the two Test buttons are never confused
/// (`_draw_step_preview`'s BGR `(235, 168, 74)`).
const STEP_REGION: [u8; 3] = [74, 168, 235];

/// The pickers' accent, `(74, 168, 192)` in the BGR the Python modules wrote it
/// in, which is the same colour as the region box above.
const GOLD: [u8; 3] = REGION;

/// Python previews at most this many match rings (`matches[:20]`).
const MAX_RINGS: usize = 20;

/// The editor shows this image a few hundred pixels wide, so a full 2560px
/// screenshot is encoded, base64'd, and shipped over the IPC bridge for nothing.
/// Anything wider is box-averaged down by an integer factor first, which cut a
/// 2560x1440 preview from ~710 KB / 706 ms to ~252 KB / 504 ms.
const MAX_PREVIEW_WIDTH: u32 = 1280;

/// An RGB canvas being drawn on. Owns its pixels so the source frame is never
/// mutated; the caller may still be reading it.
struct Canvas {
    rgb: Vec<u8>,
    width: i32,
    height: i32,
}

impl Canvas {
    /// Convert a captured BGR frame into an RGB canvas.
    fn from_frame(frame: &Frame) -> Self {
        let mut rgb = Vec::with_capacity(frame.bgr.len());
        for px in frame.bgr.chunks_exact(3) {
            rgb.extend_from_slice(&[px[2], px[1], px[0]]);
        }
        Self { rgb, width: frame.width as i32, height: frame.height as i32 }
    }

    /// Convert and box-average down by `factor` in one pass. Averaging rather
    /// than point-sampling because the preview is a screenshot of text and UI:
    /// dropping 3 of every 4 pixels makes it crawl with aliasing.
    fn from_frame_scaled(frame: &Frame, factor: u32) -> Self {
        if factor <= 1 {
            return Self::from_frame(frame);
        }
        let (fw, fh, f) = (frame.width as usize, frame.height as usize, factor as usize);
        let (ow, oh) = (fw / f, fh / f);
        let mut rgb = Vec::with_capacity(ow * oh * 3);
        let n = (f * f) as u32;
        for oy in 0..oh {
            for ox in 0..ow {
                let (mut b, mut g, mut r) = (0u32, 0u32, 0u32);
                for dy in 0..f {
                    let row = (oy * f + dy) * fw;
                    for dx in 0..f {
                        let i = (row + ox * f + dx) * 3;
                        b += u32::from(frame.bgr[i]);
                        g += u32::from(frame.bgr[i + 1]);
                        r += u32::from(frame.bgr[i + 2]);
                    }
                }
                rgb.extend_from_slice(&[(r / n) as u8, (g / n) as u8, (b / n) as u8]);
            }
        }
        Self { rgb, width: ow as i32, height: oh as i32 }
    }

    /// Paint one pixel, ignoring anything outside the canvas. Clipping here (not
    /// at each call site) is what lets the shape helpers draw straight through
    /// the edges without arithmetic on every point.
    fn put(&mut self, x: i32, y: i32, color: [u8; 3]) {
        if x < 0 || y < 0 || x >= self.width || y >= self.height {
            return;
        }
        let i = (y as usize * self.width as usize + x as usize) * 3;
        self.rgb[i..i + 3].copy_from_slice(&color);
    }

    /// A horizontal run, used to give every stroke its thickness.
    fn hspan(&mut self, x0: i32, x1: i32, y: i32, color: [u8; 3]) {
        for x in x0..=x1 {
            self.put(x, y, color);
        }
    }

    /// Axis-aligned rectangle outline, `thickness` pixels drawn inward-ish, the
    /// shape of cv2's `rectangle(..., thickness)`.
    fn rect(&mut self, x1: i32, y1: i32, x2: i32, y2: i32, color: [u8; 3], thickness: i32) {
        for t in 0..thickness {
            self.hspan(x1, x2, y1 + t, color);
            self.hspan(x1, x2, y2 - t, color);
            for y in y1..=y2 {
                self.put(x1 + t, y, color);
                self.put(x2 - t, y, color);
            }
        }
    }

    /// Circle outline via the midpoint algorithm, thickened by drawing
    /// concentric radii, cv2's `circle(..., thickness)`.
    fn circle(&mut self, cx: i32, cy: i32, radius: i32, color: [u8; 3], thickness: i32) {
        for r in (radius - thickness + 1).max(1)..=radius {
            let (mut x, mut y, mut err) = (r, 0, 0);
            while x >= y {
                for (dx, dy) in
                    [(x, y), (y, x), (-y, x), (-x, y), (-x, -y), (-y, -x), (y, -x), (x, -y)]
                {
                    self.put(cx + dx, cy + dy, color);
                }
                y += 1;
                err += 1 + 2 * y;
                if 2 * (err - x) + 1 > 0 {
                    x -= 1;
                    err += 1 - 2 * x;
                }
            }
        }
    }

    /// Filled disc: cv2's `circle(..., -1)`, the stroke endpoint markers.
    fn disc(&mut self, cx: i32, cy: i32, radius: i32, color: [u8; 3]) {
        for dy in -radius..=radius {
            let run = ((radius * radius - dy * dy) as f64).sqrt() as i32;
            self.hspan(cx - run, cx + run, cy + dy, color);
        }
    }

    /// Line at any angle, thickened by stamping a disc along it. Slower than a
    /// span fill but correct for diagonals, which the surgical strokes are.
    fn stroke(&mut self, x1: i32, y1: i32, x2: i32, y2: i32, color: [u8; 3], thickness: i32) {
        let radius = (thickness / 2).max(0);
        let steps = (x2 - x1).abs().max((y2 - y1).abs()).max(1);
        for i in 0..=steps {
            let x = x1 + (x2 - x1) * i / steps;
            let y = y1 + (y2 - y1) * i / steps;
            if radius == 0 {
                self.put(x, y, color);
            } else {
                self.disc(x, y, radius, color);
            }
        }
    }

    /// Box-average resize to an arbitrary size. cv2's `INTER_AREA`, which is what
    /// the surgical thumbnail asked for; the template thumbnail used the default
    /// `INTER_LINEAR`, and shares this path because both only ever shrink, where
    /// area averaging is the better of the two.
    fn resized(&self, width: i32, height: i32) -> Self {
        let (ow, oh) = (width.max(1), height.max(1));
        let mut rgb = Vec::with_capacity((ow * oh * 3) as usize);
        for oy in 0..oh {
            let (y0, y1) = (oy * self.height / oh, ((oy + 1) * self.height / oh).max(oy * self.height / oh + 1));
            for ox in 0..ow {
                let (x0, x1) = (ox * self.width / ow, ((ox + 1) * self.width / ow).max(ox * self.width / ow + 1));
                let (mut r, mut g, mut b, mut n) = (0u32, 0u32, 0u32, 0u32);
                for y in y0..y1.min(self.height) {
                    for x in x0..x1.min(self.width) {
                        let i = (y as usize * self.width as usize + x as usize) * 3;
                        r += u32::from(self.rgb[i]);
                        g += u32::from(self.rgb[i + 1]);
                        b += u32::from(self.rgb[i + 2]);
                        n += 1;
                    }
                }
                let n = n.max(1);
                rgb.extend_from_slice(&[(r / n) as u8, (g / n) as u8, (b / n) as u8]);
            }
        }
        Self { rgb, width: ow, height: oh }
    }

    /// Straight line, thickened perpendicular to its dominant axis. Only the
    /// horizontal and vertical arms of a crosshair are ever drawn, so a full
    /// Bresenham is not needed.
    fn line(&mut self, x1: i32, y1: i32, x2: i32, y2: i32, color: [u8; 3], thickness: i32) {
        let half = thickness / 2;
        if y1 == y2 {
            for t in -half..=half {
                self.hspan(x1.min(x2), x1.max(x2), y1 + t, color);
            }
        } else if x1 == x2 {
            for t in -half..=half {
                for y in y1.min(y2)..=y1.max(y2) {
                    self.put(x1 + t, y, color);
                }
            }
        }
    }

    /// PNG-encode the canvas.
    fn to_png(&self) -> Result<Vec<u8>, png::EncodingError> {
        let mut buf = Vec::new();
        {
            let mut encoder =
                png::Encoder::new(Cursor::new(&mut buf), self.width as u32, self.height as u32);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            // The preview is a transient screenshot shown once, so trade ratio
            // for latency; the Test button should feel instant.
            encoder.set_compression(png::Compression::Fast);
            encoder.write_header()?.write_image_data(&self.rgb)?;
        }
        Ok(buf)
    }
}

/// Draw `matches` onto `frame` and return the base64 PNG the guard editor embeds.
///
/// `region` is the pixel rectangle to outline. An encoding failure yields an
/// empty string, exactly as the sidecar returned `""` when `cv2.imencode` failed;
/// the editor already renders the verdict without an image.
pub fn annotate(frame: &Frame, region: (i32, i32, i32, i32), matches: &[Detection]) -> String {
    draw(frame, region, REGION, matches, None)
}

/// The step editor's variant: a cyan region box instead of the guard blue, and
/// an explicit crosshair position for `click` steps, which have a fixed target
/// rather than a detected one.
pub fn annotate_step(
    frame: &Frame,
    region: (i32, i32, i32, i32),
    matches: &[Detection],
    crosshair: Option<(i64, i64)>,
) -> String {
    draw(frame, region, STEP_REGION, matches, crosshair)
}

/// The shared preview drawing: region box, a ring per match, and a crosshair on
/// the best one (or on `crosshair`, when the caller names the point itself).
fn draw(
    frame: &Frame,
    region: (i32, i32, i32, i32),
    box_color: [u8; 3],
    matches: &[Detection],
    crosshair: Option<(i64, i64)>,
) -> String {
    // Scale the *image* down, then draw at the reduced size, so the annotations
    // stay full-strength instead of being thinned along with the screenshot.
    let factor = preview_factor(frame.width);
    let mut canvas = Canvas::from_frame_scaled(frame, factor);
    let s = |v: i64| (v / i64::from(factor)) as i32;

    let (x1, y1, x2, y2) = region;
    canvas.rect(s(x1.into()), s(y1.into()), s(x2.into()), s(y2.into()), box_color, 2);

    for m in matches.iter().take(MAX_RINGS) {
        canvas.circle(s(m.x), s(m.y), 8, MATCH, 2);
    }
    let point = crosshair.or_else(|| matches.first().map(|b| (b.x, b.y)));
    if let Some((x, y)) = point {
        let (bx, by) = (s(x), s(y));
        canvas.circle(bx, by, 14, BEST, 3);
        canvas.line(bx - 20, by, bx + 20, by, BEST, 2);
        canvas.line(bx, by - 20, bx, by + 20, BEST, 2);
    }

    encode(&canvas)
}

/// Integer downscale that brings `width` to at most [`MAX_PREVIEW_WIDTH`].
fn preview_factor(width: u32) -> u32 {
    if width <= MAX_PREVIEW_WIDTH {
        1
    } else {
        width.div_ceil(MAX_PREVIEW_WIDTH)
    }
}

/// Write `frame` to `path` as a PNG: the template files the pickers save, which
/// the detector later reads back through `vision::read_frame`. Replaces
/// `cv2.imwrite`.
pub fn save_png(path: &Path, frame: &Frame) -> std::io::Result<()> {
    let canvas = Canvas::from_frame(frame);
    let file = BufWriter::new(File::create(path)?);
    let mut encoder = png::Encoder::new(file, canvas.width as u32, canvas.height as u32);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .and_then(|mut w| w.write_image_data(&canvas.rgb))
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
}

/// A base64 PNG thumbnail of `frame`, scaled to `height` px tall with its aspect
/// ratio kept, the picker toasts' preview image.
pub fn thumbnail(frame: &Frame, height: u32) -> String {
    encode(&scaled_thumb(frame, height).0)
}

/// The surgical thumbnail: the same scaled crop with every drawn stroke on it
/// (a gold line, a white dot at the start, a gold dot at the end).
pub fn stroke_thumbnail(frame: &Frame, height: u32, strokes: &[[i32; 4]]) -> String {
    let (mut thumb, xscale, yscale) = scaled_thumb(frame, height);
    let width = (3.0 * xscale.min(yscale)) as i32;
    let dot = ((4.0 * xscale) as i32).max(3);
    for s in strokes {
        let (x1, y1) = ((s[0] as f64 * xscale) as i32, (s[1] as f64 * yscale) as i32);
        let (x2, y2) = ((s[2] as f64 * xscale) as i32, (s[3] as f64 * yscale) as i32);
        thumb.stroke(x1, y1, x2, y2, GOLD, width.max(2));
        thumb.disc(x1, y1, dot, [255, 255, 255]);
        thumb.disc(x2, y2, dot, GOLD);
    }
    encode(&thumb)
}

/// Shrink `frame` to `height` px tall, reporting the scale factors so callers can
/// map crop-space coordinates onto the result.
fn scaled_thumb(frame: &Frame, height: u32) -> (Canvas, f64, f64) {
    let canvas = Canvas::from_frame(frame);
    let scale = f64::from(height) / f64::from(canvas.height.max(1));
    let w = ((f64::from(canvas.width) * scale) as i32).max(1);
    let h = (height as i32).max(1);
    let thumb = canvas.resized(w, h);
    (thumb, f64::from(w) / f64::from(canvas.width.max(1)), f64::from(h) / f64::from(canvas.height.max(1)))
}

/// Encode to base64 PNG; an encoder failure yields `""`, the same empty-thumb
/// fallback the sidecar returned when `cv2.imencode` failed.
fn encode(canvas: &Canvas) -> String {
    match canvas.to_png() {
        Ok(bytes) => STANDARD.encode(bytes),
        Err(_) => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(w: u32, h: u32) -> Frame {
        Frame { bgr: vec![0u8; (w * h * 3) as usize], width: w, height: h }
    }

    fn pixel(c: &Canvas, x: i32, y: i32) -> [u8; 3] {
        let i = (y as usize * c.width as usize + x as usize) * 3;
        [c.rgb[i], c.rgb[i + 1], c.rgb[i + 2]]
    }

    #[test]
    fn bgr_frame_becomes_rgb_canvas() {
        // One pure-red pixel, which is [0,0,255] in BGR.
        let f = Frame { bgr: vec![0, 0, 255], width: 1, height: 1 };
        assert_eq!(pixel(&Canvas::from_frame(&f), 0, 0), [255, 0, 0]);
    }

    #[test]
    fn put_ignores_out_of_bounds_instead_of_panicking() {
        let mut c = Canvas::from_frame(&frame(4, 4));
        for (x, y) in [(-1, 0), (0, -1), (4, 0), (0, 4), (99, 99)] {
            c.put(x, y, [1, 2, 3]);
        }
        assert!(c.rgb.iter().all(|&b| b == 0));
    }

    #[test]
    fn rect_draws_edges_and_leaves_the_interior_clear() {
        let mut c = Canvas::from_frame(&frame(10, 10));
        c.rect(2, 2, 7, 7, REGION, 1);
        assert_eq!(pixel(&c, 2, 2), REGION); // corner
        assert_eq!(pixel(&c, 5, 2), REGION); // top edge
        assert_eq!(pixel(&c, 2, 5), REGION); // left edge
        assert_eq!(pixel(&c, 7, 7), REGION); // far corner
        assert_eq!(pixel(&c, 5, 5), [0, 0, 0]); // interior untouched
    }

    #[test]
    fn circle_marks_its_cardinal_points() {
        let mut c = Canvas::from_frame(&frame(40, 40));
        c.circle(20, 20, 8, MATCH, 1);
        for (x, y) in [(28, 20), (12, 20), (20, 28), (20, 12)] {
            assert_eq!(pixel(&c, x, y), MATCH, "missing at {x},{y}");
        }
        assert_eq!(pixel(&c, 20, 20), [0, 0, 0]); // outline, not filled
    }

    #[test]
    fn crosshair_arms_are_drawn_both_ways() {
        let mut c = Canvas::from_frame(&frame(60, 60));
        c.line(10, 30, 50, 30, BEST, 2);
        c.line(30, 10, 30, 50, BEST, 2);
        assert_eq!(pixel(&c, 10, 30), BEST);
        assert_eq!(pixel(&c, 50, 30), BEST);
        assert_eq!(pixel(&c, 30, 10), BEST);
        assert_eq!(pixel(&c, 30, 50), BEST);
    }

    #[test]
    fn shapes_clip_at_the_edges_rather_than_panicking() {
        let mut c = Canvas::from_frame(&frame(8, 8));
        c.rect(-5, -5, 20, 20, REGION, 3);
        c.circle(0, 0, 10, MATCH, 3);
        c.line(-30, 4, 30, 4, BEST, 3);
    }

    #[test]
    fn annotate_returns_decodable_png_base64() {
        let dets = vec![Detection {
            label: "Play".into(),
            x: 30,
            y: 20,
            w: 10,
            h: 6,
            confidence: 1.0,
            roi_offset: [0, 0],
        }];
        let b64 = annotate(&frame(64, 48), (4, 4, 60, 44), &dets);
        assert!(!b64.is_empty());
        let bytes = STANDARD.decode(&b64).expect("valid base64");
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "not a PNG");
    }

    #[test]
    fn annotate_without_matches_still_draws_the_region() {
        assert!(!annotate(&frame(32, 32), (2, 2, 30, 30), &[]).is_empty());
    }

    #[test]
    fn preview_factor_only_shrinks_oversized_frames() {
        assert_eq!(preview_factor(1280), 1);
        assert_eq!(preview_factor(1920), 2);
        assert_eq!(preview_factor(2560), 2);
        assert_eq!(preview_factor(3840), 3);
    }

    #[test]
    fn scaling_averages_each_block_and_halves_the_dimensions() {
        // 2x2 of one colour per block, so the average is that colour exactly.
        // Left block pure blue (BGR 255,0,0), right block pure red (BGR 0,0,255).
        let mut bgr = Vec::new();
        for _ in 0..2 {
            for _ in 0..2 {
                bgr.extend_from_slice(&[255, 0, 0]);
            }
            for _ in 0..2 {
                bgr.extend_from_slice(&[0, 0, 255]);
            }
        }
        let f = Frame { bgr, width: 4, height: 2 };
        let c = Canvas::from_frame_scaled(&f, 2);
        assert_eq!((c.width, c.height), (2, 1));
        assert_eq!(pixel(&c, 0, 0), [0, 0, 255]); // blue, as RGB
        assert_eq!(pixel(&c, 1, 0), [255, 0, 0]); // red, as RGB
    }

    #[test]
    fn scaling_by_one_is_a_plain_conversion() {
        let f = Frame { bgr: vec![10, 20, 30], width: 1, height: 1 };
        assert_eq!(pixel(&Canvas::from_frame_scaled(&f, 1), 0, 0), [30, 20, 10]);
    }
}
