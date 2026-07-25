//! In-process computer vision: the colour and template detection that used to
//! run in the Python `clawmation_vision` sidecar.
//!
//! The submodules are transcriptions of the OpenCV primitives that sidecar
//! called — [`image`] for colour conversion, morphology, contours and resizing,
//! [`clahe`] and [`canny`] for the two preprocessing passes, [`corr`] for
//! `matchTemplate`'s `TM_CCOEFF_NORMED`, and [`template`] for the multi-scale
//! robust matcher built on top of them. Thresholds users already tuned were
//! tuned against cv2's arithmetic, so those transcriptions reproduce its
//! fixed-point constants and rounding rather than being rewritten in floating
//! point.
//!
//! This module is the seam the rest of the app talks to. [`Detector`] owns the
//! template cache and the matcher's frame-to-frame memory, resolves percentage
//! regions against the screen, and answers the three questions the RPC used to:
//! does this guard fire ([`Detector::detect_guard`]), is the checkpoint on
//! screen ([`Detector::detect_checkpoint`]), what does this AI step see
//! ([`Detector::ai_detect`]).
//!
//! Two deliberate departures from the Python it replaces, both on paths that
//! were dead there:
//!
//! * Text guards never reach here. `hardware::ocr` reads them from the same
//!   frame in `core::Vision`, because the sidecar's EasyOCR path was excluded
//!   from the shipped build and every `method = "ocr"` guard failed on import.
//! * AI-step templates are loaded on demand. The monolith only ever called
//!   `load_template` for guards and checkpoints, so a step's `template` mode
//!   found nothing and its `features` mode reported "template not loaded" —
//!   a documented bug the port had no reason to carry forward. `features` shares
//!   the robust matcher, ORB having been dropped (see [`template`]).

mod canny;
mod clahe;
mod corr;
mod image;
mod template;

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use self::image::{bgr_to_gray, find_external_contours, hsv_in_range, morph_close, morph_open};
use self::template::{Matcher, Template};
use super::capture::Frame;
use crate::models::guard::Guard;
use crate::models::step::Step;
use crate::paths;

/// One detection. `x`/`y` are the match *centre* (not the top-left);
/// `roi_offset` is the region origin the coordinates are already absolute to.
/// The shape is still `Serialize`/`Deserialize` because it crosses the IPC
/// bridge to the editor in guard-test results.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Detection {
    pub label: String,
    pub x: i64,
    pub y: i64,
    pub w: i64,
    pub h: i64,
    pub confidence: f64,
    pub roi_offset: [i64; 2],
}

/// Why a vision call failed. Detection itself never fails — nothing found is an
/// empty vector, not an error — so these are all setup problems the user can act
/// on: a screen that would not capture, a template file that would not decode.
#[derive(Debug)]
pub enum VisionError {
    /// The capture backend returned no frame.
    Capture,
    /// An image file could not be read or decoded.
    Image(String),
}

impl std::fmt::Display for VisionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Capture => write!(f, "could not capture the screen"),
            Self::Image(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for VisionError {}

/// Percentage corners → pixel corners, the port of `config.py::Region.to_pixels`.
/// Truncating like Python's `int()`, so a region resolves to the same pixels the
/// sidecar would have cropped.
pub fn region_pixels(region: [f64; 4], w: i32, h: i32) -> (i32, i32, i32, i32) {
    let px = |v: f64, span: i32| (v * f64::from(span) / 100.0) as i32;
    (px(region[0], w), px(region[1], h), px(region[2], w), px(region[3], h))
}

/// Whether a region covers the whole frame, using `guard.py`'s exact tolerances.
/// A full region skips the crop entirely and searches the bare frame.
pub fn is_full_region(r: [f64; 4]) -> bool {
    r[0] <= 0.5 && r[1] <= 0.5 && r[2] >= 99.5 && r[3] >= 99.5
}

/// The search area for a region, plus its origin in screen pixels. Borrows the
/// frame whole when there is no region to crop to, as the Python did — a
/// full-screen "crop" is a wasted copy of several megabytes per poll.
fn roi<'a>(
    frame: &'a Frame,
    region: Option<[f64; 4]>,
    screen_w: i32,
    screen_h: i32,
) -> Option<(Cow<'a, Frame>, i64, i64)> {
    let Some(r) = region else {
        return Some((Cow::Borrowed(frame), 0, 0));
    };
    let (x1, y1, x2, y2) = region_pixels(r, screen_w, screen_h);
    frame
        .crop(x1, y1, x2 - x1, y2 - y1)
        .map(|c| (Cow::Owned(c), i64::from(x1), i64::from(y1)))
}

/// A guard's or step's HSV pair as the bounds `inRange` wants. The editor stores
/// them as plain integers, so out-of-gamut values are clamped rather than
/// wrapping the way an `as u8` cast would.
fn hsv_bounds(low: [i64; 3], high: [i64; 3]) -> ([u8; 3], [u8; 3]) {
    let clamp = |v: i64| v.clamp(0, 255) as u8;
    ([clamp(low[0]), clamp(low[1]), clamp(low[2])], [clamp(high[0]), clamp(high[1]), clamp(high[2])])
}

/// Decode an image file into a BGR [`Frame`] — `cv2.imread(path,
/// IMREAD_COLOR)`. Any alpha channel is dropped, as cv2 drops it. A `Frame` (and
/// not a bare buffer) because the template importer hands the result straight to
/// [`preview`](crate::hardware::preview) to re-encode and thumbnail. `::image` is
/// the crate; `self::image` is this module's own primitives.
pub fn read_frame(path: &Path) -> Result<Frame, VisionError> {
    let decoded = ::image::open(path)
        .map_err(|e| VisionError::Image(format!("could not read {}: {e}", path.display())))?;
    let rgb = decoded.to_rgb8();
    let (w, h) = (rgb.width(), rgb.height());
    let mut bgr = Vec::with_capacity((w * h * 3) as usize);
    for px in rgb.pixels() {
        bgr.extend_from_slice(&[px[2], px[1], px[0]]);
    }
    Ok(Frame { bgr, width: w, height: h })
}

/// Where a step's template lives. The step editor stores a bare file name; a
/// checkpoint stores the full path the picker returned.
fn template_path(name: &str) -> PathBuf {
    let p = Path::new(name);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        paths::templates_dir().join(name)
    }
}

/// The long-lived detector: template cache, matcher memory, and the screen
/// dimensions percentage regions resolve against.
pub struct Detector {
    screen_w: i32,
    screen_h: i32,
    /// Keyed by template file path, so replacing a guard's picture loads the new
    /// one instead of matching the old one until restart — which is what keying
    /// by guard id did.
    templates: HashMap<String, Template>,
    matcher: Matcher,
}

impl Detector {
    pub fn new(screen_w: i64, screen_h: i64) -> Self {
        Self {
            screen_w: screen_w as i32,
            screen_h: screen_h as i32,
            templates: HashMap::new(),
            matcher: Matcher::default(),
        }
    }

    /// Re-point the detector at a screen size — `init`'s `screen_w`/`screen_h`,
    /// which the config's target resolution can change between runs.
    pub fn set_screen(&mut self, screen_w: i64, screen_h: i64) {
        self.screen_w = screen_w as i32;
        self.screen_h = screen_h as i32;
    }

    /// Preprocess a template file into the cache, unless it is already there.
    /// Idempotent, so callers can front every match with it.
    fn ensure_template(&mut self, path: &Path) -> Result<String, VisionError> {
        let key = path.to_string_lossy().into_owned();
        if !self.templates.contains_key(&key) {
            let img = read_frame(path)?;
            if img.width == 0 || img.height == 0 {
                return Err(VisionError::Image(format!("{} has no pixels", path.display())));
            }
            let tpl = Template::from_bgr(&img.bgr, img.width as usize, img.height as usize);
            self.templates.insert(key.clone(), tpl);
            self.matcher.forget(&key);
        }
        Ok(key)
    }

    /// Drop a template so the next match re-reads it from disk — called when the
    /// picker overwrites a guard's picture.
    pub fn forget_template(&mut self, path: &Path) {
        let key = path.to_string_lossy().into_owned();
        self.templates.remove(&key);
        self.matcher.forget(&key);
    }

    /// Every HSV blob in `region` big enough to count, largest first — the port
    /// of `PixelDetector.detect_color`.
    pub fn detect_color(
        &self,
        frame: &Frame,
        region: Option<[f64; 4]>,
        lower: [u8; 3],
        upper: [u8; 3],
        min_area: f64,
        label: &str,
    ) -> Vec<Detection> {
        let Some((roi, ox, oy)) = roi(frame, region, self.screen_w, self.screen_h) else {
            return Vec::new();
        };
        let (w, h) = (roi.width as usize, roi.height as usize);
        let mask = hsv_in_range(&roi.bgr, w, h, lower, upper);
        // Open then close: the first drops speckle, the second seals the blob.
        let mask = morph_close(&morph_open(&mask, 1), 1);

        let mut out: Vec<Detection> = find_external_contours(&mask)
            .into_iter()
            .filter_map(|c| {
                let area = c.area();
                if area < min_area {
                    return None;
                }
                let (x, y, cw, ch) = c.bounding_rect();
                Some(Detection {
                    label: label.to_string(),
                    x: ox + i64::from(x) + i64::from(cw) / 2,
                    y: oy + i64::from(y) + i64::from(ch) / 2,
                    w: i64::from(cw),
                    h: i64::from(ch),
                    // The `+ 1` is Python's, and guards a zero-area contour.
                    confidence: (area / f64::from(cw * ch + 1)).min(1.0),
                    roi_offset: [ox, oy],
                })
            })
            .collect();
        // Biggest first. Stable, so equal-area blobs keep contour order — as
        // Python's `sort(reverse=True)` did.
        out.sort_by(|a, b| (b.w * b.h).cmp(&(a.w * a.h)));
        out
    }

    /// Find a template image in `region` — the three-tier robust match.
    pub fn match_robust(
        &mut self,
        frame: &Frame,
        path: &Path,
        region: Option<[f64; 4]>,
        threshold: f64,
        label: &str,
    ) -> Result<Vec<Detection>, VisionError> {
        let key = self.ensure_template(path)?;
        let Some((roi, ox, oy)) = roi(frame, region, self.screen_w, self.screen_h) else {
            return Ok(Vec::new());
        };
        let gray = bgr_to_gray(&roi.bgr, roi.width as usize, roi.height as usize);
        // Disjoint field borrows: the template is read out of `templates` while
        // the matcher's memory is written through `matcher`.
        let tpl = &self.templates[&key];
        Ok(self.matcher.robust(&gray, ox, oy, tpl, &key, label, threshold))
    }

    /// Run a guard's configured detection against a frame — the port of
    /// `guard.py::detect_guard`. `Err` means the guard's template file could not
    /// be read; every other outcome, including "nothing there", is an empty
    /// vector so one bad guard never aborts a poll cycle.
    pub fn detect_guard(
        &mut self,
        frame: &Frame,
        guard: &Guard,
    ) -> Result<Vec<Detection>, VisionError> {
        let region = if is_full_region(guard.region) { None } else { Some(guard.region) };
        match guard.method.as_str() {
            "template" => {
                if guard.template_path.is_empty() {
                    return Ok(Vec::new());
                }
                let path = PathBuf::from(&guard.template_path);
                self.match_robust(frame, &path, region, guard.threshold, &guard.name)
            }
            // Read in process by `hardware::ocr` before a guard reaches here.
            "ocr" | "text" => Ok(Vec::new()),
            // "color", and an empty method, which Python defaulted the same way.
            _ => {
                let (lo, hi) = hsv_bounds(guard.hsv_low, guard.hsv_high);
                Ok(self.detect_color(
                    frame,
                    region,
                    lo,
                    hi,
                    guard.min_area as f64,
                    &guard.name,
                ))
            }
        }
    }

    /// One poll of a vision checkpoint. The config is the macro event's `data`
    /// blob, and the defaults are the checkpoint ones (threshold 0.75, min area
    /// 40, label "checkpoint"), not `detect_color`'s.
    pub fn detect_checkpoint(&mut self, frame: &Frame, cfg: &Value) -> Vec<Detection> {
        let r = cfg
            .get("region")
            .and_then(Value::as_array)
            .filter(|a| a.len() == 4)
            .map(|a| {
                let mut out = [0.0; 4];
                for (slot, v) in out.iter_mut().zip(a) {
                    *slot = v.as_f64().unwrap_or(0.0);
                }
                out
            })
            .unwrap_or([0.0, 0.0, 100.0, 100.0]);
        let region = if is_full_region(r) { None } else { Some(r) };

        let method = cfg.get("method").and_then(Value::as_str).unwrap_or("color");
        let template = cfg.get("template").and_then(Value::as_str).unwrap_or("");
        if method == "template" && !template.is_empty() {
            let threshold = cfg.get("threshold").and_then(Value::as_f64).unwrap_or(0.75);
            return self
                .match_robust(frame, &template_path(template), region, threshold, "checkpoint")
                .unwrap_or_default();
        }

        let (lo, hi) = hsv_bounds(hsv_from(cfg, "hsv_low", [0, 0, 0]), hsv_from(cfg, "hsv_high", [179, 255, 255]));
        let min_area = cfg.get("min_area").and_then(Value::as_i64).unwrap_or(40);
        self.detect_color(frame, region, lo, hi, min_area as f64, "checkpoint")
    }

    /// One AI step's detection, with the status message the run log prints —
    /// the port of `steps.py::ai_detect`.
    pub fn ai_detect(&mut self, frame: &Frame, step: &Step) -> (Vec<Detection>, String) {
        let region = Some(step.region);
        if step.detect_mode == "color" {
            let (lo, hi) = hsv_bounds(step.hsv_low, step.hsv_high);
            let hits =
                self.detect_color(frame, region, lo, hi, step.min_area as f64, "target");
            let msg = format!("{} color match(es)", hits.len());
            return (hits, msg);
        }

        if step.template.is_empty() {
            return (Vec::new(), "no template selected".to_string());
        }
        // "features" was ORB in Python and is the same robust match here; the
        // wording stays so a saved step's run log reads as it always did.
        let noun = if step.detect_mode == "features" { "feature" } else { "robust" };
        match self.match_robust(
            frame,
            &template_path(&step.template),
            region,
            step.confidence,
            "target",
        ) {
            Ok(hits) => {
                let msg = format!("{} {noun} match(es)", hits.len());
                (hits, msg)
            }
            Err(_) => (Vec::new(), format!("template '{}' not loaded", step.template)),
        }
    }
}

/// A three-integer HSV bound out of a checkpoint config, falling back whole
/// rather than per-element — a config with a malformed triple gets the default
/// range, not a mix of the two.
fn hsv_from(cfg: &Value, key: &str, fallback: [i64; 3]) -> [i64; 3] {
    match cfg.get(key).and_then(Value::as_array).filter(|a| a.len() == 3) {
        Some(a) => {
            let mut out = fallback;
            for (slot, v) in out.iter_mut().zip(a) {
                *slot = v.as_i64().unwrap_or(*slot);
            }
            out
        }
        None => fallback,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A frame with one solid red square on black. Red is the easiest hue to
    /// bound without straddling the 179/0 wrap.
    fn frame_with_square(fw: u32, fh: u32, x: u32, y: u32, size: u32) -> Frame {
        let mut bgr = vec![0u8; (fw * fh * 3) as usize];
        for row in y..y + size {
            for col in x..x + size {
                let i = ((row * fw + col) * 3) as usize;
                bgr[i..i + 3].copy_from_slice(&[0, 0, 255]);
            }
        }
        Frame { bgr, width: fw, height: fh }
    }

    const RED_LOW: [u8; 3] = [0, 100, 100];
    const RED_HIGH: [u8; 3] = [10, 255, 255];

    #[test]
    fn a_colour_blob_is_found_at_its_centre() {
        let d = Detector::new(100, 100);
        let f = frame_with_square(100, 100, 20, 30, 10);
        let hits = d.detect_color(&f, None, RED_LOW, RED_HIGH, 40.0, "blob");
        assert_eq!(hits.len(), 1);
        assert_eq!((hits[0].x, hits[0].y), (25, 35));
        assert_eq!((hits[0].w, hits[0].h), (10, 10));
        assert_eq!(hits[0].label, "blob");
    }

    #[test]
    fn a_blob_under_the_minimum_area_is_dropped() {
        let d = Detector::new(100, 100);
        let f = frame_with_square(100, 100, 20, 30, 5);
        assert!(d.detect_color(&f, None, RED_LOW, RED_HIGH, 40.0, "blob").is_empty());
    }

    #[test]
    fn a_region_offsets_the_coordinates_back_to_screen_space() {
        let d = Detector::new(100, 100);
        let f = frame_with_square(100, 100, 60, 60, 10);
        // The right-bottom quarter, so the blob sits at (10, 10) inside the crop.
        let hits = d.detect_color(&f, Some([50.0, 50.0, 100.0, 100.0]), RED_LOW, RED_HIGH, 40.0, "b");
        assert_eq!(hits.len(), 1);
        assert_eq!((hits[0].x, hits[0].y), (65, 65));
        assert_eq!(hits[0].roi_offset, [50, 50]);
    }

    #[test]
    fn blobs_come_back_largest_first() {
        let mut f = frame_with_square(120, 60, 5, 5, 8);
        let big = frame_with_square(120, 60, 60, 20, 20);
        for (i, px) in big.bgr.chunks_exact(3).enumerate() {
            if px == [0, 0, 255] {
                f.bgr[i * 3..i * 3 + 3].copy_from_slice(&[0, 0, 255]);
            }
        }
        let d = Detector::new(120, 60);
        let hits = d.detect_color(&f, None, RED_LOW, RED_HIGH, 40.0, "b");
        assert_eq!(hits.len(), 2);
        assert!(hits[0].w * hits[0].h > hits[1].w * hits[1].h);
    }

    #[test]
    fn a_full_region_is_not_cropped() {
        assert!(is_full_region([0.0, 0.0, 100.0, 100.0]));
        assert!(is_full_region([0.5, 0.5, 99.5, 99.5]));
        assert!(!is_full_region([0.0, 0.0, 99.0, 100.0]));
    }

    #[test]
    fn a_region_resolves_by_truncation() {
        // 33.4% of 100 is 33.4 px, which Python's int() and Rust's cast both
        // take to 33.
        assert_eq!(region_pixels([33.4, 0.0, 66.7, 100.0], 100, 50), (33, 0, 66, 50));
    }

    #[test]
    fn a_guard_with_no_template_matches_nothing_rather_than_failing() {
        let mut d = Detector::new(100, 100);
        let g = Guard { method: "template".into(), ..Guard::default() };
        assert!(d.detect_guard(&frame_with_square(100, 100, 0, 0, 4), &g).unwrap().is_empty());
    }

    #[test]
    fn a_guard_whose_template_file_is_missing_reports_why() {
        let mut d = Detector::new(100, 100);
        let g = Guard {
            method: "template".into(),
            template_path: "no-such-template-file.png".into(),
            ..Guard::default()
        };
        let err = d.detect_guard(&frame_with_square(100, 100, 0, 0, 4), &g).unwrap_err();
        assert!(err.to_string().contains("no-such-template-file.png"), "got {err}");
    }

    #[test]
    fn a_colour_guard_uses_its_own_name_as_the_label() {
        let mut d = Detector::new(100, 100);
        let g = Guard {
            name: "Revive".into(),
            hsv_low: [0, 100, 100],
            hsv_high: [10, 255, 255],
            min_area: 40,
            ..Guard::default()
        };
        let hits = d.detect_guard(&frame_with_square(100, 100, 10, 10, 12), &g).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].label, "Revive");
    }

    #[test]
    fn a_checkpoint_falls_back_to_its_own_defaults() {
        let mut d = Detector::new(100, 100);
        let cfg = serde_json::json!({ "hsv_low": [0, 100, 100], "hsv_high": [10, 255, 255] });
        let hits = d.detect_checkpoint(&frame_with_square(100, 100, 40, 40, 10), &cfg);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].label, "checkpoint");
    }

    #[test]
    fn an_ai_colour_step_reports_its_count() {
        let mut d = Detector::new(100, 100);
        let step = Step {
            detect_mode: "color".into(),
            hsv_low: [0, 100, 100],
            hsv_high: [10, 255, 255],
            min_area: 40,
            ..Step::default()
        };
        let (hits, msg) = d.ai_detect(&frame_with_square(100, 100, 10, 10, 12), &step);
        assert_eq!(hits.len(), 1);
        assert_eq!(msg, "1 color match(es)");
    }

    #[test]
    fn an_ai_step_with_no_template_says_so() {
        let mut d = Detector::new(100, 100);
        let step = Step { detect_mode: "template".into(), ..Step::default() };
        let (hits, msg) = d.ai_detect(&frame_with_square(100, 100, 0, 0, 4), &step);
        assert!(hits.is_empty());
        assert_eq!(msg, "no template selected");
    }

    #[test]
    fn hsv_bounds_clamp_instead_of_wrapping() {
        let (lo, hi) = hsv_bounds([-5, 0, 0], [300, 255, 255]);
        assert_eq!(lo, [0, 0, 0]);
        assert_eq!(hi, [255, 255, 255]);
    }
}
