//! In-process computer vision: the colour and template detection that used to
//! run in the Python `clawmation_vision` sidecar.
//!
//! The submodules are transcriptions of the OpenCV primitives that sidecar
//! called: [`image`] for colour conversion, morphology, contours and resizing,
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
//!   found nothing and its `features` mode reported "template not loaded",
//!   a documented bug the port had no reason to carry forward. `features` shares
//!   the robust matcher, ORB having been dropped (see [`template`]).

mod canny;
mod clahe;
mod corr;
mod gpu_corr;
mod image;
mod template;

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use self::image::{
    bgr_to_gray, find_external_contours, hsv_in_range, morph_close, morph_open, Gray,
};
use self::template::{LearnedScaleSearch, Matcher, Template};
use super::capture::Frame;
use crate::engine::vision_runtime::FrameStamp;
use crate::models::guard::Guard;
use crate::models::step::Step;
use crate::models::vision_images::candidate_paths;
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
    #[serde(default = "unit_scale")]
    pub scale_x: f64,
    #[serde(default = "unit_scale")]
    pub scale_y: f64,
    /// Runtime-only provenance used to reject a match after a newer screen
    /// sample supersedes it. It is intentionally absent from IPC/persistence.
    #[serde(skip)]
    pub observation: Option<FrameStamp>,
}

fn unit_scale() -> f64 {
    1.0
}

/// Why a vision call failed. Detection itself never fails (nothing found is an
/// empty vector, not an error), so these are all setup problems the user can act
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
    (
        px(region[0], w),
        px(region[1], h),
        px(region[2], w),
        px(region[3], h),
    )
}

/// Whether a region covers the whole frame, using `guard.py`'s exact tolerances.
/// A full region skips the crop entirely and searches the bare frame.
pub fn is_full_region(r: [f64; 4]) -> bool {
    r[0] <= 0.5 && r[1] <= 0.5 && r[2] >= 99.5 && r[3] >= 99.5
}

/// The search area for a region, plus its origin in screen pixels. Borrows the
/// frame whole when there is no region to crop to, as the Python did: a
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
    (
        [clamp(low[0]), clamp(low[1]), clamp(low[2])],
        [clamp(high[0]), clamp(high[1]), clamp(high[2])],
    )
}

/// Decode an image file into a BGR [`Frame`]: `cv2.imread(path,
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
    Ok(Frame {
        bgr,
        width: w,
        height: h,
    })
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
#[derive(Clone)]
struct ReacquisitionState {
    last: Detection,
    scale_x: f64,
    scale_y: f64,
    source_key: String,
    candidate_keys: Vec<String>,
    bounds: [i64; 4],
    miss_count: u32,
    recovery_cursor: usize,
}

const REACQUISITION_PAD: i64 = 36;
const RECOVERY_TILES_PER_MISS: usize = 4;
const RECOVERY_TILE_WIDTH: i64 = 320;
const RECOVERY_TILE_HEIGHT: i64 = 180;
const RECOVERY_SCALES: [f64; 8] = [
    1.12,
    0.892_857_142_857,
    1.254_4,
    0.797_193_877_551,
    1.404_928,
    0.711_780_247_813,
    1.573_519_36,
    0.635_518_078_405,
];

fn global_probe_due(miss_count: u32) -> bool {
    miss_count > 0
}

fn recovery_scale(cursor: usize) -> f64 {
    RECOVERY_SCALES[cursor % RECOVERY_SCALES.len()]
}

fn recovery_grid(width: i64, height: i64) -> (usize, usize) {
    (
        ((width.max(1) + RECOVERY_TILE_WIDTH - 1) / RECOVERY_TILE_WIDTH) as usize,
        ((height.max(1) + RECOVERY_TILE_HEIGHT - 1) / RECOVERY_TILE_HEIGHT) as usize,
    )
}

fn recovery_probe(
    cursor: usize,
    width: i64,
    height: i64,
    overlap_x: i64,
    overlap_y: i64,
) -> ([i64; 4], f64) {
    let (columns, rows) = recovery_grid(width, height);
    let tile_count = columns * rows;
    let tile = cursor % tile_count;
    let pass = cursor / tile_count;
    let column = tile % columns;
    let row = tile / columns;

    let core_x1 = width * column as i64 / columns as i64;
    let core_y1 = height * row as i64 / rows as i64;
    let core_x2 = width * (column + 1) as i64 / columns as i64;
    let core_y2 = height * (row + 1) as i64 / rows as i64;
    let bounds = [
        (core_x1 - overlap_x).max(0),
        (core_y1 - overlap_y).max(0),
        (core_x2 + overlap_x).min(width),
        (core_y2 + overlap_y).min(height),
    ];
    let scale = if pass == 0 {
        1.0
    } else {
        recovery_scale(pass - 1)
    };
    (bounds, scale)
}

pub struct Detector {
    screen_w: i32,
    screen_h: i32,
    /// Keyed by template file path, so replacing a guard's picture loads the new
    /// one instead of matching the old one until restart, which is what keying
    /// by guard id did.
    templates: HashMap<String, Template>,
    preferred_templates: HashMap<String, String>,
    reacquisition: HashMap<String, ReacquisitionState>,
    matcher: Matcher,
}

impl Detector {
    pub fn new(screen_w: i64, screen_h: i64) -> Self {
        // Adapter discovery and shader compilation are asynchronous. Starting
        // them with the detector keeps the user's first real search off the
        // one-time CPU/GPU cold-start path.
        gpu_corr::warm_up();
        Self {
            screen_w: screen_w as i32,
            screen_h: screen_h as i32,
            templates: HashMap::new(),
            preferred_templates: HashMap::new(),
            reacquisition: HashMap::new(),
            matcher: Matcher::default(),
        }
    }

    /// Re-point the detector at a screen size: `init`'s `screen_w`/`screen_h`,
    /// which the config's target resolution can change between runs.
    pub fn set_screen(&mut self, screen_w: i64, screen_h: i64) {
        let (screen_w, screen_h) = (screen_w as i32, screen_h as i32);
        if self.screen_w != screen_w || self.screen_h != screen_h {
            self.screen_w = screen_w;
            self.screen_h = screen_h;
            self.reacquisition.clear();
            self.matcher = Matcher::default();
        }
    }

    /// Preprocess a template file into the cache, unless it is already there.
    /// Idempotent, so callers can front every match with it.
    fn ensure_template(&mut self, path: &Path) -> Result<String, VisionError> {
        let key = path.to_string_lossy().into_owned();
        if !self.templates.contains_key(&key) {
            let img = read_frame(path)?;
            if img.width == 0 || img.height == 0 {
                return Err(VisionError::Image(format!(
                    "{} has no pixels",
                    path.display()
                )));
            }
            let tpl = Template::from_bgr(&img.bgr, img.width as usize, img.height as usize);
            self.templates.insert(key.clone(), tpl);
            self.matcher.forget(&key);
        }
        Ok(key)
    }

    /// Drop a template so the next match re-reads it from disk, called when the
    /// picker overwrites a guard's picture.
    pub fn forget_template(&mut self, path: &Path) {
        let key = path.to_string_lossy().into_owned();
        self.templates.remove(&key);
        self.matcher.forget(&key);
        self.preferred_templates
            .retain(|_, preferred| preferred != &key);
        self.reacquisition
            .retain(|_, state| !state.candidate_keys.contains(&key));
    }

    fn remember_reacquisition(
        &mut self,
        operation_key: &str,
        candidate_keys: &[String],
        bounds: [i64; 4],
        source_key: &str,
        detection: &Detection,
    ) {
        let Some(template) = self.templates.get(source_key) else {
            return;
        };
        let recovery_cursor = self
            .reacquisition
            .get(operation_key)
            .map_or(0, |state| state.recovery_cursor);
        self.matcher.remember_detection(source_key, detection);
        self.preferred_templates
            .insert(operation_key.to_string(), source_key.to_string());
        self.reacquisition.insert(
            operation_key.to_string(),
            ReacquisitionState {
                last: detection.clone(),
                scale_x: detection.w as f64 / template.w.max(1) as f64,
                scale_y: detection.h as f64 / template.h.max(1) as f64,
                source_key: source_key.to_string(),
                candidate_keys: candidate_keys.to_vec(),
                bounds,
                miss_count: 0,
                recovery_cursor,
            },
        );
    }

    fn annotate_action_scale(
        &self,
        detection: &mut Detection,
        candidate_keys: &[String],
        source_key: &str,
    ) {
        // Click marks are authored against the first (canonical) image. Hover
        // alternatives may have slightly different intrinsic dimensions, so
        // prefer the canonical candidate and fall back to the winning source.
        let reference = candidate_keys
            .iter()
            .find_map(|key| self.templates.get(key))
            .or_else(|| self.templates.get(source_key));
        if let Some(template) = reference {
            detection.scale_x = detection.w as f64 / template.w.max(1) as f64;
            detection.scale_y = detection.h as f64 / template.h.max(1) as f64;
        }
    }

    /// Every HSV blob in `region` big enough to count, largest first. The port
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
                    scale_x: 1.0,
                    scale_y: 1.0,
                    observation: None,
                })
            })
            .collect();
        // Biggest first. Stable, so equal-area blobs keep contour order, as
        // Python's `sort(reverse=True)` did.
        out.sort_by(|a, b| (b.w * b.h).cmp(&(a.w * a.h)));
        out
    }

    /// Find a template image in `region`: the three-tier robust match.
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
        let mut hits = self
            .matcher
            .robust(&gray, ox, oy, tpl, &key, label, threshold);
        for hit in &mut hits {
            hit.scale_x = hit.w as f64 / tpl.w.max(1) as f64;
            hit.scale_y = hit.h as f64 / tpl.h.max(1) as f64;
        }
        Ok(hits)
    }

    fn match_robust_candidates(
        &mut self,
        frame: &Frame,
        paths: &[PathBuf],
        region: Option<[f64; 4]>,
        threshold: f64,
        label: &str,
        operation_key: &str,
    ) -> Result<Vec<Detection>, VisionError> {
        let Some((roi, ox, oy)) = roi(frame, region, self.screen_w, self.screen_h) else {
            self.reacquisition.remove(operation_key);
            return Ok(Vec::new());
        };
        let bounds = [ox, oy, i64::from(roi.width), i64::from(roi.height)];
        let candidate_fingerprint = paths
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        let state_is_valid = self.reacquisition.get(operation_key).is_some_and(|state| {
            state.candidate_keys == candidate_fingerprint
                && state.bounds == bounds
                && self.templates.contains_key(&state.source_key)
        });
        if !state_is_valid {
            self.reacquisition.remove(operation_key);
        }

        let preferred = self.preferred_templates.get(operation_key).cloned();
        let mut order = (0..paths.len()).collect::<Vec<_>>();
        if let Some(preferred) = &preferred {
            order.sort_by_key(|index| {
                paths[*index].to_string_lossy().as_ref() != preferred.as_str()
            });
        }

        let mut loaded = Vec::with_capacity(paths.len());
        let mut first_error = None;
        for index in order {
            let path = &paths[index];
            let cache_key = match self.ensure_template(path) {
                Ok(key) => key,
                Err(error) => {
                    first_error.get_or_insert(error);
                    continue;
                }
            };
            loaded.push(cache_key);
        }

        if loaded.is_empty() {
            self.reacquisition.remove(operation_key);
            return Err(first_error.unwrap_or_else(|| {
                VisionError::Image("no usable image candidates are configured".to_string())
            }));
        }

        if let Some(state) = self.reacquisition.get(operation_key).cloned() {
            let target_sizes = loaded
                .iter()
                .map(|key| {
                    let template = &self.templates[key];
                    (
                        (template.w as f64 * state.scale_x).round().max(2.0) as i64,
                        (template.h as f64 * state.scale_y).round().max(2.0) as i64,
                    )
                })
                .collect::<Vec<_>>();
            let max_w = target_sizes
                .iter()
                .map(|(w, _)| *w)
                .max()
                .unwrap_or(state.last.w);
            let max_h = target_sizes
                .iter()
                .map(|(_, h)| *h)
                .max()
                .unwrap_or(state.last.h);
            let pad_x = REACQUISITION_PAD.max(max_w / 2);
            let pad_y = REACQUISITION_PAD.max(max_h / 2);
            let local_cx = state.last.x - ox;
            let local_cy = state.last.y - oy;
            let x1 = (local_cx - max_w / 2 - pad_x).max(0);
            let y1 = (local_cy - max_h / 2 - pad_y).max(0);
            let x2 = (local_cx + max_w / 2 + pad_x).min(i64::from(roi.width));
            let y2 = (local_cy + max_h / 2 + pad_y).min(i64::from(roi.height));

            if let Some(hot) = roi.crop(x1 as i32, y1 as i32, (x2 - x1) as i32, (y2 - y1) as i32) {
                let hot_gray = bgr_to_gray(&hot.bgr, hot.width as usize, hot.height as usize);
                for (index, cache_key) in loaded.iter().enumerate() {
                    let template = &self.templates[cache_key];
                    let (target_w, target_h) = target_sizes[index];
                    let mut hits = self.matcher.focused(
                        &hot_gray,
                        ox + x1,
                        oy + y1,
                        template,
                        cache_key,
                        label,
                        threshold,
                        target_w,
                        target_h,
                    );
                    if let Some(hit) = hits.first_mut() {
                        hit.roi_offset = [ox, oy];
                        let mut hit = hit.clone();
                        self.annotate_action_scale(&mut hit, &candidate_fingerprint, cache_key);
                        self.remember_reacquisition(
                            operation_key,
                            &candidate_fingerprint,
                            bounds,
                            cache_key,
                            &hit,
                        );
                        return Ok(vec![hit]);
                    }
                }
            }

            let recovery_cursors = {
                let state = self
                    .reacquisition
                    .get_mut(operation_key)
                    .expect("validated reacquisition state remains present");
                state.miss_count = state.miss_count.saturating_add(1);
                if global_probe_due(state.miss_count) {
                    let first = state.recovery_cursor;
                    state.recovery_cursor =
                        state.recovery_cursor.wrapping_add(RECOVERY_TILES_PER_MISS);
                    Some(
                        (0..RECOVERY_TILES_PER_MISS)
                            .map(|offset| first.wrapping_add(offset))
                            .collect::<Vec<_>>(),
                    )
                } else {
                    None
                }
            };
            let Some(recovery_cursors) = recovery_cursors else {
                return Ok(Vec::new());
            };

            // A missing full-screen target used to make one recovery pass scan
            // every pixel at every appearance and two scales. On a 1440p
            // desktop that blocked fresh captures for more than half a second,
            // so a target returning at its known position could not be seen.
            // Sweep a small bounded tile batch at one scale per miss. The hot
            // zone above still runs first, while a 1440p desktop completes its
            // same-scale spatial recovery in sixteen misses instead of 192.
            for recovery_cursor in recovery_cursors {
                let ([x1, y1, x2, y2], recovery_multiplier) = recovery_probe(
                    recovery_cursor,
                    i64::from(roi.width),
                    i64::from(roi.height),
                    (max_w + 1) / 2,
                    (max_h + 1) / 2,
                );
                let Some(probe) =
                    roi.crop(x1 as i32, y1 as i32, (x2 - x1) as i32, (y2 - y1) as i32)
                else {
                    continue;
                };
                let gray = bgr_to_gray(&probe.bgr, probe.width as usize, probe.height as usize);
                let recovery_sizes = target_sizes
                    .iter()
                    .map(|(target_w, target_h)| {
                        (
                            (*target_w as f64 * recovery_multiplier).round().max(2.0) as i64,
                            (*target_h as f64 * recovery_multiplier).round().max(2.0) as i64,
                        )
                    })
                    .collect::<Vec<_>>();
                let target_min_side = recovery_sizes
                    .iter()
                    .map(|(w, h)| (*w).min(*h) as usize)
                    .min()
                    .unwrap_or(2);
                let learned = LearnedScaleSearch::new(&gray, ox + x1, oy + y1, target_min_side);
                for (index, cache_key) in loaded.iter().enumerate() {
                    let template = &self.templates[cache_key];
                    let (target_w, target_h) = recovery_sizes[index];
                    let hits = self.matcher.learned(
                        &learned, template, cache_key, label, threshold, target_w, target_h,
                    );
                    if let Some(hit) = hits.first() {
                        let mut hit = hit.clone();
                        self.annotate_action_scale(&mut hit, &candidate_fingerprint, cache_key);
                        self.remember_reacquisition(
                            operation_key,
                            &candidate_fingerprint,
                            bounds,
                            cache_key,
                            &hit,
                        );
                        return Ok(vec![hit]);
                    }
                }
            }
            return Ok(Vec::new());
        }

        let gray: Gray = bgr_to_gray(&roi.bgr, roi.width as usize, roi.height as usize);
        for cache_key in loaded {
            let template = &self.templates[&cache_key];
            let hits = self
                .matcher
                .robust(&gray, ox, oy, template, &cache_key, label, threshold);
            if let Some(hit) = hits.first() {
                let mut hit = hit.clone();
                self.annotate_action_scale(&mut hit, &candidate_fingerprint, &cache_key);
                self.remember_reacquisition(
                    operation_key,
                    &candidate_fingerprint,
                    bounds,
                    &cache_key,
                    &hit,
                );
                return Ok(vec![hit]);
            }
        }
        Ok(Vec::new())
    }

    /// Run a guard's configured detection against a frame: the port of
    /// `guard.py::detect_guard`. `Err` means the guard's template file could not
    /// be read; every other outcome, including "nothing there", is an empty
    /// vector so one bad guard never aborts a poll cycle.
    pub fn detect_guard(
        &mut self,
        frame: &Frame,
        guard: &Guard,
    ) -> Result<Vec<Detection>, VisionError> {
        let region = if is_full_region(guard.region) {
            None
        } else {
            Some(guard.region)
        };
        match guard.method.as_str() {
            "template" => {
                let candidates = candidate_paths(&guard.template_path, &guard.template_paths);
                if candidates.is_empty() {
                    return Ok(Vec::new());
                }
                let paths = candidates
                    .into_iter()
                    .map(PathBuf::from)
                    .collect::<Vec<_>>();
                let identity = if guard.id.trim().is_empty() {
                    guard.name.as_str()
                } else {
                    guard.id.as_str()
                };
                self.match_robust_candidates(
                    frame,
                    &paths,
                    region,
                    guard.threshold,
                    &guard.name,
                    &format!("guard:{identity}"),
                )
            }
            // Read in process by `hardware::ocr` before a guard reaches here.
            "ocr" | "text" => Ok(Vec::new()),
            // "color", and an empty method, which Python defaulted the same way.
            _ => {
                let (lo, hi) = hsv_bounds(guard.hsv_low, guard.hsv_high);
                Ok(self.detect_color(frame, region, lo, hi, guard.min_area as f64, &guard.name))
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
                .match_robust(
                    frame,
                    &template_path(template),
                    region,
                    threshold,
                    "checkpoint",
                )
                .unwrap_or_default();
        }

        let (lo, hi) = hsv_bounds(
            hsv_from(cfg, "hsv_low", [0, 0, 0]),
            hsv_from(cfg, "hsv_high", [179, 255, 255]),
        );
        let min_area = cfg.get("min_area").and_then(Value::as_i64).unwrap_or(40);
        self.detect_color(frame, region, lo, hi, min_area as f64, "checkpoint")
    }

    /// One AI step's detection, with the status message the run log prints.
    /// The port of `steps.py::ai_detect`.
    pub fn ai_detect(&mut self, frame: &Frame, step: &Step) -> (Vec<Detection>, String) {
        let region = (!is_full_region(step.region)).then_some(step.region);
        if step.detect_mode == "color" {
            let (lo, hi) = hsv_bounds(step.hsv_low, step.hsv_high);
            let hits = self.detect_color(frame, region, lo, hi, step.min_area as f64, "target");
            let msg = format!("{} color match(es)", hits.len());
            return (hits, msg);
        }

        let candidates = candidate_paths(&step.template, &step.templates);
        if candidates.is_empty() {
            return (Vec::new(), "no template selected".to_string());
        }
        // "features" was ORB in Python and is the same robust match here; the
        // wording stays so a saved step's run log reads as it always did.
        let noun = if step.detect_mode == "features" {
            "feature"
        } else {
            "robust"
        };
        let paths = candidates
            .into_iter()
            .map(template_path)
            .collect::<Vec<_>>();
        match self.match_robust_candidates(
            frame,
            &paths,
            region,
            step.confidence,
            "target",
            &format!("step:{}", step.id),
        ) {
            Ok(hits) => {
                let msg = format!("{} {noun} match(es)", hits.len());
                (hits, msg)
            }
            Err(error) => (Vec::new(), format!("template not loaded: {error}")),
        }
    }
}

/// A three-integer HSV bound out of a checkpoint config, falling back whole
/// rather than per-element: a config with a malformed triple gets the default
/// range, not a mix of the two.
fn hsv_from(cfg: &Value, key: &str, fallback: [i64; 3]) -> [i64; 3] {
    match cfg
        .get(key)
        .and_then(Value::as_array)
        .filter(|a| a.len() == 3)
    {
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
    use super::image::{resize, Interp};
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
        Frame {
            bgr,
            width: fw,
            height: fh,
        }
    }

    fn button_image(seed: usize) -> Gray {
        let mut image = Gray::from_vec(32, 18, vec![25; 32 * 18]);
        for y in 2..16 {
            for x in 2..30 {
                let border = x == 2 || x == 29 || y == 2 || y == 15;
                let glyph = (x + seed) % 7 < 2 && (6..12).contains(&y);
                image.set(
                    x,
                    y,
                    if border {
                        220
                    } else if glyph {
                        35
                    } else {
                        120 + ((x * 11 + y * 5 + seed * 17) % 100) as u8
                    },
                );
            }
        }
        image
    }

    fn frame_with_image(patch: &Gray, px: usize, py: usize) -> Frame {
        let (width, height) = (120usize, 80usize);
        let mut gray = Gray::new(width, height);
        for y in 0..height {
            for x in 0..width {
                gray.set(x, y, ((x * 3 + y * 5) % 50) as u8);
            }
        }
        for y in 0..patch.h {
            for x in 0..patch.w {
                gray.set(px + x, py + y, patch.at(x, y));
            }
        }
        let mut bgr = Vec::with_capacity(width * height * 3);
        for value in gray.data {
            bgr.extend_from_slice(&[value, value, value]);
        }
        Frame {
            bgr,
            width: width as u32,
            height: height as u32,
        }
    }

    fn background_frame(width: usize, height: usize) -> Frame {
        let mut bgr = Vec::with_capacity(width * height * 3);
        for y in 0..height {
            for x in 0..width {
                let value = ((x * 3 + y * 5) % 50) as u8;
                bgr.extend_from_slice(&[value, value, value]);
            }
        }
        Frame {
            bgr,
            width: width as u32,
            height: height as u32,
        }
    }

    #[test]
    fn progressive_recovery_tiles_cover_every_target_center() {
        let (width, height) = (2_560_i64, 1_440_i64);
        let (target_w, target_h) = (229_i64, 64_i64);
        let (columns, rows) = recovery_grid(width, height);
        let tile_count = columns * rows;
        let tiles = (0..tile_count)
            .map(|cursor| {
                recovery_probe(
                    cursor,
                    width,
                    height,
                    (target_w + 1) / 2,
                    (target_h + 1) / 2,
                )
                .0
            })
            .collect::<Vec<_>>();

        for y in (target_h / 2..height - target_h / 2).step_by(31) {
            for x in (target_w / 2..width - target_w / 2).step_by(31) {
                assert!(
                    tiles.iter().any(|[x1, y1, x2, y2]| {
                        x - target_w / 2 >= *x1
                            && y - target_h / 2 >= *y1
                            && x + target_w / 2 <= *x2
                            && y + target_h / 2 <= *y2
                    }),
                    "target centred at ({x}, {y}) crosses every recovery tile"
                );
            }
        }
    }

    #[test]
    fn progressive_recovery_finishes_the_learned_scale_before_scale_search() {
        let (columns, rows) = recovery_grid(2_560, 1_440);
        let tile_count = columns * rows;
        assert_eq!(recovery_probe(tile_count - 1, 2_560, 1_440, 0, 0).1, 1.0);
        assert_eq!(
            recovery_probe(tile_count, 2_560, 1_440, 0, 0).1,
            RECOVERY_SCALES[0]
        );
    }

    #[test]
    fn same_scale_spatial_recovery_has_a_fixed_miss_bound() {
        let (columns, rows) = recovery_grid(2_560, 1_440);
        let tile_count = columns * rows;
        let misses = tile_count.div_ceil(RECOVERY_TILES_PER_MISS);
        assert_eq!(tile_count, 64);
        assert_eq!(misses, 16);
    }

    fn frame_with_image_at(
        width: usize,
        height: usize,
        patch: &Gray,
        px: usize,
        py: usize,
    ) -> Frame {
        let mut frame = background_frame(width, height);
        for y in 0..patch.h {
            for x in 0..patch.w {
                let value = patch.at(x, y);
                let index = ((py + y) * width + px + x) * 3;
                frame.bgr[index..index + 3].copy_from_slice(&[value, value, value]);
            }
        }
        frame
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
        assert!(d
            .detect_color(&f, None, RED_LOW, RED_HIGH, 40.0, "blob")
            .is_empty());
    }

    #[test]
    fn a_region_offsets_the_coordinates_back_to_screen_space() {
        let d = Detector::new(100, 100);
        let f = frame_with_square(100, 100, 60, 60, 10);
        // The right-bottom quarter, so the blob sits at (10, 10) inside the crop.
        let hits = d.detect_color(
            &f,
            Some([50.0, 50.0, 100.0, 100.0]),
            RED_LOW,
            RED_HIGH,
            40.0,
            "b",
        );
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
        assert_eq!(
            region_pixels([33.4, 0.0, 66.7, 100.0], 100, 50),
            (33, 0, 66, 50)
        );
    }

    #[test]
    fn a_guard_with_no_template_matches_nothing_rather_than_failing() {
        let mut d = Detector::new(100, 100);
        let g = Guard {
            method: "template".into(),
            ..Guard::default()
        };
        assert!(d
            .detect_guard(&frame_with_square(100, 100, 0, 0, 4), &g)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn a_guard_whose_template_file_is_missing_reports_why() {
        let mut d = Detector::new(100, 100);
        let g = Guard {
            method: "template".into(),
            template_path: "no-such-template-file.png".into(),
            ..Guard::default()
        };
        let err = d
            .detect_guard(&frame_with_square(100, 100, 0, 0, 4), &g)
            .unwrap_err();
        assert!(
            err.to_string().contains("no-such-template-file.png"),
            "got {err}"
        );
    }

    #[test]
    fn normal_and_hovered_images_use_or_semantics() {
        let normal = button_image(1);
        let hovered = button_image(9);
        let mut detector = Detector::new(120, 80);
        detector
            .templates
            .insert("normal".into(), Template::from_gray(&normal));
        detector
            .templates
            .insert("hovered".into(), Template::from_gray(&hovered));
        let guard = Guard {
            id: "open-button".into(),
            name: "Open".into(),
            method: "template".into(),
            template_path: "normal".into(),
            template_paths: vec!["hovered".into()],
            threshold: 0.9,
            ..Guard::default()
        };

        let hits = detector
            .detect_guard(&frame_with_image(&hovered, 48, 26), &guard)
            .unwrap();

        assert_eq!(hits.len(), 1, "one hovered target should be one match");
        assert!((hits[0].x - 64).abs() <= 2);
        assert!((hits[0].y - 35).abs() <= 2);
        assert_eq!(detector.preferred_templates["guard:open-button"], "hovered");
    }

    #[test]
    fn an_unreadable_candidate_does_not_hide_a_valid_alternative() {
        let hovered = button_image(13);
        let mut detector = Detector::new(120, 80);
        detector
            .templates
            .insert("valid".into(), Template::from_gray(&hovered));
        let guard = Guard {
            id: "mixed".into(),
            method: "template".into(),
            template_path: "definitely-missing-template.png".into(),
            template_paths: vec!["valid".into()],
            threshold: 0.9,
            ..Guard::default()
        };

        let hits = detector
            .detect_guard(&frame_with_image(&hovered, 20, 30), &guard)
            .unwrap();

        assert_eq!(hits.len(), 1);
        assert_eq!(detector.preferred_templates["guard:mixed"], "valid");
    }

    #[test]
    fn equivalent_candidates_return_only_the_strongest_candidate_result() {
        let button = button_image(17);
        let mut detector = Detector::new(120, 80);
        detector
            .templates
            .insert("primary".into(), Template::from_gray(&button));
        detector
            .templates
            .insert("alternative".into(), Template::from_gray(&button));
        let guard = Guard {
            id: "same-target".into(),
            method: "template".into(),
            template_path: "primary".into(),
            template_paths: vec!["alternative".into()],
            threshold: 0.9,
            ..Guard::default()
        };

        let hits = detector
            .detect_guard(&frame_with_image(&button, 44, 24), &guard)
            .unwrap();

        assert_eq!(
            hits.len(),
            1,
            "alternative appearances must not duplicate the action-worthy match"
        );
    }

    #[test]
    fn an_accepted_preferred_candidate_skips_stronger_alternatives() {
        let preferred = button_image(1);
        let exact_alternative = button_image(17);
        let mut detector = Detector::new(120, 80);
        detector
            .templates
            .insert("preferred".into(), Template::from_gray(&preferred));
        detector.templates.insert(
            "exact-alternative".into(),
            Template::from_gray(&exact_alternative),
        );
        detector
            .preferred_templates
            .insert("test:short-circuit".into(), "preferred".into());

        let hits = detector
            .match_robust_candidates(
                &frame_with_image(&exact_alternative, 44, 24),
                &[
                    PathBuf::from("preferred"),
                    PathBuf::from("exact-alternative"),
                ],
                None,
                0.5,
                "target",
                "test:short-circuit",
            )
            .unwrap();

        assert!(
            !hits.is_empty(),
            "the preferred appearance should be accepted"
        );
        assert!(
            hits[0].confidence < 0.98,
            "the exact alternative was evaluated instead of short-circuiting: {}",
            hits[0].confidence
        );
        assert_eq!(
            detector.preferred_templates["test:short-circuit"],
            "preferred"
        );
    }

    #[test]
    fn a_missing_preferred_candidate_falls_through_to_a_valid_alternative() {
        let preferred = button_image(1);
        let alternative = button_image(17);
        let mut detector = Detector::new(120, 80);
        detector
            .templates
            .insert("preferred".into(), Template::from_gray(&preferred));
        detector
            .templates
            .insert("alternative".into(), Template::from_gray(&alternative));
        detector
            .preferred_templates
            .insert("test:fallback".into(), "preferred".into());

        let hits = detector
            .match_robust_candidates(
                &frame_with_image(&alternative, 44, 24),
                &[PathBuf::from("preferred"), PathBuf::from("alternative")],
                None,
                0.95,
                "target",
                "test:fallback",
            )
            .unwrap();

        assert!(!hits.is_empty());
        assert_eq!(detector.preferred_templates["test:fallback"], "alternative");
    }

    #[test]
    fn a_first_detection_stops_at_the_first_accepted_appearance() {
        let first = button_image(1);
        let exact_alternative = button_image(17);
        let mut detector = Detector::new(120, 80);
        detector
            .templates
            .insert("first".into(), Template::from_gray(&first));
        detector.templates.insert(
            "exact-alternative".into(),
            Template::from_gray(&exact_alternative),
        );

        let hits = detector
            .match_robust_candidates(
                &frame_with_image(&exact_alternative, 44, 24),
                &[PathBuf::from("first"), PathBuf::from("exact-alternative")],
                None,
                0.5,
                "target",
                "test:first-accepted",
            )
            .unwrap();

        assert!(!hits.is_empty());
        assert!(
            hits[0].confidence < 0.98,
            "the exact second appearance was evaluated: {}",
            hits[0].confidence
        );
        assert_eq!(detector.preferred_templates["test:first-accepted"], "first");
    }

    #[test]
    fn a_hovered_reappearance_reuses_the_normal_appearance_anchor() {
        let normal = button_image(1);
        let hovered = button_image(9);
        let mut detector = Detector::new(120, 80);
        detector
            .templates
            .insert("normal".into(), Template::from_gray(&normal));
        detector
            .templates
            .insert("hovered".into(), Template::from_gray(&hovered));
        let paths = [PathBuf::from("normal"), PathBuf::from("hovered")];

        let first = detector
            .match_robust_candidates(
                &frame_with_image(&normal, 44, 24),
                &paths,
                None,
                0.9,
                "target",
                "test:appearance-anchor",
            )
            .unwrap();
        assert_eq!(first.len(), 1);
        assert!(detector
            .match_robust_candidates(
                &background_frame(120, 80),
                &paths,
                None,
                0.9,
                "target",
                "test:appearance-anchor",
            )
            .unwrap()
            .is_empty());

        let reappeared = detector
            .match_robust_candidates(
                &frame_with_image(&hovered, 44, 24),
                &paths,
                None,
                0.9,
                "target",
                "test:appearance-anchor",
            )
            .unwrap();

        assert_eq!(reappeared.len(), 1);
        assert_eq!(
            detector.preferred_templates["test:appearance-anchor"],
            "hovered"
        );
        assert_eq!(
            detector.reacquisition["test:appearance-anchor"].source_key,
            "hovered"
        );
    }

    #[test]
    fn a_scale_change_eventually_uses_staggered_bounded_recovery() {
        let normal = button_image(5);
        let enlarged = resize(&normal, 48, 27, Interp::Linear);
        let mut detector = Detector::new(120, 80);
        detector
            .templates
            .insert("normal".into(), Template::from_gray(&normal));
        let paths = [PathBuf::from("normal")];

        assert!(!detector
            .match_robust_candidates(
                &frame_with_image(&normal, 12, 18),
                &paths,
                None,
                0.8,
                "target",
                "test:scale-recovery",
            )
            .unwrap()
            .is_empty());

        let changed = frame_with_image(&enlarged, 60, 30);
        let mut recovered = Vec::new();
        for _ in 0..24 {
            recovered = detector
                .match_robust_candidates(
                    &changed,
                    &paths,
                    None,
                    0.8,
                    "target",
                    "test:scale-recovery",
                )
                .unwrap();
            if !recovered.is_empty() {
                break;
            }
        }

        assert!(
            !recovered.is_empty(),
            "bounded scale recovery never found it"
        );
        assert!(
            detector.reacquisition["test:scale-recovery"].scale_x > 1.3,
            "the learned scale was not refreshed"
        );
    }

    #[test]
    fn screen_region_candidate_and_template_changes_invalidate_tracking() {
        let normal = button_image(3);
        let hovered = button_image(11);
        let mut detector = Detector::new(120, 80);
        detector
            .templates
            .insert("normal".into(), Template::from_gray(&normal));
        detector
            .templates
            .insert("hovered".into(), Template::from_gray(&hovered));

        detector
            .match_robust_candidates(
                &frame_with_image(&normal, 20, 20),
                &[PathBuf::from("normal")],
                None,
                0.9,
                "target",
                "test:invalidate",
            )
            .unwrap();
        assert!(detector.reacquisition.contains_key("test:invalidate"));

        detector.set_screen(121, 80);
        assert!(detector.reacquisition.is_empty());
        detector.set_screen(120, 80);

        detector
            .match_robust_candidates(
                &frame_with_image(&normal, 20, 20),
                &[PathBuf::from("normal")],
                None,
                0.9,
                "target",
                "test:invalidate",
            )
            .unwrap();
        detector
            .match_robust_candidates(
                &background_frame(120, 80),
                &[PathBuf::from("normal"), PathBuf::from("hovered")],
                None,
                0.9,
                "target",
                "test:invalidate",
            )
            .unwrap();
        assert!(!detector.reacquisition.contains_key("test:invalidate"));

        detector
            .match_robust_candidates(
                &frame_with_image(&normal, 20, 20),
                &[PathBuf::from("normal")],
                None,
                0.9,
                "target",
                "test:invalidate",
            )
            .unwrap();
        detector
            .match_robust_candidates(
                &background_frame(120, 80),
                &[PathBuf::from("normal")],
                Some([0.0, 0.0, 80.0, 100.0]),
                0.9,
                "target",
                "test:invalidate",
            )
            .unwrap();
        assert!(!detector.reacquisition.contains_key("test:invalidate"));

        detector
            .match_robust_candidates(
                &frame_with_image(&normal, 20, 20),
                &[PathBuf::from("normal")],
                None,
                0.9,
                "target",
                "test:invalidate",
            )
            .unwrap();
        detector.forget_template(Path::new("normal"));
        assert!(!detector.reacquisition.contains_key("test:invalidate"));
    }

    #[test]
    fn reacquisition_advances_global_recovery_on_every_miss() {
        assert!(global_probe_due(1));
        assert!(global_probe_due(2));
        assert!(global_probe_due(3));
        assert!(global_probe_due(4));
        assert!(global_probe_due(6));
    }

    #[test]
    fn staggered_scale_recovery_walks_outward_without_a_broad_sweep() {
        let first = (0..8).map(recovery_scale).collect::<Vec<_>>();
        assert_eq!(first[0], 1.12);
        assert!(first.iter().any(|scale| *scale >= 1.5));
        assert!(first.iter().any(|scale| *scale <= 0.75));
        assert_eq!(recovery_scale(0), recovery_scale(first.len()));
    }

    /// Release-only measurement of the exact normal → absent → hovered cycle a
    /// full-screen Loop uses. It prints timings instead of asserting a machine-
    /// dependent duration.
    #[test]
    #[ignore = "timing benchmark, run by hand with --release --ignored --nocapture"]
    fn bench_two_appearance_reacquisition_cycle() {
        use std::time::Instant;

        let read_bench_image = |variable: &str, fallback: Gray| {
            std::env::var_os(variable)
                .and_then(|path| read_frame(Path::new(&path)).ok())
                .map(|frame| bgr_to_gray(&frame.bgr, frame.width as usize, frame.height as usize))
                .unwrap_or(fallback)
        };
        let normal = read_bench_image(
            "CLAWMATION_BENCH_TEMPLATE_A",
            resize(&button_image(1), 146, 32, Interp::Linear),
        );
        let hovered = read_bench_image(
            "CLAWMATION_BENCH_TEMPLATE_B",
            resize(&button_image(9), 146, 32, Interp::Linear),
        );
        let mut detector = Detector::new(2560, 1440);
        detector
            .templates
            .insert("normal".into(), Template::from_gray(&normal));
        detector
            .templates
            .insert("hovered".into(), Template::from_gray(&hovered));
        let candidates = [PathBuf::from("normal"), PathBuf::from("hovered")];
        let full_region = Some([0.0, 0.0, 100.0, 100.0]);
        let normal_frame = frame_with_image_at(2560, 1440, &normal, 1400, 900);
        let absent_frame = background_frame(2560, 1440);
        let hovered_frame = frame_with_image_at(2560, 1440, &hovered, 1400, 900);

        let started = Instant::now();
        let cold = detector
            .match_robust_candidates(
                &normal_frame,
                &candidates,
                full_region,
                0.8,
                "target",
                "bench:reacquire",
            )
            .unwrap();
        println!("cold present: {:?} ({} hit)", started.elapsed(), cold.len());

        for pass in 1..=8 {
            let started = Instant::now();
            let absent = detector
                .match_robust_candidates(
                    &absent_frame,
                    &candidates,
                    full_region,
                    0.8,
                    "target",
                    "bench:reacquire",
                )
                .unwrap();
            println!(
                "tracked absent #{pass}: {:?} ({} hit)",
                started.elapsed(),
                absent.len()
            );
        }

        let started = Instant::now();
        let hovered = detector
            .match_robust_candidates(
                &hovered_frame,
                &candidates,
                full_region,
                0.8,
                "target",
                "bench:reacquire",
            )
            .unwrap();
        println!(
            "hovered reappearance: {:?} ({} hit)",
            started.elapsed(),
            hovered.len()
        );
    }

    /// End-to-end desktop measurement: DXGI capture, real saved templates,
    /// detector, reliable click, disappearance, reacquisition, and second click.
    #[test]
    #[ignore = "live hardware benchmark; clicks the configured on-screen target twice"]
    fn bench_live_loop_reaction() {
        use std::sync::Arc;
        use std::time::{Duration, Instant};

        use crate::hardware::capture::ScreenCapture;
        use crate::hardware::input::InputController;
        use crate::hardware::reliable_input::ReliableInput;

        let normal = std::env::var("CLAWMATION_BENCH_TEMPLATE_A")
            .expect("CLAWMATION_BENCH_TEMPLATE_A must name the live normal image");
        let hovered = std::env::var("CLAWMATION_BENCH_TEMPLATE_B")
            .expect("CLAWMATION_BENCH_TEMPLATE_B must name the live hovered image");
        let step = Step {
            id: "live-loop-reaction".into(),
            step_type: "find_click".into(),
            detect_mode: "template".into(),
            template: normal,
            templates: vec![hovered],
            confidence: 0.8,
            region: [0.0, 0.0, 100.0, 100.0],
            ..Default::default()
        };
        let backend = std::env::var("CLAWMATION_BENCH_BACKEND").unwrap_or_else(|_| "dxcam".into());
        let mut capture = ScreenCapture::new(&backend, None);
        let mut detector = Detector::new(2560, 1440);
        let input = ReliableInput::new(Arc::new(InputController::new()));
        let deadline = Instant::now() + Duration::from_secs(30);

        let mut prior = None;
        while Instant::now() < deadline {
            let capture_started = Instant::now();
            let frame = capture.grab().expect("live frame");
            let capture_time = capture_started.elapsed();
            let detect_started = Instant::now();
            let (hits, _) = detector.ai_detect(&frame, &step);
            let detect_time = detect_started.elapsed();
            if let Some(hit) = hits.first() {
                let action_started = Instant::now();
                prior = Some(
                    input
                        .click_at(hit.x as i32, hit.y as i32)
                        .expect("first live click"),
                );
                println!(
                    "first click: backend={} at=({}, {}) confidence={:.3} capture={capture_time:?} detect={detect_time:?} action={:?}",
                    capture.backend(),
                    hit.x,
                    hit.y,
                    hit.confidence,
                    action_started.elapsed(),
                );
                break;
            }
        }
        assert!(prior.is_some(), "the live target never appeared");

        const CONFIRMED_ABSENCE: Duration = Duration::from_millis(300);
        let mut disappeared = false;
        let mut missing_since = None;
        let mut disappearance_started = None;
        let mut frames = 0_u64;
        let mut frame_changes = 0_u64;
        let mut previous_fingerprint = None;
        let mut max_capture = Duration::ZERO;
        let mut max_detect = Duration::ZERO;
        let mut max_poll = Duration::ZERO;
        let reaction_deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < reaction_deadline {
            let capture_started = Instant::now();
            let frame = capture.grab().expect("live frame");
            let capture_time = capture_started.elapsed();
            let fingerprint = frame
                .bgr
                .iter()
                .step_by(4_096)
                .fold(0xcbf2_9ce4_8422_2325_u64, |hash, value| {
                    (hash ^ u64::from(*value)).wrapping_mul(0x100_0000_01b3)
                });
            frames += 1;
            if previous_fingerprint.is_some_and(|previous| previous != fingerprint) {
                frame_changes += 1;
            }
            previous_fingerprint = Some(fingerprint);
            let detect_started = Instant::now();
            let (hits, _) = detector.ai_detect(&frame, &step);
            let detect_time = detect_started.elapsed();
            max_capture = max_capture.max(capture_time);
            max_detect = max_detect.max(detect_time);
            max_poll = max_poll.max(capture_time + detect_time);
            if hits.is_empty() {
                let started = *missing_since.get_or_insert_with(Instant::now);
                if !disappeared && started.elapsed() >= CONFIRMED_ABSENCE {
                    disappeared = true;
                    disappearance_started = Some(started);
                    println!(
                        "confirmed disappearance after {:?} (frame {frames}, capture={capture_time:?}, detect={detect_time:?})",
                        started.elapsed()
                    );
                }
                continue;
            }
            if !disappeared {
                missing_since = None;
                continue;
            }
            if disappeared {
                let hit = &hits[0];
                let action_started = Instant::now();
                input
                    .click_at_with_prior(hit.x as i32, hit.y as i32, prior.as_ref())
                    .expect("second live click");
                println!(
                    "second click: absent_for={:?} capture={capture_time:?} detect={detect_time:?} action={:?} frames={frames} max_capture={max_capture:?} max_detect={max_detect:?} max_poll={max_poll:?}",
                    disappearance_started
                        .expect("confirmed disappearance has a start")
                        .elapsed(),
                    action_started.elapsed(),
                );
                return;
            }
        }
        panic!(
            "the live target did not disappear and reappear inside 30 seconds ({frames} captures, {frame_changes} sampled frame changes)"
        );
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
        let hits = d
            .detect_guard(&frame_with_square(100, 100, 10, 10, 12), &g)
            .unwrap();
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
        let step = Step {
            detect_mode: "template".into(),
            ..Step::default()
        };
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
