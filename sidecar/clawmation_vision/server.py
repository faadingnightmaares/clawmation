"""Clawmation vision sidecar — JSON-RPC over stdio.

The Rust/Tauri backend owns the app spine; this process owns the cv2/easyocr
vision compute. It holds ONE long-lived `PixelDetector` and ONE `ScreenCapture`
for the whole process lifetime — the detector is stateful (per-template cache,
temporal-coherence `_last_match`, lazy easyocr model load), exactly as the
original Python app shared a single detector + capture across its GuardEngine,
VisionAgent, and guard-test paths. Requests are answered one at a time, so that
single detector is never touched concurrently.

Protocol — newline-delimited JSON, one object per line. Request:

    {"id": <n>, "method": "<name>", "params": {...}}

Response (exactly one per request, same `id`):

    {"id": <n>, "ok": true,  "result": <value>}
    {"id": <n>, "ok": false, "error": "<message>"}

Every detection method grabs its own frame from the capture, so a frame never
crosses the process boundary — matching the original, where the frame provider
and the detector lived in one process and the ~11 MB array was never serialized.
The orchestration that surrounds detection (cooldowns, pause/resume, click/key
actions, macro sequencing) lives in the Rust backend, which drives this over RPC.

Run as: ``python -m clawmation_vision.server`` (stdio). stdout carries only
protocol JSON; all logging goes to stderr.
"""
from __future__ import annotations

import json
import logging
import os
import sys
from pathlib import Path

from .capture import ScreenCapture
from .config import HSVRange, Region
from .detection import Detection, PixelDetector
from .guard import Guard, detect_guard
from .steps import Step, ai_detect, test_step

logger = logging.getLogger("clawmation_vision")


def _det_dict(d: Detection) -> dict:
    """Serialize a Detection, coercing any numpy scalars to JSON-native types."""
    return {
        "label": d.label,
        "x": int(d.x),
        "y": int(d.y),
        "w": int(d.w),
        "h": int(d.h),
        "confidence": float(d.confidence),
        "roi_offset": [int(d.roi_offset[0]), int(d.roi_offset[1])],
    }


def _hsv(low: list, high: list) -> HSVRange:
    """Build an HSVRange from [h, s, v] lower/upper triples — the exact
    interleaving `detect_guard`'s color branch uses."""
    return HSVRange(low[0], high[0], low[1], high[1], low[2], high[2])


def _region(r) -> Region | None:
    """A [x1, y1, x2, y2] percentage box, or None. A full-screen box collapses
    to None so the detector searches the bare frame (matches `detect_guard`)."""
    if r is None:
        return None
    is_full = r[0] <= 0.5 and r[1] <= 0.5 and r[2] >= 99.5 and r[3] >= 99.5
    return None if is_full else Region(*r)


class VisionServer:
    """Long-lived detector + capture behind a small RPC method table."""

    def __init__(self) -> None:
        self._detector: PixelDetector | None = None
        self._capture: ScreenCapture | None = None
        self._handlers = {
            "ping": self._ping,
            "init": self._init,
            "load_template": self._load_template,
            "detect_guards": self._detect_guards,
            "detect_guard": self._detect_guard,
            "guard_test": self._guard_test,
            "detect_color": self._detect_color,
            "detect_checkpoint": self._detect_checkpoint,
            "ai_detect": self._ai_detect,
            "ai_test_step": self._ai_test_step,
            "color_present": self._color_present,
            "ocr_find": self._ocr_find,
            "capture_fps": self._capture_fps,
            "guard_pick_color": self._guard_pick_color,
            "guard_pick_region": self._guard_pick_region,
            "capture_template": self._capture_template,
            "add_template_image": self._add_template_image,
            "surgical_capture": self._surgical_capture,
            "shutdown": self._shutdown,
        }

    # ── readiness / frame source ─────────────────────────────────────────────
    @property
    def _ready(self) -> bool:
        return self._detector is not None and self._capture is not None

    def _require_ready(self) -> None:
        if not self._ready:
            raise RuntimeError("sidecar not initialized; call 'init' first")

    def _grab(self):
        self._require_ready()
        frame = self._capture.grab()
        if frame is None:
            raise RuntimeError("capture returned no frame")
        return frame

    def _detect_on_frame(self, guard_dict: dict, frame) -> list[Detection]:
        """The guard seam, isolated so the live grab-path and tests share it."""
        self._require_ready()
        return detect_guard(self._detector, Guard.from_dict(guard_dict), frame)

    # ── handlers ─────────────────────────────────────────────────────────────
    def _ping(self, p: dict) -> dict:
        return {"pong": True, "ready": self._ready}

    def _init(self, p: dict) -> dict:
        w = int(p["screen_w"])
        h = int(p["screen_h"])
        backend = p.get("capture_backend", "dxcam")
        self._detector = PixelDetector(screen_w=w, screen_h=h)
        self._capture = ScreenCapture(backend=backend)
        return {"backend": self._capture.backend, "screen_w": w, "screen_h": h}

    def _load_template(self, p: dict) -> dict:
        self._require_ready()
        self._detector.load_template(p["name"], p["path"])
        return {"loaded": p["name"]}

    def _detect_guards(self, p: dict) -> dict:
        """One frame, many guards — preserves the poll-loop's one-frame-per-cycle
        temporal consistency (every guard in a cycle sees the same frame)."""
        frame = self._grab()
        return {
            g.get("id", ""): [_det_dict(d) for d in self._detect_on_frame(g, frame)]
            for g in p["guards"]
        }

    def _detect_guard(self, p: dict) -> list:
        frame = self._grab()
        return [_det_dict(d) for d in self._detect_on_frame(p["guard"], frame)]

    def _guard_test(self, p: dict) -> dict:
        """Dry-run a guard against a fresh frame — match + preview image. The
        guard editor's Test button; a verbatim port of the desktop `Api.guard_test`.
        A None frame returns the inner `{ok: false, ...}` result (not a transport
        error) so the message reaches the editor exactly as the monolith returned."""
        self._require_ready()
        frame = self._capture.grab()
        if frame is None:
            return {"ok": False, "error": "Could not capture frame"}
        return self._guard_test_on_frame(p["guard"], frame)

    def _guard_test_on_frame(self, guard_dict: dict, frame) -> dict:
        """guard_test's compute — detect, draw the preview, assemble the result —
        split from the grab so tests drive it with a synthesized frame, exactly as
        `_detect_on_frame` is. Region box, match rings, and the best-match crosshair
        keep the desktop app's BGR colors so the preview is pixel-identical."""
        import base64
        import cv2
        self._require_ready()
        try:
            g = Guard.from_dict(guard_dict)
        except Exception as e:
            return {"ok": False, "error": f"Bad guard: {e}"}

        matches = detect_guard(self._detector, g, frame)

        # Draw region box + matches onto a preview.
        preview = frame.copy()
        h, w = frame.shape[:2]
        x1 = int(g.region[0] / 100 * w); y1 = int(g.region[1] / 100 * h)
        x2 = int(g.region[2] / 100 * w); y2 = int(g.region[3] / 100 * h)
        cv2.rectangle(preview, (x1, y1), (x2, y2), (74, 168, 192), 2)
        for m in matches[:20]:
            cv2.circle(preview, (m.x, m.y), 8, (95, 174, 95), 2)
        if matches:
            best = matches[0]
            cv2.circle(preview, (best.x, best.y), 14, (95, 95, 255), 3)
            cv2.line(preview, (best.x - 20, best.y), (best.x + 20, best.y), (95, 95, 255), 2)
            cv2.line(preview, (best.x, best.y - 20), (best.x, best.y + 20), (95, 95, 255), 2)

        ok, buf = cv2.imencode(".jpg", preview, [cv2.IMWRITE_JPEG_QUALITY, 70])
        preview_b64 = base64.b64encode(buf.tobytes()).decode("ascii") if ok else ""

        found = matches[0] if matches else None
        conf = round(float(found.confidence), 3) if found else 0.0
        return {
            "ok": bool(matches),
            "matched": len(matches),
            "found_x": int(found.x) if found else -1,
            "found_y": int(found.y) if found else -1,
            "confidence": conf,
            "message": (f"{len(matches)} match(es) · conf {conf:.2f}" if matches else "no match"),
            "preview": preview_b64,
        }

    def _detect_color(self, p: dict) -> list:
        frame = self._grab()
        dets = self._detector.detect_color(
            frame, _hsv(p["hsv_low"], p["hsv_high"]), _region(p.get("region")),
            min_area=int(p.get("min_area", 50)), label=p.get("label", "color"),
        )
        return [_det_dict(d) for d in dets]

    def _detect_checkpoint(self, p: dict) -> list:
        """One poll of a checkpoint's detector — the detection half of
        `MacroPlayer._run_checkpoint.detect`. The checkpoint dict arrives as `p`:
        `method` routes template vs colour, `region` collapses a full-screen box
        to None, and the checkpoint defaults (threshold 0.75, min_area 40, label
        "checkpoint") differ from the generic `detect_color` handler. A detect
        failure yields no matches, exactly as the in-process closure swallowed its
        own exceptions to []."""
        frame = self._grab()
        method = p.get("method", "color")
        region = _region(p.get("region") or [0, 0, 100, 100])
        if method == "template" and p.get("template"):
            try:
                tpl_key = self._detector.ensure_template_path(p["template"])
                dets = self._detector.match_robust(
                    frame, tpl_key, region, threshold=float(p.get("threshold", 0.75)),
                )
            except Exception as e:
                logger.warning("Checkpoint template detect failed: %s", e)
                dets = []
            return [_det_dict(d) for d in dets]
        try:
            dets = self._detector.detect_color(
                frame, _hsv(p["hsv_low"], p["hsv_high"]), region,
                min_area=int(p.get("min_area", 40)), label="checkpoint",
            )
        except Exception as e:
            logger.warning("Checkpoint color detect failed: %s", e)
            dets = []
        return [_det_dict(d) for d in dets]

    def _ai_detect(self, p: dict) -> dict:
        """One detection for a single AI step — the run loop's per-step probe
        (Rust polls this for `find_click`/`wait_for`). Grabs a fresh frame and
        returns the matches plus the same status message the monolith's
        `AIExecutor._detect` built, so Rust composes the run log verbatim."""
        frame = self._grab()
        matches, message = ai_detect(self._detector, Step.from_dict(p["step"]), frame)
        return {
            "matches": [
                {"x": int(m.x), "y": int(m.y), "confidence": float(m.confidence)}
                for m in matches
            ],
            "message": message,
        }

    def _ai_test_step(self, p: dict) -> dict:
        """Dry-run a single step against a fresh frame — result + preview image.
        The step editor's Test button; a verbatim port of the desktop
        `Api.ai_test_step`. The step is parsed BEFORE the frame is grabbed, so a
        malformed step reports "Bad step: …" even when capture would also fail —
        the monolith's order, which differs from `guard_test`'s grab-first."""
        self._require_ready()
        try:
            st = Step.from_dict(p["step"])
        except Exception as e:
            return {"ok": False, "error": f"Bad step: {e}"}
        frame = self._capture.grab()
        if frame is None:
            return {"ok": False, "error": "Could not capture frame"}
        return self._ai_test_step_on_frame(st, frame)

    def _ai_test_step_on_frame(self, st: Step, frame) -> dict:
        """test_step's compute — evaluate the step, draw the preview, assemble the
        result — split from the grab so tests drive it with a synthesized frame, as
        `_guard_test_on_frame` is. The preview keeps `_draw_step_preview`'s BGR
        colors and its cyan region box (distinct from guard's blue), and re-detects
        colour on the copy AFTER drawing the box, so it's pixel-identical."""
        import base64
        import cv2
        self._require_ready()
        result = test_step(self._detector, st, frame)

        preview = frame.copy()
        h, w = frame.shape[:2]
        x1 = int(st.region[0] / 100 * w); y1 = int(st.region[1] / 100 * h)
        x2 = int(st.region[2] / 100 * w); y2 = int(st.region[3] / 100 * h)
        cv2.rectangle(preview, (x1, y1), (x2, y2), (235, 168, 74), 2)
        if st.type in ("find_click", "wait_for") and st.detect_mode == "color":
            region = Region(*st.region)
            hsv = HSVRange(
                st.hsv_low[0], st.hsv_high[0],
                st.hsv_low[1], st.hsv_high[1],
                st.hsv_low[2], st.hsv_high[2],
            )
            matches = self._detector.detect_color(preview, hsv, region, min_area=st.min_area)
            for m in matches[:20]:
                cv2.circle(preview, (m.x, m.y), 8, (95, 174, 95), 2)
            if matches:
                best = matches[0]
                cv2.circle(preview, (best.x, best.y), 14, (95, 95, 255), 3)
                cv2.line(preview, (best.x - 20, best.y), (best.x + 20, best.y), (95, 95, 255), 2)
                cv2.line(preview, (best.x, best.y - 20), (best.x, best.y + 20), (95, 95, 255), 2)
        elif st.type == "click":
            cv2.circle(preview, (st.x, st.y), 14, (95, 95, 255), 3)
            cv2.line(preview, (st.x - 20, st.y), (st.x + 20, st.y), (95, 95, 255), 2)
            cv2.line(preview, (st.x, st.y - 20), (st.x, st.y + 20), (95, 95, 255), 2)

        ok, buf = cv2.imencode(".jpg", preview, [cv2.IMWRITE_JPEG_QUALITY, 70])
        preview_b64 = base64.b64encode(buf.tobytes()).decode("ascii") if ok else ""
        return {
            "ok": result.ok,
            "message": result.message,
            "found_x": int(result.found_x),
            "found_y": int(result.found_y),
            "matched": int(result.matched),
            "confidence": round(result.confidence, 3),
            "preview": preview_b64,
        }

    def _color_present(self, p: dict) -> dict:
        frame = self._grab()
        present = self._detector.color_present(
            frame, _hsv(p["hsv_low"], p["hsv_high"]), Region(*p["region"]),
            min_pixel_ratio=float(p.get("min_pixel_ratio", 0.05)),
        )
        return {"present": bool(present)}

    def _ocr_find(self, p: dict) -> list:
        frame = self._grab()
        dets = self._detector.ocr_find(frame, p["text"], _region(p.get("region")))
        return [_det_dict(d) for d in dets]

    def _capture_fps(self, p: dict) -> dict:
        self._require_ready()
        return {"fps": round(float(self._capture.fps), 1)}

    def _shutdown(self, p: dict) -> dict:
        return {"bye": True}

    # ── editor setup pickers ─────────────────────────────────────────────────
    # Interactive Win32 overlays the guard/trigger editor opens to sample a
    # colour, drag a watch region, or capture/import a button template — verbatim
    # ports of the desktop `Api.*` pickers. Each runs its own one-shot capture
    # (never the shared detector) and blocks on the user, so it holds the io lock
    # for its whole lifetime, exactly as the desktop app blocked its UI thread.
    # The template pickers take their save directory by value (`templates_dir`)
    # rather than reading `guards.TEMPLATES_DIR`, keeping the sidecar path-global
    # free like the rest of the process.
    def _guard_pick_color(self, p: dict) -> dict:
        from .color_sampler import sample_color
        res = sample_color()
        if not res:
            return {"ok": False, "error": "cancelled"}
        return {"ok": True, **res}

    def _guard_pick_region(self, p: dict) -> dict:
        from .region_picker import pick_region_rect
        res = pick_region_rect()
        if res is None:
            return {"ok": False, "error": "cancelled"}
        x, y, w, h = res
        return {"ok": True, "x": x, "y": y, "w": w, "h": h}

    def _capture_template(self, p: dict) -> dict:
        import base64
        import time as _time

        import cv2

        from .region_picker import NativeRegionPicker
        templates_dir = Path(p["templates_dir"])

        cap = ScreenCapture(backend="mss")
        frame = cap.grab()
        cap.close()
        if frame is None:
            return {"ok": False, "error": "Could not capture screen"}

        picker = NativeRegionPicker(frame)
        region = picker.pick()
        if region is None:
            return {"ok": False, "error": "cancelled"}

        x, y, w, h = region
        fh, fw = frame.shape[:2]
        x = max(0, min(x, fw - 1)); y = max(0, min(y, fh - 1))
        w = min(w, fw - x); h = min(h, fh - y)
        if w < 8 or h < 8:
            return {"ok": False, "error": "Selection too small"}

        crop = frame[y:y + h, x:x + w]
        templates_dir.mkdir(parents=True, exist_ok=True)
        path = templates_dir / f"template_{int(_time.time())}_{w}x{h}.png"
        cv2.imwrite(str(path), crop)

        thumb = crop.copy()
        th = 64
        scale = th / crop.shape[0]
        thumb = cv2.resize(crop, (int(crop.shape[1] * scale), th))
        ok, buf = cv2.imencode(".png", thumb)
        thumb_b64 = base64.b64encode(buf.tobytes()).decode("ascii") if ok else ""
        return {"ok": True, "path": str(path), "w": w, "h": h, "thumb": thumb_b64}

    def _surgical_capture(self, p: dict) -> dict:
        from .surgical_picker import surgical_capture
        res = surgical_capture(Path(p["templates_dir"]))
        if res is None:
            return {"ok": False, "error": "cancelled"}
        return {"ok": True, **res}

    def _add_template_image(self, p: dict) -> dict:
        import base64
        import time as _time

        import cv2
        templates_dir = Path(p["templates_dir"])

        # tkinter ships with Python — a file dialog with no extra dependency.
        try:
            import tkinter as tk
            from tkinter import filedialog
            root = tk.Tk()
            root.withdraw()
            root.attributes("-topmost", True)
            filepath = filedialog.askopenfilename(
                title="Select button image",
                filetypes=[
                    ("Images", "*.png *.jpg *.jpeg *.bmp *.webp"),
                    ("All files", "*.*"),
                ],
            )
            root.destroy()
        except Exception as e:
            return {"ok": False, "error": f"File dialog failed: {e}"}

        if not filepath:
            return {"ok": False, "error": "cancelled"}

        img = cv2.imread(filepath, cv2.IMREAD_COLOR)
        if img is None:
            return {"ok": False, "error": "Could not read image file"}
        h, w = img.shape[:2]
        if w < 8 or h < 8:
            return {"ok": False, "error": "Image too small"}

        templates_dir.mkdir(parents=True, exist_ok=True)
        path = templates_dir / f"template_{int(_time.time())}_{w}x{h}.png"
        cv2.imwrite(str(path), img)

        thumb = img.copy()
        th = 64
        scale = th / h
        thumb = cv2.resize(img, (int(w * scale), th))
        ok, buf = cv2.imencode(".png", thumb)
        thumb_b64 = base64.b64encode(buf.tobytes()).decode("ascii") if ok else ""
        return {"ok": True, "path": str(path), "w": w, "h": h, "thumb": thumb_b64}

    # ── dispatch ─────────────────────────────────────────────────────────────
    def handle(self, req: dict) -> dict:
        rid = req.get("id")
        handler = self._handlers.get(req.get("method"))
        if handler is None:
            return {"id": rid, "ok": False, "error": f"unknown method: {req.get('method')!r}"}
        try:
            return {"id": rid, "ok": True, "result": handler(req.get("params") or {})}
        except Exception as e:  # never let one bad request kill the loop
            logger.warning("request %s failed: %s", rid, e)
            return {"id": rid, "ok": False, "error": f"{type(e).__name__}: {e}"}

    def serve(self, stdin, write) -> None:
        for line in stdin:
            line = line.strip()
            if not line:
                continue
            try:
                req = json.loads(line)
            except Exception as e:
                write({"id": None, "ok": False, "error": f"malformed request: {e}"})
                continue
            write(self.handle(req))
            if req.get("method") == "shutdown":
                break


def _reserve_stdout():
    """Hand back a UTF-8 text stream on the *real* stdout, then point OS fd 1 at
    stderr so nothing else can write to it. stdout is the protocol channel and
    must carry only JSON-RPC — but a library can bypass `sys.stdout` and write to
    fd 1 directly. The concrete offender: easyocr prints ``"Using CPU. Note: this
    module is much faster with a GPU."`` (a bare ``print``, not a log) to stdout
    the first time an OCR guard loads its model; that lone non-JSON line would
    desync the Rust client's read loop for every response after it. Redirecting
    fd 1 to stderr sends such stray output to the log stream where it belongs, and
    the returned stream is the only path left to the true protocol pipe."""
    real_fd = os.dup(1)          # save the true stdout before reassigning fd 1
    os.dup2(2, 1)                # fd 1 now goes wherever stderr goes
    return os.fdopen(real_fd, "w", encoding="utf-8", closefd=True)


def main() -> None:
    logging.basicConfig(
        level=logging.WARNING, stream=sys.stderr,
        format="%(levelname)s %(name)s: %(message)s",
    )
    # Force UTF-8 on stdin so non-ASCII guard names survive Windows' cp1252 stdio.
    sys.stdin.reconfigure(encoding="utf-8")
    protocol_out = _reserve_stdout()

    server = VisionServer()

    def write(obj: dict) -> None:
        protocol_out.write(json.dumps(obj) + "\n")
        protocol_out.flush()

    server.serve(sys.stdin, write)


if __name__ == "__main__":
    main()
