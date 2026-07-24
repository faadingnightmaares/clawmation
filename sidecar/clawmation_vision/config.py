"""Detection primitives shared by the vision compute — the exact HSVRange /
Region / DEFAULT_RESOLUTION definitions the original `anime_macro.config`
exposed, decoupled from that module's path globals and their import-time
side effects (the sidecar is stateless: paths and resolution arrive by value
over RPC, never from a global). `detection.py` and `capture.py` import these
verbatim, so the definitions below are byte-for-byte the originals.
"""
from __future__ import annotations

from dataclasses import dataclass

# ── Screen ───────────────────────────────────────────────────────────────────
DEFAULT_RESOLUTION = (2560, 1440)


# ── Generic detection primitives ─────────────────────────────────────────────

@dataclass(frozen=True)
class Region:
    """A screen region defined by percentage bounds (x1, y1, x2, y2)."""
    x1: float
    y1: float
    x2: float
    y2: float

    def to_pixels(self, w: int, h: int) -> tuple[int, int, int, int]:
        return (
            int(self.x1 * w / 100),
            int(self.y1 * h / 100),
            int(self.x2 * w / 100),
            int(self.y2 * h / 100),
        )


@dataclass(frozen=True)
class HSVRange:
    """An HSV color range (H 0-179, S 0-255, V 0-255) for color-based detection."""
    h_low: int; h_high: int
    s_low: int; s_high: int
    v_low: int; v_high: int

    @property
    def lower(self) -> tuple[int, int, int]:
        return (self.h_low, self.s_low, self.v_low)

    @property
    def upper(self) -> tuple[int, int, int]:
        return (self.h_high, self.s_high, self.v_high)
