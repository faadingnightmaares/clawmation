"""The AI step detection seam — `Step` plus `ai_detect`/`test_step`, lifted
verbatim from `anime_macro.ai_macro`. Like `guard.py`, this is the exact
Rust/Python boundary: per-frame vision dispatch (region math + detect-mode
routing into the cv2 primitives) stays in Python. The orchestration that used to
surround it in `ai_macro.py` — the run loop, action execution, wait_for polling,
macro conversion, load/save — now lives in the Rust backend, which drives this
over RPC.
"""
from __future__ import annotations

import time
from dataclasses import dataclass, field, asdict

import numpy as np

from .config import Region, HSVRange
from .detection import PixelDetector


@dataclass
class Step:
    """A single AI macro step."""
    id: str
    type: str
    enabled: bool = True
    label: str = ""

    # Action params
    x: int = 0
    y: int = 0
    key: str = ""
    text: str = ""
    delay: float = 0.0
    scroll_amount: int = 0  # for scroll step: +N = up, -N = down

    # Detection params (for find_click / wait_for)
    detect_mode: str = "color"   # "color" | "template" | "features"
    hsv_low: list[int] = field(default_factory=lambda: [0, 0, 0])
    hsv_high: list[int] = field(default_factory=lambda: [179, 255, 255])
    template: str = ""           # template name in templates/
    region: list[float] = field(default_factory=lambda: [0.0, 0.0, 100.0, 100.0])  # % bounds
    min_area: int = 40
    timeout: float = 10.0        # for wait_for
    confidence: float = 0.8      # for template matching

    def to_dict(self) -> dict:
        return asdict(self)

    @classmethod
    def from_dict(cls, d: dict) -> "Step":
        known = {k: v for k, v in d.items() if k in cls.__dataclass_fields__}
        return cls(**known)


@dataclass
class StepResult:
    """Outcome of running/testing a single step."""
    ok: bool
    message: str
    found_x: int = -1
    found_y: int = -1
    matched: int = 0
    confidence: float = 0.0
    elapsed: float = 0.0


def ai_detect(detector: PixelDetector, step: Step, frame: np.ndarray) -> tuple[list, str]:
    """Run the step's detection on a frame. Returns (matches, message).

    Shared by the live run loop (Rust drives it per step via the `ai_detect` RPC)
    and the dry-run `test_step`, so both behave identically.

    NOTE: AI-step templates are never loaded into the detector's cache — the
    monolith only calls `load_template` for checkpoints and guards, never for
    steps. So `features` always raises the KeyError below → "template 'X' not
    loaded", and `template` returns 0 matches (`match_robust` swallows the same
    KeyError → []). Only `color` actually detects. This is the monolith's behavior
    on a live, user-selectable path (the step editor exposes all three modes and
    defaults find_click/wait_for to `template`), reproduced faithfully — not a new bug.
    """
    region = Region(*step.region)
    if step.detect_mode == "color":
        hsv = HSVRange(
            step.hsv_low[0], step.hsv_high[0],
            step.hsv_low[1], step.hsv_high[1],
            step.hsv_low[2], step.hsv_high[2],
        )
        matches = detector.detect_color(
            frame, hsv, region, min_area=step.min_area, label="target"
        )
        return matches, f"{len(matches)} color match(es)"
    elif step.detect_mode == "features":
        if not step.template:
            return [], "no template selected"
        try:
            matches = detector.match_features(
                frame, step.template, region, threshold=step.confidence
            )
        except KeyError:
            return [], f"template '{step.template}' not loaded"
        return matches, f"{len(matches)} feature match(es)"
    else:  # template — three-tier robust: multiscale → edges → AKAZE
        if not step.template:
            return [], "no template selected"
        try:
            matches = detector.match_robust(
                frame, step.template, region, threshold=step.confidence
            )
        except KeyError:
            return [], f"template '{step.template}' not loaded"
        return matches, f"{len(matches)} robust match(es)"


def test_step(detector: PixelDetector, step: Step, frame: np.ndarray) -> StepResult:
    """Dry-run a step's detection against a provided frame (no clicking)."""
    t0 = time.perf_counter()

    if step.type in ("find_click", "wait_for"):
        matches, msg = ai_detect(detector, step, frame)
        if matches:
            best = matches[0]
            return StepResult(True, f"would click ({best.x}, {best.y}) — {msg}",
                              best.x, best.y, len(matches), best.confidence,
                              time.perf_counter() - t0)
        return StepResult(False, f"nothing found — {msg}", elapsed=time.perf_counter() - t0)

    if step.type == "click":
        return StepResult(True, f"would click ({step.x}, {step.y})", step.x, step.y,
                          elapsed=time.perf_counter() - t0)
    if step.type == "key":
        return StepResult(True, f"would press '{step.key}'", elapsed=time.perf_counter() - t0)
    if step.type == "type":
        return StepResult(True, f"would type '{step.text}'", elapsed=time.perf_counter() - t0)
    if step.type == "scroll":
        return StepResult(True, f"would scroll {step.scroll_amount:+d}", elapsed=time.perf_counter() - t0)
    if step.type == "delay":
        return StepResult(True, f"would wait {step.delay}s", elapsed=time.perf_counter() - t0)

    return StepResult(False, f"unknown step type", elapsed=time.perf_counter() - t0)
