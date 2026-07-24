"""The guard detection seam — `Guard` plus `detect_guard`, lifted verbatim from
`anime_macro.guards`. This is the exact Rust/Python boundary: everything here is
per-frame vision dispatch (region math + method routing into the cv2/easyocr
primitives), which stays in Python. The orchestration that used to surround it in
`guards.py` — GuardEngine, VisionAgent, load/save, cooldowns, pause/resume,
actions — now lives in the Rust backend, which drives this over RPC.
"""
from __future__ import annotations

import logging
from dataclasses import dataclass, field, asdict

import numpy as np

from .config import Region, HSVRange
from .detection import PixelDetector

logger = logging.getLogger(__name__)


@dataclass
class Guard:
    """A single screen-watcher attached to a macro.

    Detection method:
      - "color":    HSV color blob (fastest, least specific)
      - "template": multi-scale image match (fast + very accurate for buttons)
      - "ocr":      read on-screen text (most robust to visual changes, slowest)
    """
    id: str
    name: str = "Guard"
    method: str = "color"  # "color" | "template" | "ocr"

    # Trigger (color method): HSV color blob inside a percentage region
    hsv_low: list[int] = field(default_factory=lambda: [0, 0, 0])
    hsv_high: list[int] = field(default_factory=lambda: [179, 255, 255])
    region: list[float] = field(default_factory=lambda: [0.0, 0.0, 100.0, 100.0])
    min_area: int = 40

    # Trigger (template method): saved template image + match threshold
    template_path: str = ""
    threshold: float = 0.8
    # Surgical click offset: [x, y] relative to the matched region's top-left.
    # When set, the guard clicks this exact pixel instead of the match center,
    # keeping the click glued to the element regardless of background.
    click_offset: list = field(default_factory=list)
    # Surgical drawn line: [sx, sy, ex, ey] relative to the match top-left.
    # When set, the guard DRAGS along this path (a sweep) instead of a single
    # click — perfect for sliders/charge bars. Takes priority over click_offset.
    click_line: list = field(default_factory=list)
    # Surgical drawn strokes: list of [sx, sy, ex, ey]. Every stroke is replayed
    # as a smooth drag, in order. Takes priority over click_line/click_offset.
    click_lines: list = field(default_factory=list)

    # Trigger (ocr method): text to find on screen
    ocr_text: str = ""

    # Action when triggered
    action: str = "click"      # "click" | "key"
    key: str = ""              # for action == "key"

    # Orchestration: run a sequence of macros after the action.
    # Each entry: {"name": str, "repeat": int}  (repeat=0 → infinite, but
    # VisionAgent caps it to a sane default so a runaway loop can't wedge it).
    macro_sequence: list = field(default_factory=list)

    # Timing
    resume_delay: float = 3.0  # seconds to wait after the action, before resuming
    cooldown: float = 2.0      # min seconds between consecutive firings

    enabled: bool = True

    def to_dict(self) -> dict:
        return asdict(self)

    @classmethod
    def from_dict(cls, d: dict) -> "Guard":
        known = {k: v for k, v in d.items() if k in cls.__dataclass_fields__}
        return cls(**known)


def detect_guard(detector: PixelDetector, guard: Guard, frame: np.ndarray) -> list:
    """Run a guard's configured detection method against a frame.

    Shared by the live GuardEngine and the dry-run guard_test API so both
    behave identically. Returns a list of Detection objects. Never raises —
    any internal error (bad frame, corrupt template, OpenCV exception) returns [].
    """
    try:
        r = guard.region or [0, 0, 100, 100]
        # Full-screen region -> pass None so the detector searches the bare frame
        # and skips a wasteful full-frame copy.
        is_full = r[0] <= 0.5 and r[1] <= 0.5 and r[2] >= 99.5 and r[3] >= 99.5
        region = None if is_full else Region(*r)
        method = guard.method or "color"

        if method == "template":
            # Three-tier robust match: multiscale correlation → edge matching → ORB.
            # Survives UI scaling from 30% to 200%, brightness shifts, and rotation.
            if not guard.template_path:
                return []
            tpl_name = f"_guard_{guard.id}"
            try:
                if tpl_name not in detector._templates:
                    detector.load_template(tpl_name, guard.template_path)
            except Exception as e:
                logger.warning("Guard '%s' template failed to load: %s", guard.name, e)
                return []
            return detector.match_robust(
                frame, tpl_name, region,
                threshold=guard.threshold,
            )

        if method == "ocr":
            # Read on-screen text — most robust to visual changes, slowest.
            if not guard.ocr_text:
                return []
            try:
                return detector.ocr_find(frame, guard.ocr_text, region)
            except Exception as e:
                logger.warning("Guard '%s' OCR failed: %s", guard.name, e)
                return []

        # Default: HSV color blob (fastest).
        hsv = HSVRange(
            guard.hsv_low[0], guard.hsv_high[0],
            guard.hsv_low[1], guard.hsv_high[1],
            guard.hsv_low[2], guard.hsv_high[2],
        )
        return detector.detect_color(
            frame, hsv, region, min_area=guard.min_area, label=guard.name
        )
    except Exception as e:
        logger.warning("detect_guard failed for '%s': %s", guard.name, e)
        return []
