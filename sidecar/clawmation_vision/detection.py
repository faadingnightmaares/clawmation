"""Pixel detection engine — HSV thresholding, template matching, OCR.

All detection is ROI-scoped to avoid false positives from the 3D game world.

Performance design (blazing-fast path):
  * Every template is preprocessed ONCE at load time into a cache: grayscale,
    CLAHE-equalized, Canny edges, ORB keypoints/descriptors, and a geometric
    pyramid of resized copies. None of this is recomputed per frame.
  * Per frame, the search area is converted to grayscale + CLAHE + Canny ONCE
    and reused across every scale in the pyramid (the old code redid this for
    each of 15 scales — ~15x the work).
  * Feature matching uses ORB + FLANN-LSH (cached template descriptors) instead
    of AKAZE + brute-force, which is an order of magnitude faster.
  * Coarse-to-fine: the search area is downscaled for the scale sweep so each
    matchTemplate runs on a fraction of the pixels, then the best candidate is
    refined at native resolution.
  * Early exit: a confident hit (>= threshold + margin) stops the sweep.
"""

from __future__ import annotations

import logging
from dataclasses import dataclass
from pathlib import Path

import cv2
import numpy as np

from .config import HSVRange, Region

logger = logging.getLogger(__name__)


@dataclass
class Detection:
    """A single detection result."""
    label: str
    x: int  # centre x in full-frame pixels
    y: int  # centre y in full-frame pixels
    w: int
    h: int
    confidence: float
    roi_offset: tuple[int, int] = (0, 0)  # ROI origin for coordinate mapping


# Precomputed, per-template data. Built once at load time.
@dataclass
class _TemplateCache:
    color: np.ndarray
    gray: np.ndarray
    clahe: np.ndarray
    edges: np.ndarray
    orb_kp: list
    orb_des: np.ndarray | None
    mask: np.ndarray | None  # content mask: 255=UI element, 0=background
    w: int
    h: int


class PixelDetector:
    """Multi-strategy pixel detection engine."""

    # Scale sweep: 0.3x..2.0x covers UI scaling from 30% to 200%.
    DEFAULT_SCALE_RANGE = (0.3, 2.0)
    DEFAULT_SCALE_STEPS = 12
    # Downscale factor for the coarse sweep (search runs on 1/2-res pixels).
    COARSE_FACTOR = 0.5
    # Downscale factor for ORB feature matching (fewer keypoints = faster).
    ORB_FACTOR = 0.25
    # A hit this far above threshold stops the sweep early.
    EARLY_EXIT_MARGIN = 0.05

    def __init__(self, screen_w: int = 2560, screen_h: int = 1440):
        self.screen_w = screen_w
        self.screen_h = screen_h
        self._templates: dict[str, np.ndarray] = {}
        self._cache: dict[str, _TemplateCache] = {}
        self._orb = cv2.ORB_create(nfeatures=500)
        self._flann = None  # lazy FLANN-LSH matcher
        self._clahe = cv2.createCLAHE(clipLimit=2.0, tileGridSize=(8, 8))
        self._ocr_reader = None  # lazy init
        # Temporal coherence: last match position per template. When a template
        # is found, we search a small window around the last position first
        # (5-10x faster than full-frame). Falls back to full search if not found.
        self._last_match: dict[str, tuple[int, int, int, int]] = {}  # name -> (x, y, w, h)

    # ── ROI extraction ───────────────────────────────────────────────────────

    def extract_roi(self, frame: np.ndarray, region: Region) -> np.ndarray:
        """Crop a Region (percentage-based) from a full frame."""
        x1, y1, x2, y2 = region.to_pixels(self.screen_w, self.screen_h)
        return frame[y1:y2, x1:x2].copy()

    def roi_origin(self, region: Region) -> tuple[int, int]:
        x1, y1, _, _ = region.to_pixels(self.screen_w, self.screen_h)
        return (x1, y1)

    # ── HSV color detection ──────────────────────────────────────────────────

    def detect_color(
        self,
        frame: np.ndarray,
        hsv_range: HSVRange,
        region: Region | None = None,
        min_area: int = 50,
        label: str = "color",
    ) -> list[Detection]:
        """Find blobs matching an HSV color range within an optional ROI."""
        if region:
            roi = self.extract_roi(frame, region)
            ox, oy = self.roi_origin(region)
        else:
            roi = frame
            ox, oy = 0, 0

        hsv = cv2.cvtColor(roi, cv2.COLOR_BGR2HSV)
        lower = np.array(hsv_range.lower, dtype=np.uint8)
        upper = np.array(hsv_range.upper, dtype=np.uint8)
        mask = cv2.inRange(hsv, lower, upper)

        # Morphological cleanup
        kernel = cv2.getStructuringElement(cv2.MORPH_RECT, (3, 3))
        mask = cv2.morphologyEx(mask, cv2.MORPH_OPEN, kernel, iterations=1)
        mask = cv2.morphologyEx(mask, cv2.MORPH_CLOSE, kernel, iterations=1)

        contours, _ = cv2.findContours(mask, cv2.RETR_EXTERNAL, cv2.CHAIN_APPROX_SIMPLE)
        detections = []
        for cnt in contours:
            area = cv2.contourArea(cnt)
            if area < min_area:
                continue
            x, y, w, h = cv2.boundingRect(cnt)
            detections.append(Detection(
                label=label,
                x=ox + x + w // 2,
                y=oy + y + h // 2,
                w=w, h=h,
                confidence=min(1.0, area / (w * h + 1)),  # fill ratio
                roi_offset=(ox, oy),
            ))

        # Sort by area descending (largest = most confident)
        detections.sort(key=lambda d: d.w * d.h, reverse=True)
        return detections

    def color_present(
        self,
        frame: np.ndarray,
        hsv_range: HSVRange,
        region: Region,
        min_pixel_ratio: float = 0.05,
    ) -> bool:
        """Quick boolean: is this color present in the ROI above a threshold?"""
        roi = self.extract_roi(frame, region)
        hsv = cv2.cvtColor(roi, cv2.COLOR_BGR2HSV)
        lower = np.array(hsv_range.lower, dtype=np.uint8)
        upper = np.array(hsv_range.upper, dtype=np.uint8)
        mask = cv2.inRange(hsv, lower, upper)
        ratio = np.count_nonzero(mask) / mask.size
        return ratio >= min_pixel_ratio

    # ── Template loading + cache ─────────────────────────────────────────────

    def load_template(self, name: str, path: str | Path) -> None:
        """Load a template image and precompute its detection cache once."""
        img = cv2.imread(str(path), cv2.IMREAD_COLOR)
        if img is None:
            raise FileNotFoundError(f"Template not found: {path}")
        self._templates[name] = img
        self._cache[name] = self._build_cache(img)
        logger.info("Loaded template '%s' (%dx%d)", name, img.shape[1], img.shape[0])

    def _build_cache(self, img: np.ndarray) -> _TemplateCache:
        gray = cv2.cvtColor(img, cv2.COLOR_BGR2GRAY)
        clahe = self._clahe.apply(gray)
        edges = cv2.Canny(gray, 50, 150)
        kp, des = self._orb.detectAndCompute(clahe, None)
        mask = self._auto_mask(gray, edges)
        return _TemplateCache(
            color=img, gray=gray, clahe=clahe, edges=edges,
            orb_kp=kp, orb_des=des, mask=mask,
            w=img.shape[1], h=img.shape[0],
        )

    def _auto_mask(self, gray: np.ndarray, edges: np.ndarray) -> np.ndarray | None:
        """Auto-generate a content mask for background-invariant matching.

        Detects the UI element's footprint by dilating edges and flood-filling
        from the borders. Pixels reachable from the crop border are treated as
        background (0); everything else is the UI element (255).

        Returns None if the mask is >95% content (tight crop, no background to
        suppress), signalling callers to skip the mask overhead.
        """
        h, w = gray.shape
        # Dilate edges to close gaps in the UI element's boundary
        kernel = cv2.getStructuringElement(cv2.MORPH_ELLIPSE, (5, 5))
        dilated = cv2.dilate(edges, kernel, iterations=2)

        # Flood-fill from all four borders. Everything reachable = background.
        pad = np.zeros((h + 2, w + 2), np.uint8)
        flood = dilated.copy()
        # Seed from each border
        for x in range(0, w, max(1, w // 20)):
            if flood[0, x] == 0:
                cv2.floodFill(flood, pad, (x, 0), 128)
            if flood[h - 1, x] == 0:
                cv2.floodFill(flood, pad, (x, h - 1), 128)
        for y in range(0, h, max(1, h // 20)):
            if flood[y, 0] == 0:
                cv2.floodFill(flood, pad, (0, y), 128)
            if flood[y, w - 1] == 0:
                cv2.floodFill(flood, pad, (w - 1, y), 128)

        # Content = NOT background (pixels NOT reached by flood fill)
        mask = np.where(flood == 0, 255, 0).astype(np.uint8)

        # Close holes inside the UI element (text interiors, etc.)
        mask = cv2.morphologyEx(mask, cv2.MORPH_CLOSE, kernel, iterations=2)

        content_ratio = np.count_nonzero(mask) / mask.size

        # If the mask is essentially all-background or all-content, skip it.
        # All-background (<5%) means the UI element has no closed boundary
        # (thin lines, sparse features). All-content (>95%) means a tight crop
        # with negligible background — mask overhead isn't worth it.
        if content_ratio < 0.05 or content_ratio > 0.95:
            return None

        return mask

    def _get_cache(self, template_name: str) -> _TemplateCache:
        if template_name not in self._cache:
            raise KeyError(f"Template '{template_name}' not loaded")
        return self._cache[template_name]

    def ensure_template_path(self, path: str | Path) -> str:
        """Load a template from a file path if not already loaded. Returns the
        cache key (derived from the path) so callers can pass it to match_robust.
        Used by vision checkpoints that store a template by file path."""
        key = f"_tpl_{Path(path).name}"
        if key not in self._cache:
            self.load_template(key, path)
        return key

    # ── Template matching (single-scale, cached preprocessing) ───────────────

    def match_template(
        self,
        frame: np.ndarray,
        template_name: str,
        region: Region | None = None,
        threshold: float = 0.8,
        multiscale: bool = False,
        scale_range: tuple[float, float] = DEFAULT_SCALE_RANGE,
        scale_steps: int = DEFAULT_SCALE_STEPS,
    ) -> list[Detection]:
        """Find occurrences of a template in the frame (optionally ROI-scoped).

        When multiscale=True, runs the fast coarse-to-fine scale sweep.
        """
        cache = self._get_cache(template_name)
        if region:
            search = self.extract_roi(frame, region)
            ox, oy = self.roi_origin(region)
        else:
            search = frame
            ox, oy = 0, 0

        if not multiscale:
            s_gray = self._to_gray(search)
            s_eq = self._clahe.apply(s_gray)
            return self._match_corr(s_eq, cache.clahe, threshold, ox, oy, template_name,
                                    mask=cache.mask)

        return self._scale_sweep(search, cache, threshold, ox, oy, template_name,
                                 scale_range, scale_steps, use_edges=False)

    @staticmethod
    def _to_gray(img: np.ndarray) -> np.ndarray:
        return cv2.cvtColor(img, cv2.COLOR_BGR2GRAY) if img.ndim == 3 else img

    def _match_corr(
        self, search_eq: np.ndarray, tpl_eq: np.ndarray, threshold: float,
        ox: int, oy: int, label: str,
        mask: np.ndarray | None = None,
    ) -> list[Detection]:
        """Correlate a pre-equalized search area against a pre-equalized template
        at native scale (1:1). When `mask` is provided, background pixels in the
        template are excluded from the correlation (background-invariant matching)."""
        th, tw = tpl_eq.shape[:2]
        if th > search_eq.shape[0] or tw > search_eq.shape[1] or th < 2 or tw < 2:
            return []
        if mask is not None and mask.shape[:2] == tpl_eq.shape[:2]:
            result = cv2.matchTemplate(search_eq, tpl_eq, cv2.TM_CCOEFF_NORMED, mask=mask)
        else:
            result = cv2.matchTemplate(search_eq, tpl_eq, cv2.TM_CCOEFF_NORMED)
        locs = np.where(result >= threshold)
        out = []
        for pt in zip(*locs[::-1]):
            out.append(Detection(
                label=label,
                x=ox + pt[0] + tw // 2,
                y=oy + pt[1] + th // 2,
                w=tw, h=th,
                confidence=float(result[pt[1], pt[0]]),
                roi_offset=(ox, oy),
            ))
        return out

    def _scale_sweep(
        self, search: np.ndarray, cache: _TemplateCache, threshold: float,
        ox: int, oy: int, label: str,
        scale_range: tuple[float, float], scale_steps: int, use_edges: bool,
    ) -> list[Detection]:
        """Coarse-to-fine multiscale sweep.

        1. Preprocess the search area ONCE (gray+CLAHE or Canny edges).
        2. Downscale it for a fast coarse sweep with a relaxed threshold.
        3. Refine the best coarse candidate at NATIVE resolution to get the
           true confidence score.
        4. Early-exit if a coarse hit is already very confident.

        When `cache.mask` is set, it is resized alongside the template and
        passed to matchTemplate to exclude background pixels from the
        correlation score. This makes detection robust to changing backgrounds
        (sand, grass, water, etc.) as long as the UI element itself is stable.
        The mask is NOT used for edge-based matching (use_edges=True) — edges
        are already background-invariant.
        """
        sh, sw = search.shape[:2]
        cf = self.COARSE_FACTOR

        if use_edges:
            search_proc = cv2.Canny(self._to_gray(search), 50, 150)
            tpl_base = cache.edges
            mask_base = None  # edges are already background-invariant
        else:
            search_proc = self._clahe.apply(self._to_gray(search))
            tpl_base = cache.clahe
            mask_base = cache.mask  # may be None (tight crop)

        # Coarse (downscaled) search area
        coarse_w, coarse_h = max(1, int(sw * cf)), max(1, int(sh * cf))
        coarse = cv2.resize(search_proc, (coarse_w, coarse_h), interpolation=cv2.INTER_AREA)

        scales = np.geomspace(scale_range[0], scale_range[1], scale_steps)
        # Relaxed threshold for the coarse pass (downscaling lowers correlation)
        coarse_thresh = max(threshold - 0.15, 0.4)
        early = threshold + self.EARLY_EXIT_MARGIN

        best: Detection | None = None
        best_scale = 1.0
        best_mask_native: np.ndarray | None = None

        for scale in scales:
            full_tw, full_th = int(cache.w * scale), int(cache.h * scale)
            if full_tw < 8 or full_th < 8 or full_th > sh or full_tw > sw:
                continue
            ctw, cth = max(2, int(full_tw * cf)), max(2, int(full_th * cf))
            if cth > coarse_h or ctw > coarse_w:
                continue
            interp = cv2.INTER_AREA if scale < 1 else cv2.INTER_LINEAR
            tpl = cv2.resize(tpl_base, (ctw, cth), interpolation=interp)

            # Resize mask alongside template (INTER_NEAREST for binary mask)
            mask_coarse = None
            if mask_base is not None:
                mask_coarse = cv2.resize(mask_base, (ctw, cth), interpolation=cv2.INTER_NEAREST)

            if mask_coarse is not None and mask_coarse.shape[:2] == tpl.shape[:2]:
                res = cv2.matchTemplate(coarse, tpl, cv2.TM_CCOEFF_NORMED, mask=mask_coarse)
            else:
                res = cv2.matchTemplate(coarse, tpl, cv2.TM_CCOEFF_NORMED)
            _, max_val, _, max_loc = cv2.minMaxLoc(res)
            if max_val < coarse_thresh:
                continue

            fx = int(max_loc[0] / cf)
            fy = int(max_loc[1] / cf)
            det = Detection(
                label=label, x=ox + fx + full_tw // 2, y=oy + fy + full_th // 2,
                w=full_tw, h=full_th, confidence=float(max_val), roi_offset=(ox, oy),
            )
            if best is None or max_val > best.confidence:
                best = det
                best_scale = scale
                # Cache the native-resolution mask for refinement
                if mask_base is not None:
                    best_mask_native = cv2.resize(
                        mask_base, (full_tw, full_th), interpolation=cv2.INTER_NEAREST)
            if max_val >= early:
                break

        if best is None:
            return []

        # Refine at native resolution: correlate the template at the best scale
        # against the full-res search area, centred on the coarse hit.
        full_tw, full_th = int(cache.w * best_scale), int(cache.h * best_scale)
        interp = cv2.INTER_AREA if best_scale < 1 else cv2.INTER_LINEAR
        tpl_native = cv2.resize(tpl_base, (full_tw, full_th), interpolation=interp)

        # Crop a window around the coarse hit for a fast native-resolution match
        cx, cy = best.x - ox, best.y - oy  # centre in search-area coords
        pad_x, pad_y = full_tw, full_th
        wx1 = max(0, cx - pad_x)
        wy1 = max(0, cy - pad_y)
        wx2 = min(sw, cx + pad_x)
        wy2 = min(sh, cy + pad_y)
        window = search_proc[wy1:wy2, wx1:wx2]

        if window.shape[0] >= full_th and window.shape[1] >= full_tw:
            if best_mask_native is not None and best_mask_native.shape[:2] == tpl_native.shape[:2]:
                res2 = cv2.matchTemplate(window, tpl_native, cv2.TM_CCOEFF_NORMED, mask=best_mask_native)
            else:
                res2 = cv2.matchTemplate(window, tpl_native, cv2.TM_CCOEFF_NORMED)
            _, max_val2, _, max_loc2 = cv2.minMaxLoc(res2)
            if max_val2 >= threshold:
                return [Detection(
                    label=label,
                    x=ox + wx1 + max_loc2[0] + full_tw // 2,
                    y=oy + wy1 + max_loc2[1] + full_th // 2,
                    w=full_tw, h=full_th,
                    confidence=float(max_val2),
                    roi_offset=(ox, oy),
                )]

        # Native refinement didn't confirm — return the coarse hit only if it
        # clears the real threshold (can happen when downscaling barely hurts).
        if best.confidence >= threshold:
            return [best]
        return []

    # ── Robust three-tier matching ───────────────────────────────────────────

    def match_robust(
        self,
        frame: np.ndarray,
        template_name: str,
        region: Region | None = None,
        threshold: float = 0.75,
        scale_range: tuple[float, float] = DEFAULT_SCALE_RANGE,
        scale_steps: int = DEFAULT_SCALE_STEPS,
    ) -> list[Detection]:
        """Three-tier robust matching, each tier fast:
          0. Temporal coherence: search a small window around the last match
             position first (5-10x faster for repeated detection).
          1. multiscale CLAHE correlation (coarse-to-fine)
          2. multiscale Canny-edge correlation (same sweep, edge images)
          3. ORB + FLANN feature matching (scale/rotation invariant)
        Returns the first tier that finds a match. Never raises — any internal
        error (bad frame, corrupt template, OpenCV exception) returns [].
        """
        try:
            cache = self._get_cache(template_name)
            if region:
                search = self.extract_roi(frame, region)
                ox, oy = self.roi_origin(region)
            else:
                search = frame
                ox, oy = 0, 0

            # Tier 0: Temporal coherence — search a small window around the
            # last match position. If the template was found at (x, y) last
            # frame, it's very likely still near there. Searching a ±60px
            # window is 5-10x faster than full-frame.
            last = self._last_match.get(template_name)
            if last is not None:
                lx, ly, lw, lh = last
                # Window in search-area coords (subtract ROI origin)
                wx = lx - ox
                wy = ly - oy
                pad = 60
                x1 = max(0, wx - lw // 2 - pad)
                y1 = max(0, wy - lh // 2 - pad)
                x2 = min(search.shape[1], wx + lw // 2 + pad)
                y2 = min(search.shape[0], wy + lh // 2 + pad)
                if x2 - x1 >= lw and y2 - y1 >= lh:
                    window = search[y1:y2, x1:x2]
                    hits = self._match_corr(
                        self._clahe.apply(self._to_gray(window)),
                        cache.clahe, threshold,
                        ox + x1, oy + y1, template_name,
                        mask=cache.mask,
                    )
                    if hits:
                        # Update last match position
                        best = hits[0]
                        self._last_match[template_name] = (best.x, best.y, best.w, best.h)
                        return hits

            # Tier 1: multiscale correlation
            hits = self._scale_sweep(search, cache, threshold, ox, oy, template_name,
                                     scale_range, scale_steps, use_edges=False)
            if hits:
                best = hits[0]
                self._last_match[template_name] = (best.x, best.y, best.w, best.h)
                return hits

            # Tier 2: edge-based correlation
            edge_threshold = max(threshold - 0.1, 0.5)
            hits = self._scale_sweep(search, cache, edge_threshold, ox, oy, template_name,
                                     scale_range, scale_steps, use_edges=True)
            if hits:
                best = hits[0]
                self._last_match[template_name] = (best.x, best.y, best.w, best.h)
                return hits

            # Tier 3: ORB feature matching
            hits = self._match_orb(search, cache, ox, oy, template_name)
            if hits:
                best = hits[0]
                self._last_match[template_name] = (best.x, best.y, best.w, best.h)
            return hits
        except Exception as e:
            logger.warning("match_robust failed for '%s': %s", template_name, e)
            return []

    def _match_orb(
        self, search: np.ndarray, cache: _TemplateCache,
        ox: int, oy: int, label: str, min_good: int = 6,
    ) -> list[Detection]:
        """ORB feature matching with cached template descriptors + FLANN-LSH.

        Runs on a downscaled copy of the search area: feature matching does not
        need native resolution, and fewer keypoints makes the match an order of
        magnitude faster. Detected coordinates are scaled back up.
        """
        if cache.orb_des is None or len(cache.orb_kp) < min_good:
            return []

        sh, sw = search.shape[:2]
        of = self.ORB_FACTOR
        ow, oh = max(1, int(sw * of)), max(1, int(sh * of))
        search_small = cv2.resize(self._to_gray(search), (ow, oh), interpolation=cv2.INTER_AREA)

        kp2, des2 = self._orb.detectAndCompute(search_small, None)
        if des2 is None or len(kp2) < min_good:
            return []

        matcher = self._get_flann()
        try:
            raw = matcher.knnMatch(cache.orb_des, des2, k=2)
        except Exception:
            return []

        good = [m for pair in raw if len(pair) == 2
                for m, n in [pair] if m.distance < 0.75 * n.distance]
        if len(good) < min_good:
            return []

        src_pts = np.float32([cache.orb_kp[m.queryIdx].pt for m in good]).reshape(-1, 1, 2)
        # Scale matched points back up to full search-area resolution
        dst_pts = np.float32([kp2[m.trainIdx].pt for m in good]).reshape(-1, 1, 2) / of
        try:
            M, mask = cv2.findHomography(src_pts, dst_pts, cv2.RANSAC, 5.0)
        except cv2.error:
            return []
        if M is None:
            return []

        h, w = cache.h, cache.w
        corners = np.float32([[0, 0], [w, 0], [w, h], [0, h]]).reshape(-1, 1, 2)
        try:
            warped = cv2.perspectiveTransform(corners, M)
        except cv2.error:
            return []
        xs = warped[:, 0, 0]
        ys = warped[:, 0, 1]
        cx = int(float(np.mean(xs)))
        cy = int(float(np.mean(ys)))
        bw = int(float(np.max(xs) - np.min(xs)))
        bh = int(float(np.max(ys) - np.min(ys)))

        inliers = int(mask.sum()) if mask is not None else len(good)
        confidence = min(1.0, inliers / max(1, len(good)))
        return [Detection(
            label=label, x=ox + cx, y=oy + cy, w=max(bw, 1), h=max(bh, 1),
            confidence=confidence, roi_offset=(ox, oy),
        )]

    def _get_flann(self):
        if self._flann is None:
            # FLANN-LSH for binary ORB descriptors — much faster than brute force.
            index_params = dict(algorithm=6, table_number=6, key_size=12, multi_probe_level=1)
            search_params = dict(checks=50)
            self._flann = cv2.FlannBasedMatcher(index_params, search_params)
        return self._flann

    @staticmethod
    def _nms(detections: list[Detection], overlap_thresh: float = 0.5) -> list[Detection]:
        if not detections:
            return []
        boxes = np.array([[d.x - d.w // 2, d.y - d.h // 2, d.x + d.w // 2, d.y + d.h // 2] for d in detections])
        scores = np.array([d.confidence for d in detections])

        x1, y1, x2, y2 = boxes[:, 0], boxes[:, 1], boxes[:, 2], boxes[:, 3]
        areas = (x2 - x1) * (y2 - y1)
        order = scores.argsort()[::-1]

        keep = []
        while order.size > 0:
            i = order[0]
            keep.append(i)
            xx1 = np.maximum(x1[i], x1[order[1:]])
            yy1 = np.maximum(y1[i], y1[order[1:]])
            xx2 = np.minimum(x2[i], x2[order[1:]])
            yy2 = np.minimum(y2[i], y2[order[1:]])
            w = np.maximum(0, xx2 - xx1)
            h = np.maximum(0, yy2 - yy1)
            overlap = (w * h) / areas[order[1:]]
            inds = np.where(overlap <= overlap_thresh)[0]
            order = order[inds + 1]

        return [detections[i] for i in keep]

    # ── Feature matching (ORB — scale/rotation invariant, public API) ────────

    def match_features(
        self,
        frame: np.ndarray,
        template_name: str,
        region: Region | None = None,
        threshold: float = 0.75,
        min_good: int = 6,
    ) -> list[Detection]:
        """Locate a template via ORB feature matching (cached descriptors).

        Survives scale changes, rotation, and mild perspective. Faster than the
        previous AKAZE + brute-force path. `threshold` is accepted for API
        compatibility; ORB uses a fixed Lowe ratio of 0.75.
        """
        cache = self._get_cache(template_name)
        if region:
            search = self.extract_roi(frame, region)
            ox, oy = self.roi_origin(region)
        else:
            search = frame
            ox, oy = 0, 0
        return self._match_orb(search, cache, ox, oy, template_name, min_good=min_good)

    # ── OCR ──────────────────────────────────────────────────────────────────

    def _init_ocr(self) -> None:
        if self._ocr_reader is None:
            import easyocr
            self._ocr_reader = easyocr.Reader(["en"], gpu=False)
            logger.info("EasyOCR initialized")

    def ocr_region(
        self,
        frame: np.ndarray,
        region: Region,
        color_filter: HSVRange | None = None,
    ) -> str:
        """OCR a specific region. Optionally pre-filter by color to isolate text."""
        self._init_ocr()
        roi = self.extract_roi(frame, region)

        if color_filter:
            hsv = cv2.cvtColor(roi, cv2.COLOR_BGR2HSV)
            lower = np.array(color_filter.lower, dtype=np.uint8)
            upper = np.array(color_filter.upper, dtype=np.uint8)
            mask = cv2.inRange(hsv, lower, upper)
            # White text on black background for OCR
            roi = cv2.bitwise_and(roi, roi, mask=mask)

        results = self._ocr_reader.readtext(roi, detail=0, paragraph=True)
        return " ".join(results).strip()

    def ocr_find(
        self,
        frame: np.ndarray,
        text: str,
        region: Region | None = None,
        color_filter: HSVRange | None = None,
    ) -> list[Detection]:
        """Find on-screen text and return its location(s). Case-insensitive substring match."""
        self._init_ocr()
        if region:
            roi = self.extract_roi(frame, region)
            ox, oy = self.roi_origin(region)
        else:
            roi = frame
            ox, oy = 0, 0

        if color_filter:
            hsv = cv2.cvtColor(roi, cv2.COLOR_BGR2HSV)
            lower = np.array(color_filter.lower, dtype=np.uint8)
            upper = np.array(color_filter.upper, dtype=np.uint8)
            mask = cv2.inRange(hsv, lower, upper)
            roi = cv2.bitwise_and(roi, roi, mask=mask)

        target = text.strip().lower()
        results = self._ocr_reader.readtext(roi, detail=1, paragraph=False)
        detections = []
        for bbox, found_text, conf in results:
            if target and target not in found_text.strip().lower():
                continue
            # bbox is 4 corners: [[x1,y1],[x2,y2],[x3,y3],[x4,y4]]
            xs = [p[0] for p in bbox]
            ys = [p[1] for p in bbox]
            cx = int(sum(xs) / 4)
            cy = int(sum(ys) / 4)
            w = int(max(xs) - min(xs))
            h = int(max(ys) - min(ys))
            detections.append(Detection(
                label=found_text,
                x=ox + cx, y=oy + cy, w=max(w, 1), h=max(h, 1),
                confidence=float(conf),
                roi_offset=(ox, oy),
            ))
        detections.sort(key=lambda d: d.confidence, reverse=True)
        return detections

    def ocr_number(
        self,
        frame: np.ndarray,
        region: Region,
        color_filter: HSVRange | None = None,
    ) -> int | None:
        """OCR a region expected to contain a number. Returns parsed int or None."""
        text = self.ocr_region(frame, region, color_filter)
        # Strip non-digit chars (¥, commas, spaces, 'k', etc.)
        cleaned = text.replace(",", "").replace("¥", "").replace(" ", "")
        # Handle "8.9k" style
        if "k" in cleaned.lower():
            try:
                return int(float(cleaned.lower().replace("k", "")) * 1000)
            except ValueError:
                return None
        digits = "".join(c for c in cleaned if c.isdigit())
        return int(digits) if digits else None
