//! Template matching: `detection.py::PixelDetector`'s robust tiers, ported.
//!
//! A template is preprocessed once into a [`Template`]: its original grayscale
//! pixels for normalized correlation, Canny edges for background-invariant
//! correlation, and an auto-generated content mask that excludes whatever the
//! crop caught around the UI element. Per frame the search area is preprocessed
//! once and reused across the whole scale sweep.
//!
//! [`Matcher::robust`] runs three tiers and returns the first that hits:
//!
//! 0. **Temporal coherence**: correlate at native scale in a ±60 px window
//!    around wherever this template was last seen. Almost every repeat detection
//!    lands here, and it is the reason a guard polling at 50 ms is affordable.
//! 1. **Multi-scale raw correlation**, coarse-to-fine with an early exit.
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
/// The coarse pass runs on downscaled pixels. Half resolution is a quarter of
/// the work, and it is where this was tuned, so it stays the ceiling.
const COARSE_MAX_FACTOR: f64 = 0.5;
/// Coarse-pass area the sweep aims for, in pixels.
///
/// Half resolution is right for the region-scoped searches guards use, and
/// wrong by an order of magnitude for a watch trigger set to "anywhere": a
/// 2560x1440 screen is thirty times a drawn region, and correlation is linear
/// in area, so the same ladder that costs a guard a few milliseconds costs the
/// watcher seconds. Since the coarse pass only nominates and native resolution
/// judges, the cheaper pass buys back a poll interval that actually catches
/// things at the price of a slightly noisier shortlist.
const COARSE_TARGET_PX: f64 = 60_000.0;
/// A coarse template smaller than this on a side has no shape left to correlate
/// against, and a nomination made from noise is worse than none: it takes a
/// shortlist slot from a real one.
const COARSE_MIN_SIDE: usize = 6;
/// Native scale gets one higher-detail nomination before the broad ladder.
/// Flat buttons and glyphs lose their identity at six pixels tall even though
/// that is enough for textured targets; twelve preserves their structure.
const NATIVE_MIN_SIDE: usize = 12;
const NATIVE_PEAKS: usize = 12;
/// How far under the real threshold a coarse score may sit and still be worth
/// confirming at native resolution, and the floor that slack stops at.
const COARSE_SLACK: f64 = 0.25;
const COARSE_FLOOR: f64 = 0.35;
/// How many distinct coarse nominations get confirmed at native resolution.
/// The old three-slot list was regularly consumed by neighbouring scales of one
/// look-alike, leaving no room for the real target elsewhere on a full screen.
const SHORTLIST: usize = 12;
/// Plausible positions retained from each scale. Native confirmation is cheap
/// because it runs in a small window, and judging several locations is far more
/// reliable than trusting one downscaled maximum.
const PEAKS_PER_SCALE: usize = 4;
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
    raw: Gray,
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
        Self {
            w: gray.w,
            h: gray.h,
            raw: gray.clone(),
            clahe: Clahe::detector_default().apply(gray),
            edges,
            mask,
        }
    }
}

/// One downscaled view shared by every appearance of the same logical target.
///
/// Rebuilding the screen statistics for normal, hovered, and pressed pictures
/// made an absent full-screen target the slowest possible case. A learned-scale
/// probe instead prepares the screen once, nominates at the last confirmed
/// scale, and still lets native pixels enforce the user's real threshold.
pub struct LearnedScaleSearch<'a> {
    search: &'a Gray,
    ox: i64,
    oy: i64,
    factor: f64,
    coarse: Searched,
}

impl<'a> LearnedScaleSearch<'a> {
    pub fn new(search: &'a Gray, ox: i64, oy: i64, target_min_side: usize) -> Self {
        let factor = coarse_factor(
            search.w as i64,
            search.h as i64,
            target_min_side.max(COARSE_MIN_SIDE),
        );
        let coarse_w = ((search.w as f64 * factor) as usize).max(1);
        let coarse_h = ((search.h as f64 * factor) as usize).max(1);
        let coarse = Searched::new(&resize(search, coarse_w, coarse_h, Interp::Area));
        Self {
            search,
            ox,
            oy,
            factor,
            coarse,
        }
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

    let content = flood
        .data
        .iter()
        .map(|&v| if v == 0 { 255 } else { 0 })
        .collect();
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
        Self {
            clahe: Clahe::detector_default(),
            last: HashMap::new(),
        }
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

    /// Correlate one small hot zone at a previously confirmed scale. This is
    /// deliberately exact: a fast path may nominate cheaply, but it may never
    /// weaken the threshold that decides whether an action is allowed.
    #[allow(clippy::too_many_arguments)]
    pub fn focused(
        &mut self,
        search: &Gray,
        ox: i64,
        oy: i64,
        tpl: &Template,
        key: &str,
        label: &str,
        threshold: f64,
        target_w: i64,
        target_h: i64,
    ) -> Vec<Detection> {
        if target_w < 2 || target_h < 2 {
            return Vec::new();
        }
        let (native, mask) = native_pair(
            &tpl.raw,
            tpl.mask.as_ref(),
            target_w as usize,
            target_h as usize,
        );
        let hits = match_corr(search, &native, mask.as_ref(), threshold, ox, oy, label);
        if let Some(hit) = hits.first() {
            self.remember(key, hit);
        }
        hits
    }

    /// Probe the complete search region at one already-confirmed scale, sharing
    /// the expensive downscaled search statistics across all appearances.
    #[allow(clippy::too_many_arguments)]
    pub fn learned(
        &mut self,
        search: &LearnedScaleSearch<'_>,
        tpl: &Template,
        key: &str,
        label: &str,
        threshold: f64,
        target_w: i64,
        target_h: i64,
    ) -> Vec<Detection> {
        let Some(hit) = confirm_at_size(
            search.search,
            &tpl.raw,
            tpl.mask.as_ref(),
            search.ox,
            search.oy,
            label,
            threshold,
            target_w,
            target_h,
            search.factor,
            Some(&search.coarse),
            false,
        ) else {
            return Vec::new();
        };
        self.remember(key, &hit);
        vec![hit]
    }

    pub fn remember_detection(&mut self, key: &str, detection: &Detection) {
        self.remember(key, detection);
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
            &window,
            &tpl.raw,
            tpl.mask.as_ref(),
            threshold,
            ox + x1,
            oy + y1,
            label,
        );
        if !hits.is_empty() {
            self.remember(key, &hits[0]);
            return Some(hits);
        }

        // A remembered target can undergo a local nonlinear lighting change.
        // Equalising this small window is cheap and avoids the full-screen CLAHE
        // mismatch that made cold scans unreliable.
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
            None
        } else {
            self.remember(key, &hits[0]);
            Some(hits)
        }
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
        let edge_frame;
        let (proc, base, mask_base) = if use_edges {
            // Edges are already background-invariant; a mask on top only
            // narrows the evidence.
            edge_frame = canny(search, 50, 150);
            (&edge_frame, &tpl.edges, None)
        } else {
            // CCOEFF_NORMED already removes linear brightness and contrast
            // changes. Keeping the original pixels is crucial for flat UI:
            // CLAHE maps a tiny crop and a full-screen frame with different tile
            // histograms, so even byte-identical pixels can stop correlating.
            (search, &tpl.raw, tpl.mask.as_ref())
        };

        let factor = coarse_factor(sw, sh, tpl.w.min(tpl.h));
        let mut prepared_coarse = None;

        // Most Watch pictures were captured from this same display and therefore
        // reappear at (or very near) native scale. Give that overwhelmingly
        // common case one higher-detail nomination before the cheaper broad
        // ladder. This is both faster and more reliable than asking a 6px-tall
        // version of a flat button to compete with every scale and look-alike.
        if !use_edges {
            let native_floor =
                (NATIVE_MIN_SIDE as f64 / tpl.w.min(tpl.h).max(1) as f64).min(COARSE_MAX_FACTOR);
            let native_factor = factor.max(native_floor);
            if native_factor == factor {
                let coarse_w = ((sw as f64 * factor) as usize).max(1);
                let coarse_h = ((sh as f64 * factor) as usize).max(1);
                prepared_coarse = Some(Searched::new(&resize(
                    proc,
                    coarse_w,
                    coarse_h,
                    Interp::Area,
                )));
            }
            if let Some(hit) = confirm_native(
                proc,
                base,
                mask_base,
                ox,
                oy,
                label,
                threshold,
                native_factor,
                prepared_coarse.as_ref(),
            ) {
                if work(sw, sh, hit.w, hit.h) <= FULL_NATIVE_BUDGET {
                    let all = match_corr(proc, base, mask_base, threshold, ox, oy, label);
                    if !all.is_empty() {
                        return all;
                    }
                }
                return vec![hit];
            }
        }

        let coarse_w = ((sw as f64 * factor) as usize).max(1);
        let coarse_h = ((sh as f64 * factor) as usize).max(1);
        let coarse = prepared_coarse
            .unwrap_or_else(|| Searched::new(&resize(proc, coarse_w, coarse_h, Interp::Area)));

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
            let ctw = (full_tw as f64 * factor) as usize;
            let cth = (full_th as f64 * factor) as usize;
            if ctw < COARSE_MIN_SIDE || cth < COARSE_MIN_SIDE || ctw > coarse_w || cth > coarse_h {
                continue;
            }
            let interp = if scale < 1.0 {
                Interp::Area
            } else {
                Interp::Linear
            };
            let small = resize(base, ctw, cth, interp);
            let small_mask = mask_base.map(|m| resize(m, ctw, cth, Interp::Nearest));
            let Some(res) = coarse.ccoeff_normed(&small, small_mask.as_ref()) else {
                continue;
            };
            let peaks = res.peaks(coarse_thresh, PEAKS_PER_SCALE);
            let Some((_, _, max_val)) = peaks.first().copied() else {
                continue;
            };
            let mut scale_candidates = Vec::with_capacity(peaks.len());
            for (mx, my, score) in peaks {
                let fx = (mx as f64 / factor) as i64;
                let fy = (my as f64 / factor) as i64;
                let candidate = Detection {
                    label: label.to_string(),
                    x: ox + fx + full_tw / 2,
                    y: oy + fy + full_th / 2,
                    w: full_tw,
                    h: full_th,
                    confidence: f64::from(score),
                    roi_offset: [ox, oy],
                };
                nominate(&mut shortlist, candidate.clone());
                scale_candidates.push(candidate);
            }
            if max_val >= early {
                // A coarse score alone must never stop the scale ladder: a
                // look-alike at the wrong scale can score highly after the
                // downsample and hide the real target. Only native-resolution
                // confirmation earns the early exit.
                if let Some(hit) = scale_candidates
                    .iter()
                    .filter_map(|cand| {
                        refine(
                            proc, base, mask_base, cand, ox, oy, label, threshold, factor,
                        )
                    })
                    .max_by(|a, b| a.confidence.total_cmp(&b.confidence))
                {
                    if work(sw, sh, hit.w, hit.h) <= FULL_NATIVE_BUDGET {
                        let (native, mask) =
                            native_pair(base, mask_base, hit.w as usize, hit.h as usize);
                        let all =
                            match_corr(proc, &native, mask.as_ref(), threshold, ox, oy, label);
                        if !all.is_empty() {
                            return all;
                        }
                    }
                    return vec![hit];
                }
            }
        }

        let mut confirmed: Option<Detection> = None;
        for cand in &shortlist {
            let Some(hit) = refine(
                &proc, base, mask_base, cand, ox, oy, label, threshold, factor,
            ) else {
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
    // Neighbouring scale rungs around the same on-screen object are one
    // nomination, not several. Keep the strongest representative so the list
    // has room for plausible locations elsewhere on the screen.
    if let Some(existing) = shortlist.iter_mut().find(|d| {
        let same_place = (d.x - det.x).abs() * 2 < d.w.max(det.w).max(1)
            && (d.y - det.y).abs() * 2 < d.h.max(det.h).max(1);
        let same_scale = (d.w - det.w).abs() * 20 <= d.w.max(det.w).max(1)
            && (d.h - det.h).abs() * 20 <= d.h.max(det.h).max(1);
        same_place && same_scale
    }) {
        if det.confidence > existing.confidence {
            *existing = det;
            shortlist.sort_by(|a, b| b.confidence.total_cmp(&a.confidence));
        }
        return;
    }
    let at = shortlist
        .iter()
        .position(|d| det.confidence > d.confidence)
        .unwrap_or(shortlist.len());
    if at >= SHORTLIST {
        return;
    }
    shortlist.insert(at, det);
    shortlist.truncate(SHORTLIST);
}

/// Nominate and immediately verify native scale before the broad scale ladder.
/// Returning only after native-resolution correlation keeps the coarse score
/// from weakening the user's configured threshold.
#[allow(clippy::too_many_arguments)]
fn confirm_native(
    proc: &Gray,
    base: &Gray,
    mask_base: Option<&Gray>,
    ox: i64,
    oy: i64,
    label: &str,
    threshold: f64,
    factor: f64,
    prepared_coarse: Option<&Searched>,
) -> Option<Detection> {
    confirm_at_size(
        proc,
        base,
        mask_base,
        ox,
        oy,
        label,
        threshold,
        base.w as i64,
        base.h as i64,
        factor,
        prepared_coarse,
        true,
    )
}

/// Nominate one known target size on the shared coarse screen, then confirm it
/// against native pixels. This is the learned-scale counterpart of the broad
/// scale ladder and therefore cannot return a coarse-only result.
#[allow(clippy::too_many_arguments)]
fn confirm_at_size(
    proc: &Gray,
    base: &Gray,
    mask_base: Option<&Gray>,
    ox: i64,
    oy: i64,
    label: &str,
    threshold: f64,
    target_w: i64,
    target_h: i64,
    factor: f64,
    prepared_coarse: Option<&Searched>,
    use_coarse_mask: bool,
) -> Option<Detection> {
    let (sw, sh) = (proc.w as i64, proc.h as i64);
    let coarse_w = ((sw as f64 * factor) as usize).max(1);
    let coarse_h = ((sh as f64 * factor) as usize).max(1);
    let (tw, th) = (target_w, target_h);
    if tw < 2 || th < 2 || tw > sw || th > sh {
        return None;
    }
    let ctw = (tw as f64 * factor) as usize;
    let cth = (th as f64 * factor) as usize;
    if ctw < COARSE_MIN_SIDE || cth < COARSE_MIN_SIDE || ctw > coarse_w || cth > coarse_h {
        return None;
    }

    let owned_coarse;
    let coarse = match prepared_coarse {
        Some(coarse) => coarse,
        None => {
            owned_coarse = Searched::new(&resize(proc, coarse_w, coarse_h, Interp::Area));
            &owned_coarse
        }
    };
    let (native, native_mask) = native_pair(base, mask_base, tw as usize, th as usize);
    let small = resize(&native, ctw, cth, Interp::Area);
    // The learned whole-screen probe deliberately nominates without a mask:
    // masked correlation cannot use the accelerated convolution path and made
    // a missing target more expensive than the old complete scale sweep. Native
    // confirmation below still applies the real mask and configured threshold.
    let small_mask = use_coarse_mask
        .then(|| {
            native_mask
                .as_ref()
                .map(|m| resize(m, ctw, cth, Interp::Nearest))
        })
        .flatten();
    let scores = coarse.ccoeff_normed(&small, small_mask.as_ref())?;
    // Near-native verification is deliberately more permissive at the coarse
    // stage. A one-pixel crop drift can damage a tiny downsample badly, while
    // native correlation still has enough evidence to judge it correctly.
    let coarse_thresh = (threshold - 0.4).max(0.25) as f32;

    scores
        .peaks(coarse_thresh, NATIVE_PEAKS)
        .into_iter()
        .filter_map(|(mx, my, score)| {
            let fx = (mx as f64 / factor) as i64;
            let fy = (my as f64 / factor) as i64;
            let cand = Detection {
                label: label.to_string(),
                x: ox + fx + tw / 2,
                y: oy + fy + th / 2,
                w: tw,
                h: th,
                confidence: f64::from(score),
                roi_offset: [ox, oy],
            };
            refine_near_native(
                proc, base, mask_base, &cand, ox, oy, label, threshold, factor,
            )
        })
        .max_by(|a, b| a.confidence.total_cmp(&b.confidence))
}

/// Verify a native nomination with a few pixel-level crop variants. Screen
/// pickers and DPI rounding move opposite edges independently, so this is more
/// faithful than pretending every near-native difference is uniform scaling.
#[allow(clippy::too_many_arguments)]
fn refine_near_native(
    proc: &Gray,
    base: &Gray,
    mask_base: Option<&Gray>,
    cand: &Detection,
    ox: i64,
    oy: i64,
    label: &str,
    threshold: f64,
    factor: f64,
) -> Option<Detection> {
    const WIDTH_DELTAS: [i64; 7] = [0, 1, -1, 2, -2, 3, -3];
    const HEIGHT_DELTAS: [i64; 5] = [0, 1, -1, 2, -2];

    for &dh in &HEIGHT_DELTAS {
        for &dw in &WIDTH_DELTAS {
            let mut variant = cand.clone();
            variant.w = (cand.w + dw).max(2);
            variant.h = (cand.h + dh).max(2);
            if let Some(hit) = refine(
                proc, base, mask_base, &variant, ox, oy, label, threshold, factor,
            ) {
                return Some(hit);
            }
        }
    }
    None
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
    factor: f64,
) -> Option<Detection> {
    let (sw, sh) = (proc.w as i64, proc.h as i64);
    let (tw, th) = (cand.w, cand.h);
    let (cx, cy) = (cand.x - ox, cand.y - oy);
    let (sx, sy) = (refine_slack(tw, factor), refine_slack(th, factor));
    let x1 = (cx - tw / 2 - sx).max(0);
    let y1 = (cy - th / 2 - sy).max(0);
    let x2 = (cx + tw / 2 + sx).min(sw);
    let y2 = (cy + th / 2 + sy).min(sh);
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

/// How far the native refine looks from where the coarse pass pointed. A
/// nomination one rung off in scale sits proportionally off-centre, so the slack
/// grows with the template, and one coarse pixel covers `1 / factor` native
/// ones, so it also grows as the coarse pass gets cheaper.
fn refine_slack(side: i64, factor: f64) -> i64 {
    let quantisation = (1.0 / factor).ceil() as i64;
    (((side as f64 * (SCALE_RATIO - 1.0)) as i64) + 8 + quantisation).max(12)
}

/// How far down the coarse pass shrinks a search of this size. Bounded below by
/// what leaves the template a shape to correlate against, since a nomination
/// made from a three-pixel-tall smear is worse than no nomination at all.
fn coarse_factor(sw: i64, sh: i64, tpl_min_side: usize) -> f64 {
    let area = (sw.max(1) as f64) * (sh.max(1) as f64);
    let floor = (COARSE_MIN_SIDE as f64 / tpl_min_side.max(1) as f64).min(COARSE_MAX_FACTOR);
    (COARSE_TARGET_PX / area)
        .sqrt()
        .clamp(floor, COARSE_MAX_FACTOR)
}

/// The template and its mask, resized to the size a confirmed detection claims.
fn native_pair(base: &Gray, mask: Option<&Gray>, tw: usize, th: usize) -> (Gray, Option<Gray>) {
    let interp = if tw < base.w {
        Interp::Area
    } else {
        Interp::Linear
    };
    (
        resize(base, tw, th, interp),
        mask.map(|m| resize(m, tw, th, Interp::Nearest)),
    )
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
        let same = kept
            .iter()
            .any(|k| (k.x - d.x).abs() * 2 < d.w.max(1) && (k.y - d.y).abs() * 2 < d.h.max(1));
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
                let v = if inside {
                    40 + ((x * 13 + y * 7) % 180) as u8
                } else {
                    12
                };
                g.set(x, y, v);
            }
        }
        g
    }

    /// A flat, mostly-white UI button with dark glyph-like strokes. Unlike
    /// `badge`, its histogram is intentionally nothing like the whole scene's:
    /// this catches preprocessing that changes the same pixels differently
    /// depending on whether they are equalised as a crop or as a full screen.
    fn flat_button(w: usize, h: usize) -> Gray {
        let mut g = Gray::from_vec(w, h, vec![245; w * h]);
        let y0 = h / 3;
        let y1 = h * 2 / 3;
        for x in w / 4..w * 3 / 4 {
            if x % 11 < 6 {
                for y in y0..y1 {
                    if y == y0 || y == y1 - 1 || x % 11 == 0 {
                        g.set(x, y, 35);
                    }
                }
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
    fn two_copies(w: usize, h: usize, patch: &Gray, a: (usize, usize), b: (usize, usize)) -> Gray {
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
        assert!(
            ladder.contains(&SCALE_MIN) && ladder.contains(&SCALE_MAX),
            "{ladder:?}"
        );
        assert!(
            ladder.iter().all(|s| (SCALE_MIN..=SCALE_MAX).contains(s)),
            "{ladder:?}"
        );

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
        assert!(
            interior.windows(2).all(|p| p[1] >= p[0] - 1e-9),
            "{ladder:?}"
        );
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
        assert_eq!(
            (kept[0].x, kept[0].y),
            (102, 101),
            "the strongest of the cluster wins"
        );
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
    fn learned_scale_finds_a_target_that_moved_across_the_search_area() {
        let patch = badge(28, 20);
        let tpl = Template::from_gray(&patch);
        let img = scene(240, 160, &patch, 175, 110);
        let learned = LearnedScaleSearch::new(&img, 0, 0, patch.h);
        let hits = Matcher::new().learned(
            &learned,
            &tpl,
            "target",
            "target",
            0.85,
            patch.w as i64,
            patch.h as i64,
        );

        assert_eq!(hits.len(), 1, "got {hits:?}");
        assert!((hits[0].x - 189).abs() <= 3);
        assert!((hits[0].y - 120).abs() <= 3);
    }

    #[test]
    fn focused_and_learned_searches_never_weaken_the_threshold() {
        let wanted = badge(28, 20);
        let other = flat_button(28, 20);
        let tpl = Template::from_gray(&wanted);
        let img = scene(180, 120, &other, 70, 45);
        let mut matcher = Matcher::new();

        assert!(matcher
            .focused(
                &img,
                0,
                0,
                &tpl,
                "target",
                "target",
                0.99,
                wanted.w as i64,
                wanted.h as i64,
            )
            .is_empty());

        let learned = LearnedScaleSearch::new(&img, 0, 0, wanted.h);
        assert!(matcher
            .learned(
                &learned,
                &tpl,
                "target",
                "target",
                0.99,
                wanted.w as i64,
                wanted.h as i64,
            )
            .is_empty());
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
        assert!(Matcher::new()
            .robust(&noise, 0, 0, &tpl, "t", "t", 0.9)
            .is_empty());
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
        assert_eq!(
            (second[0].w, second[0].h),
            (26, 20),
            "tier 0 correlates at native scale"
        );
        assert!(
            (second[0].x - lx).abs() <= 2,
            "moved from {lx} to {}",
            second[0].x
        );

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
        assert!(Matcher::new()
            .robust(&small, 0, 0, &tpl, "t", "t", 0.7)
            .is_empty());
    }

    #[test]
    fn the_coarse_pass_gets_cheaper_as_the_search_gets_wider() {
        // A drawn guard region keeps the half resolution this was tuned at.
        assert!((coarse_factor(480, 320, 32) - 0.5).abs() < 1e-9);
        // A whole 2560x1440 screen does not: at half resolution it would cost
        // thirty times as much for one nomination.
        assert!(coarse_factor(2560, 1440, 32) < 0.25);
        // However wide the search, the coarse template keeps a shape. Six
        // pixels on the short side of a 12-pixel template is half resolution,
        // and no search area may argue it lower.
        assert!((coarse_factor(2560, 1440, 12) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn a_target_is_found_on_a_search_too_wide_for_the_half_resolution_pass() {
        // The tuned tests all run on scenes small enough to stay at half
        // resolution, so without this one nothing covers what a watch trigger
        // set to the whole screen actually executes.
        let patch = badge(120, 40);
        let tpl = Template::from_gray(&patch);
        assert!(coarse_factor(1200, 800, tpl.w.min(tpl.h)) < COARSE_MAX_FACTOR);

        let hits = Matcher::new().robust(
            &scene(1200, 800, &patch, 700, 500),
            0,
            0,
            &tpl,
            "t",
            "t",
            0.8,
        );
        assert_eq!(hits.len(), 1);
        assert_eq!(
            (hits[0].w, hits[0].h),
            (120, 40),
            "the native rung is the one that won"
        );
        assert!(
            hits[0].confidence >= 0.8,
            "an exact cold match must satisfy the configured threshold, got {:?}",
            hits[0]
        );
        // Within a few pixels rather than on the pixel: this patch is a diagonal
        // ramp, and equalising a 1200x800 scene against a 120x40 crop moves its
        // correlation peak a little whatever the coarse pass does. The same
        // scene at half resolution is off by the same amount, so the tolerance
        // is the fixture's, not the wider search's.
        assert!(
            (hits[0].x - 760).abs() <= 8 && (hits[0].y - 520).abs() <= 8,
            "{:?}",
            hits[0]
        );
    }

    #[test]
    fn a_flat_ui_target_keeps_its_confidence_on_a_cold_full_screen_scan() {
        let patch = flat_button(175, 36);
        let tpl = Template::from_gray(&patch);
        let hits = Matcher::new().robust(
            &scene(1200, 800, &patch, 700, 500),
            0,
            0,
            &tpl,
            "button",
            "button",
            0.8,
        );
        assert_eq!(hits.len(), 1, "the exact button was missed: {hits:?}");
        assert!(
            hits[0].confidence >= 0.8,
            "the exact button fell below its configured threshold: {:?}",
            hits[0]
        );
    }

    #[test]
    fn a_flat_ui_target_survives_small_capture_and_scale_drift() {
        let patch = flat_button(175, 36);
        let tpl = Template::from_gray(&patch);
        // A repeated crop commonly differs by a pixel or two because the game
        // window or DPI rounding moved. It is still the same target.
        let drifted = resize(&patch, 178, 37, Interp::Linear);
        let hits = Matcher::new().robust(
            &scene(1200, 800, &drifted, 700, 500),
            0,
            0,
            &tpl,
            "button",
            "button",
            0.8,
        );
        assert!(!hits.is_empty(), "the slightly resized button was missed");
        assert!(
            (hits[0].x - (700 + 89)).abs() <= 6 && (hits[0].y - (500 + 18)).abs() <= 6,
            "the match moved away from the button: {:?}",
            hits[0]
        );
    }

    /// What a full-screen Watch trigger actually costs, present and absent.
    /// Prints; asserts nothing about duration, because a timing assertion on a
    /// shared runner is a flaky test rather than a measurement.
    #[test]
    #[ignore = "timing benchmark, run by hand: prints, asserts nothing about duration"]
    fn bench_full_screen_sweep() {
        use std::time::Instant;

        let patch = badge(146, 32);
        let tpl = Template::from_gray(&patch);
        let present = scene(2560, 1440, &patch, 1400, 900);
        let mut absent = Gray::new(2560, 1440);
        for y in 0..1440 {
            for x in 0..2560 {
                absent.set(x, y, ((x * 3 + y * 5) % 60) as u8);
            }
        }

        let t = Instant::now();
        let hits = Matcher::new().robust(&present, 0, 0, &tpl, "t", "t", 0.8);
        println!("present, cold: {:?} ({} hit(s))", t.elapsed(), hits.len());

        let mut warm = Matcher::new();
        warm.robust(&present, 0, 0, &tpl, "t", "t", 0.8);
        let t = Instant::now();
        warm.robust(&present, 0, 0, &tpl, "t", "t", 0.8);
        println!("present, remembered: {:?}", t.elapsed());

        let t = Instant::now();
        let hits = Matcher::new().robust(&absent, 0, 0, &tpl, "t", "t", 0.8);
        println!("absent: {:?} ({} hit(s))", t.elapsed(), hits.len());

        let t = Instant::now();
        let hits = Matcher::new().robust(&present, 0, 0, &tpl, "t", "t", 0.8);
        println!(
            "present, GPU warm / matcher cold: {:?} ({} hit(s))",
            t.elapsed(),
            hits.len()
        );

        let t = Instant::now();
        let hits = Matcher::new().robust(&absent, 0, 0, &tpl, "t", "t", 0.8);
        println!(
            "absent, GPU warm: {:?} ({} hit(s))",
            t.elapsed(),
            hits.len()
        );
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
        let mask = Template::from_gray(&inset)
            .mask
            .expect("a bordered crop should mask");
        assert_eq!(mask.at(20, 20), 255, "the element itself must be kept");
        assert_eq!(mask.at(1, 1), 0, "the surround must be cleared");
    }
}
