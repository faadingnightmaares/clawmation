//! Template matching — `detection.py::PixelDetector`'s robust tiers, ported.
//!
//! A template is preprocessed once into a [`Template`]: CLAHE-equalised for
//! brightness-invariant correlation, Canny edges for background-invariant
//! correlation, and an auto-generated content mask that excludes whatever the
//! crop caught around the UI element. Per frame the search area is preprocessed
//! once and reused across the whole scale sweep.
//!
//! [`Matcher::robust`] runs three tiers and returns the first that hits:
//!
//! 0. **Temporal coherence** — correlate at native scale in a ±60 px window
//!    around wherever this template was last seen. Almost every repeat detection
//!    lands here, and it is the reason a guard polling at 50 ms is affordable.
//! 1. **Multi-scale CLAHE correlation**, coarse-to-fine with an early exit.
//! 2. **Multi-scale Canny-edge correlation**, at a relaxed threshold.
//!
//! Python had a fourth tier — ORB keypoints matched through FLANN-LSH into a
//! RANSAC homography. It is **deliberately not ported**. Reproducing it means
//! reproducing OpenCV's exact 256x4 rBRIEF sampling pattern (a hard-coded table
//! with no derivation) or accepting descriptors that differ from the ones the
//! thresholds were tuned against; and on the flat, low-texture game UI this app
//! targets it rarely found the six good matches it needed for a homography.
//! Tiers 0-2 are what actually fire. The cost of dropping it is rotated or
//! perspective-warped targets, which tiers 1 and 2 could not match either.

use std::collections::HashMap;

use super::canny::canny;
use super::clahe::Clahe;
use super::corr::Searched;
use super::image::{
    bgr_to_gray, dilate_ellipse5, flood_fill, morph_close_ellipse5, resize, Gray, Interp,
};
use super::Detection;

/// The sweep covers UI scaling from 30% to 200%.
const SCALE_MIN: f64 = 0.3;
const SCALE_MAX: f64 = 2.0;
const SCALE_STEPS: usize = 12;
/// The coarse pass runs on half-resolution pixels, so a quarter of the work.
const COARSE_FACTOR: f64 = 0.5;
/// A coarse hit this far above threshold ends the sweep.
const EARLY_EXIT_MARGIN: f64 = 0.05;
/// Slack around the last known position for the temporal-coherence tier.
const TEMPORAL_PAD: i64 = 60;

/// A template with everything the matcher needs precomputed. Built once per
/// load, never per frame.
pub struct Template {
    pub w: usize,
    pub h: usize,
    clahe: Gray,
    edges: Gray,
    /// 255 where the UI element is, 0 where the crop caught background. `None`
    /// when the crop is tight enough (or sparse enough) that masking is noise.
    mask: Option<Gray>,
}

impl Template {
    pub fn from_bgr(bgr: &[u8], w: usize, h: usize) -> Self {
        Self::from_gray(&bgr_to_gray(bgr, w, h))
    }

    pub fn from_gray(gray: &Gray) -> Self {
        let edges = canny(gray, 50, 150);
        let mask = auto_mask(&edges);
        Self { w: gray.w, h: gray.h, clahe: Clahe::detector_default().apply(gray), edges, mask }
    }
}

/// Derive a content mask by dilating the edges and flood-filling inwards from
/// the border: anything the fill reaches is background.
///
/// Python passed the grayscale in too, but only ever read its shape.
fn auto_mask(edges: &Gray) -> Option<Gray> {
    let (w, h) = (edges.w, edges.h);
    if w == 0 || h == 0 {
        return None;
    }
    let mut flood = dilate_ellipse5(edges, 2);
    let mut x = 0;
    let step_x = (w / 20).max(1);
    while x < w {
        if flood.at(x, 0) == 0 {
            flood_fill(&mut flood, (x, 0), 128);
        }
        if flood.at(x, h - 1) == 0 {
            flood_fill(&mut flood, (x, h - 1), 128);
        }
        x += step_x;
    }
    let mut y = 0;
    let step_y = (h / 20).max(1);
    while y < h {
        if flood.at(0, y) == 0 {
            flood_fill(&mut flood, (0, y), 128);
        }
        if flood.at(w - 1, y) == 0 {
            flood_fill(&mut flood, (w - 1, y), 128);
        }
        y += step_y;
    }

    let content = flood.data.iter().map(|&v| if v == 0 { 255 } else { 0 }).collect();
    // Close the holes inside the element — text interiors, mostly.
    let mask = morph_close_ellipse5(&Gray::from_vec(w, h, content), 2);

    let ratio = mask.count_nonzero() as f64 / (w * h) as f64;
    // Under 5% means the element has no closed boundary for the fill to stop
    // at (thin lines, sparse glyphs); over 95% means the crop is already tight
    // and the mask would only cost time.
    if ratio < 0.05 || ratio > 0.95 {
        return None;
    }
    Some(mask)
}

/// Holds the equaliser and the per-template memory of where each was last seen.
pub struct Matcher {
    clahe: Clahe,
    last: HashMap<String, (i64, i64, i64, i64)>,
}

impl Default for Matcher {
    fn default() -> Self {
        Self::new()
    }
}

impl Matcher {
    pub fn new() -> Self {
        Self { clahe: Clahe::detector_default(), last: HashMap::new() }
    }

    /// Forget where a template was last seen — used when its image is replaced,
    /// so a stale position cannot anchor tier 0 to the wrong place.
    pub fn forget(&mut self, name: &str) {
        self.last.remove(name);
    }

    /// The three-tier match. `search` is the grayscale search area, `ox`/`oy`
    /// its origin in full-frame pixels; returned coordinates are absolute.
    /// `key` addresses the per-template memory (the caller's cache key, which is
    /// the template's file path), `label` is what the detections are named.
    pub fn robust(
        &mut self,
        search: &Gray,
        ox: i64,
        oy: i64,
        tpl: &Template,
        key: &str,
        label: &str,
        threshold: f64,
    ) -> Vec<Detection> {
        if search.is_empty() {
            return Vec::new();
        }

        if let Some(hits) = self.temporal(search, ox, oy, tpl, key, label, threshold) {
            return hits;
        }

        let hits = self.sweep(search, ox, oy, tpl, label, threshold, false);
        if !hits.is_empty() {
            self.remember(key, &hits[0]);
            return hits;
        }

        let edge_threshold = (threshold - 0.1).max(0.5);
        let hits = self.sweep(search, ox, oy, tpl, label, edge_threshold, true);
        if !hits.is_empty() {
            self.remember(key, &hits[0]);
        }
        hits
    }

    fn remember(&mut self, name: &str, d: &Detection) {
        self.last.insert(name.to_string(), (d.x, d.y, d.w, d.h));
    }

    /// Tier 0. `None` when there is no remembered position, the window does not
    /// fit, or nothing correlated — all of which mean "fall through".
    fn temporal(
        &mut self,
        search: &Gray,
        ox: i64,
        oy: i64,
        tpl: &Template,
        key: &str,
        label: &str,
        threshold: f64,
    ) -> Option<Vec<Detection>> {
        let &(lx, ly, lw, lh) = self.last.get(key)?;
        let (sw, sh) = (search.w as i64, search.h as i64);
        let (wx, wy) = (lx - ox, ly - oy);
        let x1 = (wx - lw / 2 - TEMPORAL_PAD).max(0);
        let y1 = (wy - lh / 2 - TEMPORAL_PAD).max(0);
        let x2 = (wx + lw / 2 + TEMPORAL_PAD).min(sw);
        let y2 = (wy + lh / 2 + TEMPORAL_PAD).min(sh);
        if x2 - x1 < lw || y2 - y1 < lh {
            return None;
        }
        let window = search.crop(x1 as i32, y1 as i32, (x2 - x1) as i32, (y2 - y1) as i32)?;
        let hits = match_corr(
            &self.clahe.apply(&window),
            &tpl.clahe,
            tpl.mask.as_ref(),
            threshold,
            ox + x1,
            oy + y1,
            label,
        );
        if hits.is_empty() {
            return None;
        }
        self.remember(key, &hits[0]);
        Some(hits)
    }

    /// Tiers 1 and 2: sweep the scale range on a half-resolution copy, then
    /// re-correlate the winner at native resolution to get its true score.
    fn sweep(
        &self,
        search: &Gray,
        ox: i64,
        oy: i64,
        tpl: &Template,
        label: &str,
        threshold: f64,
        use_edges: bool,
    ) -> Vec<Detection> {
        let (sw, sh) = (search.w as i64, search.h as i64);
        let (proc, base, mask_base) = if use_edges {
            // Edges are already background-invariant; a mask on top only
            // narrows the evidence.
            (canny(search, 50, 150), &tpl.edges, None)
        } else {
            (self.clahe.apply(search), &tpl.clahe, tpl.mask.as_ref())
        };

        let coarse_w = ((sw as f64 * COARSE_FACTOR) as usize).max(1);
        let coarse_h = ((sh as f64 * COARSE_FACTOR) as usize).max(1);
        let coarse = Searched::new(&resize(&proc, coarse_w, coarse_h, Interp::Area));

        // The coarse pass loses correlation to the downscale, so it judges
        // against a relaxed bar and only nominates a candidate.
        let coarse_thresh = (threshold - 0.15).max(0.4) as f32;
        let early = (threshold + EARLY_EXIT_MARGIN) as f32;

        let mut best: Option<(Detection, f64)> = None;
        for scale in geomspace(SCALE_MIN, SCALE_MAX, SCALE_STEPS) {
            let full_tw = (tpl.w as f64 * scale) as i64;
            let full_th = (tpl.h as f64 * scale) as i64;
            if full_tw < 8 || full_th < 8 || full_th > sh || full_tw > sw {
                continue;
            }
            let ctw = ((full_tw as f64 * COARSE_FACTOR) as usize).max(2);
            let cth = ((full_th as f64 * COARSE_FACTOR) as usize).max(2);
            if cth > coarse_h || ctw > coarse_w {
                continue;
            }
            let interp = if scale < 1.0 { Interp::Area } else { Interp::Linear };
            let small = resize(base, ctw, cth, interp);
            let small_mask = mask_base.map(|m| resize(m, ctw, cth, Interp::Nearest));
            let Some(res) = coarse.ccoeff_normed(&small, small_mask.as_ref()) else {
                continue;
            };
            let Some((mx, my, max_val)) = res.best() else {
                continue;
            };
            if max_val < coarse_thresh {
                continue;
            }

            let fx = (mx as f64 / COARSE_FACTOR) as i64;
            let fy = (my as f64 / COARSE_FACTOR) as i64;
            let det = Detection {
                label: label.to_string(),
                x: ox + fx + full_tw / 2,
                y: oy + fy + full_th / 2,
                w: full_tw,
                h: full_th,
                confidence: f64::from(max_val),
                roi_offset: [ox, oy],
            };
            let better = match &best {
                Some((b, _)) => f64::from(max_val) > b.confidence,
                None => true,
            };
            if better {
                best = Some((det, scale));
            }
            if max_val >= early {
                break;
            }
        }

        let Some((best, best_scale)) = best else {
            return Vec::new();
        };

        // Refine: the coarse score is not the real one, so re-correlate the
        // winning scale at native resolution in a window around it.
        let full_tw = (tpl.w as f64 * best_scale) as i64;
        let full_th = (tpl.h as f64 * best_scale) as i64;
        let interp = if best_scale < 1.0 { Interp::Area } else { Interp::Linear };
        let native = resize(base, full_tw as usize, full_th as usize, interp);
        let native_mask =
            mask_base.map(|m| resize(m, full_tw as usize, full_th as usize, Interp::Nearest));

        let (cx, cy) = (best.x - ox, best.y - oy);
        let wx1 = (cx - full_tw).max(0);
        let wy1 = (cy - full_th).max(0);
        let wx2 = (cx + full_tw).min(sw);
        let wy2 = (cy + full_th).min(sh);
        let window =
            search_window(&proc, wx1, wy1, wx2, wy2).filter(|w| {
                w.h as i64 >= full_th && w.w as i64 >= full_tw
            });
        if let Some(window) = window {
            if let Some(res) = Searched::new(&window).ccoeff_normed(&native, native_mask.as_ref()) {
                if let Some((mx, my, max_val)) = res.best() {
                    if f64::from(max_val) >= threshold {
                        return vec![Detection {
                            label: label.to_string(),
                            x: ox + wx1 + mx as i64 + full_tw / 2,
                            y: oy + wy1 + my as i64 + full_th / 2,
                            w: full_tw,
                            h: full_th,
                            confidence: f64::from(max_val),
                            roi_offset: [ox, oy],
                        }];
                    }
                }
            }
        }

        // The refinement did not confirm. The coarse hit still counts if it
        // cleared the real threshold on its own — a big, high-contrast target
        // barely loses correlation to the downscale.
        if best.confidence >= threshold {
            vec![best]
        } else {
            Vec::new()
        }
    }
}

fn search_window(src: &Gray, x1: i64, y1: i64, x2: i64, y2: i64) -> Option<Gray> {
    if x2 <= x1 || y2 <= y1 {
        return None;
    }
    src.crop(x1 as i32, y1 as i32, (x2 - x1) as i32, (y2 - y1) as i32)
}

/// Correlate a preprocessed search area against a preprocessed template at 1:1,
/// returning every position at or above `threshold`.
fn match_corr(
    search: &Gray,
    tpl: &Gray,
    mask: Option<&Gray>,
    threshold: f64,
    ox: i64,
    oy: i64,
    label: &str,
) -> Vec<Detection> {
    let (tw, th) = (tpl.w as i64, tpl.h as i64);
    if th > search.h as i64 || tw > search.w as i64 || th < 2 || tw < 2 {
        return Vec::new();
    }
    let Some(scores) = Searched::new(search).ccoeff_normed(tpl, mask) else {
        return Vec::new();
    };
    scores
        .above(threshold as f32)
        .into_iter()
        .map(|(x, y, v)| Detection {
            label: label.to_string(),
            x: ox + x as i64 + tw / 2,
            y: oy + y as i64 + th / 2,
            w: tw,
            h: th,
            confidence: f64::from(v),
            roi_offset: [ox, oy],
        })
        .collect()
}

/// `np.geomspace(start, stop, num)` — evenly spaced on a log scale, with both
/// endpoints exact. The pinning matters: `int(template_w * scale)` truncates, so
/// a last step of 1.9999999 would size the template one pixel smaller than 2.0
/// does.
fn geomspace(start: f64, stop: f64, num: usize) -> Vec<f64> {
    if num == 0 {
        return Vec::new();
    }
    if num == 1 {
        return vec![start];
    }
    let (a, b) = (start.log10(), stop.log10());
    let step = (b - a) / (num - 1) as f64;
    let mut out: Vec<f64> = (0..num).map(|i| 10f64.powf(a + step * i as f64)).collect();
    out[0] = start;
    out[num - 1] = stop;
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A patch with a bright interior and a dark surround — enough structure for
    /// Canny to close a boundary, which is what `auto_mask` needs.
    fn badge(w: usize, h: usize) -> Gray {
        let mut g = Gray::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let inside = x >= 4 && y >= 4 && x + 4 < w && y + 4 < h;
                let v = if inside { 40 + ((x * 13 + y * 7) % 180) as u8 } else { 12 };
                g.set(x, y, v);
            }
        }
        g
    }

    /// Paste `patch` into a larger scene at `(px, py)`.
    fn scene(w: usize, h: usize, patch: &Gray, px: usize, py: usize) -> Gray {
        let mut g = Gray::new(w, h);
        for y in 0..h {
            for x in 0..w {
                g.set(x, y, ((x * 3 + y * 5) % 60) as u8);
            }
        }
        for y in 0..patch.h {
            for x in 0..patch.w {
                g.set(px + x, py + y, patch.at(x, y));
            }
        }
        g
    }

    #[test]
    fn geomspace_hits_both_endpoints_exactly() {
        let s = geomspace(0.3, 2.0, 12);
        assert_eq!(s.len(), 12);
        assert_eq!(s[0], 0.3);
        assert_eq!(s[11], 2.0);
        assert!(s.windows(2).all(|p| p[1] > p[0]), "not monotonic: {s:?}");
        // Log-spacing means a constant ratio between neighbours.
        let r = s[1] / s[0];
        assert!(s.windows(2).all(|p| ((p[1] / p[0]) - r).abs() < 1e-9));
    }

    #[test]
    fn a_template_is_found_where_it_was_pasted() {
        let patch = badge(28, 20);
        let tpl = Template::from_gray(&patch);
        let img = scene(160, 120, &patch, 55, 33);
        let hits = Matcher::new().robust(&img, 0, 0, &tpl, "t", "t", 0.75);
        assert_eq!(hits.len(), 1, "got {hits:?}");
        let d = &hits[0];
        assert!((d.x - (55 + 14)).abs() <= 2, "x was {}", d.x);
        assert!((d.y - (33 + 10)).abs() <= 2, "y was {}", d.y);
        assert!(d.confidence >= 0.75);
    }

    #[test]
    fn a_scaled_up_template_is_still_found() {
        let patch = badge(20, 16);
        let tpl = Template::from_gray(&patch);
        let big = resize(&patch, 30, 24, Interp::Linear);
        let img = scene(200, 150, &big, 70, 50);
        let hits = Matcher::new().robust(&img, 0, 0, &tpl, "t", "t", 0.7);
        assert!(!hits.is_empty(), "the 1.5x copy was not found");
        assert!((hits[0].x - 85).abs() <= 6, "x was {}", hits[0].x);
    }

    #[test]
    fn the_roi_origin_is_added_to_every_coordinate() {
        let patch = badge(24, 18);
        let tpl = Template::from_gray(&patch);
        let img = scene(140, 110, &patch, 40, 30);
        let hits = Matcher::new().robust(&img, 300, 200, &tpl, "t", "t", 0.75);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].roi_offset, [300, 200]);
        assert!(hits[0].x > 300 && hits[0].y > 200);
    }

    #[test]
    fn nothing_is_reported_when_the_template_is_absent() {
        let tpl = Template::from_gray(&badge(24, 18));
        let mut noise = Gray::new(120, 90);
        for y in 0..90 {
            for x in 0..120 {
                noise.set(x, y, ((x * 31 + y * 17) % 255) as u8);
            }
        }
        assert!(Matcher::new().robust(&noise, 0, 0, &tpl, "t", "t", 0.9).is_empty());
    }

    #[test]
    fn the_second_look_reuses_the_remembered_position() {
        let patch = badge(26, 20);
        let tpl = Template::from_gray(&patch);
        let img = scene(180, 140, &patch, 60, 44);
        let mut m = Matcher::new();
        let first = m.robust(&img, 0, 0, &tpl, "t", "t", 0.75);
        assert_eq!(first.len(), 1);
        assert_eq!(m.last["t"], (60 + 13, 44 + 10, 26, 20));

        // Tier 0 reports *every* position that clears the threshold, so a
        // self-similar patch answers more than once — where the sweep only ever
        // nominates a single winner. That difference is how this test knows the
        // remembered position was used instead of a fresh sweep.
        let second = m.robust(&img, 0, 0, &tpl, "t", "t", 0.75);
        assert!(second.len() > 1, "the sweep ran again: {second:?}");
        let peak = second.iter().max_by(|a, b| a.confidence.total_cmp(&b.confidence)).unwrap();
        assert_eq!((peak.w, peak.h), (26, 20), "tier 0 correlates at native scale");
        assert_eq!((peak.x, peak.y), (60 + 13, 44 + 10));

        m.forget("t");
        assert!(!m.last.contains_key("t"));
    }

    #[test]
    fn a_template_that_cannot_be_shrunk_to_fit_finds_nothing() {
        // The sweep bottoms out at 30%, so an 80x60 template still asks for
        // 24x18 of search area. Offer it less and every scale is skipped —
        // "bigger than the frame" on its own is not disqualifying.
        let tpl = Template::from_gray(&badge(80, 60));
        let mut small = Gray::new(20, 16);
        for y in 0..16 {
            for x in 0..20 {
                small.set(x, y, ((x * 7 + y * 11) % 200) as u8);
            }
        }
        assert!(Matcher::new().robust(&small, 0, 0, &tpl, "t", "t", 0.7).is_empty());
    }

    #[test]
    fn a_bordered_element_masks_its_surround_and_a_shapeless_crop_does_not() {
        // A thin stroke closes no boundary, so the fill runs straight past it
        // and leaves nothing to call content — the under-5% branch.
        let mut stroke = Gray::from_vec(24, 24, vec![20; 576]);
        for i in 4..20 {
            stroke.set(i, i, 220);
        }
        assert!(Template::from_gray(&stroke).mask.is_none());

        // A block inset in a flat surround does close one, so the fill stops at
        // its edge: the mask keeps the block and clears the surround.
        let mut inset = Gray::from_vec(40, 40, vec![5; 1600]);
        for y in 8..32 {
            for x in 8..32 {
                inset.set(x, y, 190);
            }
        }
        let mask = Template::from_gray(&inset).mask.expect("a bordered crop should mask");
        assert_eq!(mask.at(20, 20), 255, "the element itself must be kept");
        assert_eq!(mask.at(1, 1), 0, "the surround must be cleared");
    }
}
