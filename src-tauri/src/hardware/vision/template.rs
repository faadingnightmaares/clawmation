//! Template matching: `detection.py::PixelDetector`'s robust tiers, ported.
//!
//! A template is preprocessed once into a [`Template`]: CLAHE-equalised for
//! brightness-invariant correlation, Canny edges for background-invariant
//! correlation, and an auto-generated content mask that excludes whatever the
//! crop caught around the UI element. Per frame the search area is preprocessed
//! once and reused across the whole scale sweep.
//!
//! [`Matcher::robust`] runs three tiers and returns the first that hits:
//!
//! 0. **Temporal coherence**: correlate at native scale in a ±60 px window
//!    around wherever this template was last seen. Almost every repeat detection
//!    lands here, and it is the reason a guard polling at 50 ms is affordable.
//! 1. **Multi-scale CLAHE correlation**, coarse-to-fine with an early exit.
//! 2. **Multi-scale Canny-edge correlation**, at a relaxed threshold.
//!
//! Tiers 1 and 2 share one shape, and the shape is the whole story of how often
//! they miss. A half-resolution pass *nominates* scales, and native-resolution
//! correlation *judges* them. Three things follow from keeping those two jobs
//! apart. The nomination bar sits well under the real threshold, because a small
//! busy element can score 0.5 at half resolution and 0.95 at native. More than
//! one nomination survives, because at half resolution the ranking between
//! neighbouring scales is noise and the best coarse score is regularly not the
//! right scale. And once a scale is confirmed, one last native pass over the
//! whole search area reports every copy of the target, not just the one the
//! sweep happened to point at.
//!
//! Python had a fourth tier: ORB keypoints matched through FLANN-LSH into a
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

/// The sweep covers UI scaling from 30% to 200%, stepping out from native by
/// [`SCALE_RATIO`] each rung. 12% rungs put any real scale within 6% of one the
/// sweep actually tries, which is inside what a normalised correlation shrugs
/// off; the 18% rungs this used to have were not.
const SCALE_MIN: f64 = 0.3;
const SCALE_MAX: f64 = 2.0;
const SCALE_RATIO: f64 = 1.12;
/// The coarse pass runs on half-resolution pixels, so a quarter of the work.
const COARSE_FACTOR: f64 = 0.5;
/// A coarse template smaller than this on a side has no shape left to correlate
/// against, and a nomination made from noise is worse than none: it takes a
/// shortlist slot from a real one.
const COARSE_MIN_SIDE: usize = 6;
/// How far under the real threshold a coarse score may sit and still be worth
/// confirming at native resolution, and the floor that slack stops at.
const COARSE_SLACK: f64 = 0.25;
const COARSE_FLOOR: f64 = 0.35;
/// How many coarse nominations get confirmed at native resolution.
const SHORTLIST: usize = 3;
/// A coarse hit this far above threshold ends the sweep.
const EARLY_EXIT_MARGIN: f64 = 0.05;
/// Slack around the last known position for the temporal-coherence tier.
const TEMPORAL_PAD: i64 = 60;
/// Past this much correlation work, in the pixel-products [`work`] counts, the
/// confirming pass over the whole search area is skipped and the windowed
/// refine stands on its own. It only costs the second and third copy of a
/// target, and only on searches wide enough that one full-frame correlation
/// would be felt.
const FULL_NATIVE_BUDGET: u64 = 2_000_000_000;

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
    // Close the holes inside the element: text interiors, mostly.
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

    /// Forget where a template was last seen. Used when its image is replaced,
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
    /// fit, or nothing correlated, all of which mean "fall through".
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

    /// Tiers 1 and 2: nominate scales on a half-resolution copy, confirm the
    /// best few at native resolution, then look once more at the confirmed
    /// scale so every copy of the target is reported.
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

        // The coarse pass nominates; it does not judge. It loses correlation to
        // the downscale, so holding it to the real threshold here throws away
        // exactly the marginal targets this tier exists to catch.
        let coarse_thresh = (threshold - COARSE_SLACK).max(COARSE_FLOOR) as f32;
        let early = (threshold + EARLY_EXIT_MARGIN) as f32;

        let mut shortlist: Vec<Detection> = Vec::new();
        for scale in scale_ladder() {
            let full_tw = (tpl.w as f64 * scale) as i64;
            let full_th = (tpl.h as f64 * scale) as i64;
            if full_tw < 8 || full_th < 8 || full_th > sh || full_tw > sw {
                continue;
            }
            let ctw = (full_tw as f64 * COARSE_FACTOR) as usize;
            let cth = (full_th as f64 * COARSE_FACTOR) as usize;
            if ctw < COARSE_MIN_SIDE || cth < COARSE_MIN_SIDE || ctw > coarse_w || cth > coarse_h {
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
            nominate(
                &mut shortlist,
                Detection {
                    label: label.to_string(),
                    x: ox + fx + full_tw / 2,
                    y: oy + fy + full_th / 2,
                    w: full_tw,
                    h: full_th,
                    confidence: f64::from(max_val),
                    roi_offset: [ox, oy],
                },
            );
            if max_val >= early {
                break;
            }
        }

        let mut confirmed: Option<Detection> = None;
        for cand in &shortlist {
            let Some(hit) = refine(&proc, base, mask_base, cand, ox, oy, label, threshold) else {
                continue;
            };
            let better = match &confirmed {
                Some(c) => hit.confidence > c.confidence,
                None => true,
            };
            if better {
                confirmed = Some(hit);
            }
        }

        let Some(hit) = confirmed else {
            // Nothing survived at native resolution. The strongest nomination
            // still stands if it cleared the real threshold on its own: a big,
            // high-contrast target barely loses correlation to the downscale.
            return match shortlist.into_iter().next() {
                Some(best) if best.confidence >= threshold => vec![best],
                _ => Vec::new(),
            };
        };

        // The scale is settled, so look once more at native resolution over the
        // whole search area. This is the only pass that can report a second copy
        // of the target: everything above nominates one position per scale.
        if work(sw, sh, hit.w, hit.h) <= FULL_NATIVE_BUDGET {
            let (native, mask) = native_pair(base, mask_base, hit.w as usize, hit.h as usize);
            let all = match_corr(&proc, &native, mask.as_ref(), threshold, ox, oy, label);
            if !all.is_empty() {
                return all;
            }
        }
        vec![hit]
    }
}

/// Insert into the shortlist, strongest first, capped at [`SHORTLIST`]. Ties go
/// to whatever is already there, and because the ladder runs outwards from
/// native scale that means the rung nearest 1.0 keeps the slot.
fn nominate(shortlist: &mut Vec<Detection>, det: Detection) {
    let at =
        shortlist.iter().position(|d| det.confidence > d.confidence).unwrap_or(shortlist.len());
    if at >= SHORTLIST {
        return;
    }
    shortlist.insert(at, det);
    shortlist.truncate(SHORTLIST);
}

/// Confirm one nomination at native resolution, in a window around where the
/// coarse pass pointed. `None` when the window does not fit or the true score
/// falls short of `threshold`.
#[allow(clippy::too_many_arguments)]
fn refine(
    proc: &Gray,
    base: &Gray,
    mask_base: Option<&Gray>,
    cand: &Detection,
    ox: i64,
    oy: i64,
    label: &str,
    threshold: f64,
) -> Option<Detection> {
    let (sw, sh) = (proc.w as i64, proc.h as i64);
    let (tw, th) = (cand.w, cand.h);
    let (cx, cy) = (cand.x - ox, cand.y - oy);
    let x1 = (cx - tw / 2 - refine_slack(tw)).max(0);
    let y1 = (cy - th / 2 - refine_slack(th)).max(0);
    let x2 = (cx + tw / 2 + refine_slack(tw)).min(sw);
    let y2 = (cy + th / 2 + refine_slack(th)).min(sh);
    if x2 - x1 < tw || y2 - y1 < th {
        return None;
    }
    let window = proc.crop(x1 as i32, y1 as i32, (x2 - x1) as i32, (y2 - y1) as i32)?;
    let (native, mask) = native_pair(base, mask_base, tw as usize, th as usize);
    let res = Searched::new(&window).ccoeff_normed(&native, mask.as_ref())?;
    let (mx, my, max_val) = res.best()?;
    if f64::from(max_val) < threshold {
        return None;
    }
    Some(Detection {
        label: label.to_string(),
        x: ox + x1 + mx as i64 + tw / 2,
        y: oy + y1 + my as i64 + th / 2,
        w: tw,
        h: th,
        confidence: f64::from(max_val),
        roi_offset: [ox, oy],
    })
}

/// How far the native refine looks from where the coarse pass pointed. The
/// downscale costs a pixel or two of position, and a nomination one rung off in
/// scale sits proportionally off-centre, so the slack grows with the template.
fn refine_slack(side: i64) -> i64 {
    (((side as f64 * (SCALE_RATIO - 1.0)) as i64) + 8).max(12)
}

/// The template and its mask, resized to the size a confirmed detection claims.
fn native_pair(base: &Gray, mask: Option<&Gray>, tw: usize, th: usize) -> (Gray, Option<Gray>) {
    let interp = if tw < base.w { Interp::Area } else { Interp::Linear };
    (resize(base, tw, th, interp), mask.map(|m| resize(m, tw, th, Interp::Nearest)))
}

/// The pixel-product a correlation is linear in: output positions times
/// template area.
fn work(sw: i64, sh: i64, tw: i64, th: i64) -> u64 {
    let positions = (sw - tw + 1).max(0) as u64 * (sh - th + 1).max(0) as u64;
    positions.saturating_mul((tw.max(0) as u64).saturating_mul(th.max(0) as u64))
}

/// Keep the strongest detection out of each cluster of overlapping ones.
///
/// A correlation peak is never one pixel wide: every position within a few of
/// the true one clears the threshold too, so a raw threshold pass hands back a
/// blob per target. Downstream counts those ("3 robust match(es)"), draws them
/// on the detections overlay and clicks them, so the blob has to collapse to the
/// one detection it really is, while two genuinely separate copies stay two.
fn suppress_overlaps(mut dets: Vec<Detection>) -> Vec<Detection> {
    dets.sort_by(|a, b| b.confidence.total_cmp(&a.confidence));
    let mut kept: Vec<Detection> = Vec::new();
    for d in dets {
        // Centres less than half a template apart cannot be two of anything.
        let same = kept.iter().any(|k| {
            (k.x - d.x).abs() * 2 < d.w.max(1) && (k.y - d.y).abs() * 2 < d.h.max(1)
        });
        if !same {
            kept.push(d);
        }
    }
    kept
}

/// Correlate a preprocessed search area against a preprocessed template at 1:1,
/// returning one detection per target at or above `threshold`.
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
    suppress_overlaps(
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
            .collect(),
    )
}

/// The scales the sweep tries, ordered outwards from native.
///
/// Native comes first and is exact, which matters more than anything else here:
/// the template was almost always cropped from the same screen it is now being
/// looked for on, so 1.0 is the answer most of the time, and a ladder that
/// merely brackets native asks every one of those matches to survive a resample
/// it never needed. Running outwards also puts the early exit where it pays.
/// Both ends are pinned because `(w as f64 * scale) as i64` truncates: a last
/// rung of 1.9999 would size the template a pixel short of 2.0.
fn scale_ladder() -> Vec<f64> {
    let mut out = vec![1.0];
    let (mut down, mut up) = (1.0 / SCALE_RATIO, SCALE_RATIO);
    while down > SCALE_MIN || up < SCALE_MAX {
        if down > SCALE_MIN {
            out.push(down);
            down /= SCALE_RATIO;
        }
        if up < SCALE_MAX {
            out.push(up);
            up *= SCALE_RATIO;
        }
    }
    out.push(SCALE_MIN);
    out.push(SCALE_MAX);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A patch with a bright interior and a dark surround, enough structure for
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
        paste(&mut g, patch, px, py);
        g
    }

    /// The same scene with the patch in two places.
    fn two_copies(
        w: usize,
        h: usize,
        patch: &Gray,
        a: (usize, usize),
        b: (usize, usize),
    ) -> Gray {
        let mut g = scene(w, h, patch, a.0, a.1);
        paste(&mut g, patch, b.0, b.1);
        g
    }

    fn paste(g: &mut Gray, patch: &Gray, px: usize, py: usize) {
        for y in 0..patch.h {
            for x in 0..patch.w {
                g.set(px + x, py + y, patch.at(x, y));
            }
        }
    }

    #[test]
    fn the_ladder_starts_at_native_and_leaves_no_scale_far_from_a_rung() {
        let ladder = scale_ladder();
        assert_eq!(ladder[0], 1.0, "native scale is tried first");
        assert!(ladder.contains(&SCALE_MIN) && ladder.contains(&SCALE_MAX), "{ladder:?}");
        assert!(ladder.iter().all(|s| (SCALE_MIN..=SCALE_MAX).contains(s)), "{ladder:?}");

        // Sorted, no two neighbours further apart than one rung: that bound is
        // what caps how far off a real scale can be from the nearest one tried.
        let mut sorted = ladder.clone();
        sorted.sort_by(f64::total_cmp);
        for pair in sorted.windows(2) {
            assert!(pair[1] / pair[0] <= SCALE_RATIO + 1e-9, "gap at {pair:?}");
        }

        // Ordered outwards from native, so the early exit sees the likeliest
        // scales first.
        let distance: Vec<f64> = ladder.iter().map(|s| (s.ln()).abs()).collect();
        let interior = &distance[..distance.len() - 2];
        assert!(interior.windows(2).all(|p| p[1] >= p[0] - 1e-9), "{ladder:?}");
    }

    #[test]
    fn overlapping_peaks_collapse_but_two_targets_stay_two() {
        let at = |x: i64, y: i64, c: f64| Detection {
            label: "t".into(),
            x,
            y,
            w: 30,
            h: 20,
            confidence: c,
            roi_offset: [0, 0],
        };
        // Three positions off the same peak, plus a genuinely separate copy.
        let kept = suppress_overlaps(vec![
            at(100, 100, 0.81),
            at(102, 101, 0.94),
            at(101, 99, 0.88),
            at(300, 100, 0.83),
        ]);
        assert_eq!(kept.len(), 2, "got {kept:?}");
        assert_eq!((kept[0].x, kept[0].y), (102, 101), "the strongest of the cluster wins");
        assert_eq!(kept[1].x, 300);
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
    fn every_copy_of_the_target_is_reported() {
        // One nomination per scale is all the sweep can make, so finding the
        // second copy is entirely down to the confirming pass over the whole
        // search area. Clicking a row of identical items depends on it.
        let patch = badge(26, 20);
        let tpl = Template::from_gray(&patch);
        let img = two_copies(220, 140, &patch, (20, 44), (150, 44));
        let hits = Matcher::new().robust(&img, 0, 0, &tpl, "t", "t", 0.75);
        assert_eq!(hits.len(), 2, "got {hits:?}");
        let mut xs: Vec<i64> = hits.iter().map(|d| d.x).collect();
        xs.sort_unstable();
        assert!((xs[0] - (20 + 13)).abs() <= 2, "first copy at {}", xs[0]);
        assert!((xs[1] - (150 + 13)).abs() <= 2, "second copy at {}", xs[1]);
    }

    #[test]
    fn the_second_look_reuses_the_remembered_position() {
        let patch = badge(26, 20);
        let tpl = Template::from_gray(&patch);
        let img = two_copies(220, 140, &patch, (20, 44), (150, 44));
        let mut m = Matcher::new();
        assert_eq!(m.robust(&img, 0, 0, &tpl, "t", "t", 0.75).len(), 2);
        let (lx, _, _, _) = m.last["t"];

        // Tier 0 only ever looks ±TEMPORAL_PAD around where it last saw the
        // template, and the two copies are 130 px apart, so answering with one
        // of them, and only one, is how this test knows the sweep did not rerun.
        let second = m.robust(&img, 0, 0, &tpl, "t", "t", 0.75);
        assert_eq!(second.len(), 1, "the sweep ran again: {second:?}");
        assert_eq!((second[0].w, second[0].h), (26, 20), "tier 0 correlates at native scale");
        assert!((second[0].x - lx).abs() <= 2, "moved from {lx} to {}", second[0].x);

        m.forget("t");
        assert!(!m.last.contains_key("t"));
    }

    #[test]
    fn a_scale_between_two_rungs_is_still_found() {
        // 1.06x sits halfway between the 1.0 and 1.12 rungs: the worst case the
        // ladder can hand the matcher, and the case the old 18% rungs lost.
        let patch = badge(32, 24);
        let tpl = Template::from_gray(&patch);
        let odd = resize(&patch, 34, 25, Interp::Linear);
        let img = scene(240, 180, &odd, 80, 60);
        let hits = Matcher::new().robust(&img, 0, 0, &tpl, "t", "t", 0.75);
        assert!(!hits.is_empty(), "the 1.06x copy was not found");
        assert!((hits[0].x - (80 + 17)).abs() <= 6, "x was {}", hits[0].x);
    }

    #[test]
    fn a_template_that_cannot_be_shrunk_to_fit_finds_nothing() {
        // The sweep bottoms out at 30%, so an 80x60 template still asks for
        // 24x18 of search area. Offer it less and every scale is skipped;
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
        // and leaves nothing to call content: the under-5% branch.
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
