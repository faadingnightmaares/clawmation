"""Sidecar verification — runnable with pytest OR as a plain script
(``python tests/test_protocol.py``), so it needs no test runner installed.

Two things are proven:

  1. The JSON-RPC shell: readiness before/after ``init``, unknown-method and
     malformed-line errors, request ids echoed, ``shutdown`` ends the loop.
  2. The detection seam is intact end-to-end: a real guard runs through the
     verbatim ``detect_guard`` -> ``match_robust`` path and localizes a template
     on a supplied frame. A seeded-noise template is used so the match is
     deterministic and self-contained (no ``dist/`` assets, no easyocr).
"""
from __future__ import annotations

import base64
import io
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path
from unittest.mock import patch

# Make the sidecar package importable when run as a bare script.
SIDECAR_ROOT = Path(__file__).resolve().parent.parent
if str(SIDECAR_ROOT) not in sys.path:
    sys.path.insert(0, str(SIDECAR_ROOT))

import cv2  # noqa: E402
import numpy as np  # noqa: E402

from clawmation_vision.server import VisionServer  # noqa: E402
from clawmation_vision.steps import Step  # noqa: E402

SCREEN_W, SCREEN_H = 1400, 900


def _server() -> VisionServer:
    s = VisionServer()
    s.handle({"id": 0, "method": "init", "params": {
        "screen_w": SCREEN_W, "screen_h": SCREEN_H, "capture_backend": "mss"}})
    return s


# ── 1. protocol shell ────────────────────────────────────────────────────────
def test_ping_readiness_flips_after_init() -> None:
    s = VisionServer()
    before = s.handle({"id": 1, "method": "ping"})
    assert before == {"id": 1, "ok": True, "result": {"pong": True, "ready": False}}

    init = s.handle({"id": 2, "method": "init", "params": {
        "screen_w": SCREEN_W, "screen_h": SCREEN_H, "capture_backend": "mss"}})
    assert init["ok"] is True
    assert init["result"] == {"backend": "mss", "screen_w": SCREEN_W, "screen_h": SCREEN_H}

    after = s.handle({"id": 3, "method": "ping"})
    assert after["result"]["ready"] is True


def test_unknown_method_is_a_clean_error() -> None:
    s = VisionServer()
    resp = s.handle({"id": 7, "method": "does_not_exist"})
    assert resp["id"] == 7 and resp["ok"] is False
    assert "unknown method" in resp["error"]


def test_detect_before_init_errors_not_crashes() -> None:
    s = VisionServer()
    resp = s.handle({"id": 8, "method": "detect_guard", "params": {"guard": {"id": "g"}}})
    assert resp["ok"] is False
    assert "not initialized" in resp["error"]


def test_serve_handles_malformed_line_and_shutdown() -> None:
    s = VisionServer()
    out: list[dict] = []
    stdin = io.StringIO(
        "not json at all\n"
        '{"id": 1, "method": "ping"}\n'
        "\n"  # blank line skipped
        '{"id": 2, "method": "shutdown"}\n'
        '{"id": 3, "method": "ping"}\n'  # never reached: shutdown broke the loop
    )
    s.serve(stdin, out.append)

    assert out[0] == {"id": None, "ok": False, "error": out[0]["error"]}
    assert "malformed request" in out[0]["error"]
    assert out[1]["id"] == 1 and out[1]["result"]["pong"] is True
    assert out[2] == {"id": 2, "ok": True, "result": {"bye": True}}
    assert len(out) == 3  # loop stopped at shutdown; the trailing ping was ignored


def test_reserve_stdout_isolates_protocol_from_library_prints() -> None:
    """A stray write to stdout (e.g. easyocr's "Using CPU..." banner on first OCR
    model load) must never reach the protocol stream. `_reserve_stdout` moves fd 1
    to stderr and hands protocol writes a private stream on the real stdout, so a
    plain `print` lands on stderr while the JSON line stays clean on stdout."""
    snippet = (
        "import sys\n"
        "from clawmation_vision.server import _reserve_stdout\n"
        "out = _reserve_stdout()\n"
        "print('BANNER-TO-STDOUT')\n"            # a library print → must go to stderr
        "sys.stderr.write('LOG-LINE\\n')\n"
        "out.write('{\"protocol\": \"line\"}\\n')\n"
        "out.flush()\n"
    )
    env = {**os.environ, "PYTHONPATH": str(SIDECAR_ROOT), "PYTHONIOENCODING": "utf-8"}
    result = subprocess.run(
        [sys.executable, "-c", snippet],
        capture_output=True, text=True, encoding="utf-8", env=env,
    )
    assert result.returncode == 0, result.stderr
    # stdout carries the protocol line and nothing else.
    assert result.stdout.strip() == '{"protocol": "line"}', repr(result.stdout)
    assert "BANNER-TO-STDOUT" not in result.stdout
    # The banner was diverted to stderr.
    assert "BANNER-TO-STDOUT" in result.stderr


# ── 2. detection seam: real match through the verbatim path ──────────────────
def _noise_template(w: int, h: int, seed: int) -> np.ndarray:
    """A seeded-noise BGR patch. Noise autocorrelates sharply only at zero
    offset, so TM_CCOEFF localizes it unambiguously at the composited spot."""
    rng = np.random.RandomState(seed)
    return rng.randint(0, 256, (h, w, 3), dtype=np.uint8)


def test_template_guard_localizes_on_supplied_frame() -> None:
    s = _server()
    tw, th, px, py = 96, 64, 400, 250
    tpl = _noise_template(tw, th, seed=7)

    with tempfile.TemporaryDirectory() as d:
        tpl_path = Path(d) / "guard_tpl.png"
        cv2.imwrite(str(tpl_path), tpl)

        frame = np.full((SCREEN_H, SCREEN_W, 3), 90, dtype=np.uint8)
        frame[py:py + th, px:px + tw] = tpl  # composite at a known spot, scale 1.0

        guard = {
            "id": "tpl-guard", "name": "Reconnect", "method": "template",
            "template_path": str(tpl_path), "threshold": 0.6,
            "region": [0.0, 0.0, 100.0, 100.0],
        }
        dets = s._detect_on_frame(guard, frame)

    assert dets, "expected the template guard to fire"
    best = dets[0]
    gt_cx, gt_cy = px + tw // 2, py + th // 2
    err = ((best.x - gt_cx) ** 2 + (best.y - gt_cy) ** 2) ** 0.5
    assert err <= 12, f"localization off by {err:.1f}px (center=({best.x},{best.y}))"


def test_color_guard_fires_on_supplied_frame() -> None:
    s = _server()
    # A saturated-red blob on gray; the color branch should find its center.
    frame = np.full((SCREEN_H, SCREEN_W, 3), 90, dtype=np.uint8)
    frame[300:360, 500:600] = (0, 0, 255)  # BGR red

    guard = {
        "id": "color-guard", "name": "RedBlob", "method": "color",
        "hsv_low": [0, 120, 120], "hsv_high": [10, 255, 255],
        "region": [0.0, 0.0, 100.0, 100.0], "min_area": 40,
    }
    dets = s._detect_on_frame(guard, frame)
    assert dets, "expected the color guard to fire on the red blob"
    best = dets[0]
    assert 500 <= best.x <= 600 and 300 <= best.y <= 360


# ── 3. guard_test: the editor's dry-run (match + preview image) ──────────────
def test_guard_test_matches_and_returns_a_preview() -> None:
    s = _server()
    frame = np.full((SCREEN_H, SCREEN_W, 3), 90, dtype=np.uint8)
    frame[300:360, 500:600] = (0, 0, 255)  # BGR red blob

    guard = {
        "id": "color-guard", "name": "RedBlob", "method": "color",
        "hsv_low": [0, 120, 120], "hsv_high": [10, 255, 255],
        "region": [0.0, 0.0, 100.0, 100.0], "min_area": 40,
    }
    res = s._guard_test_on_frame(guard, frame)

    assert res["ok"] is True
    assert res["matched"] >= 1
    assert 500 <= res["found_x"] <= 600 and 300 <= res["found_y"] <= 360
    assert res["confidence"] > 0
    assert "match(es)" in res["message"]
    # The preview is a valid JPEG of the frame, base64-encoded.
    img = cv2.imdecode(np.frombuffer(base64.b64decode(res["preview"]), np.uint8), cv2.IMREAD_COLOR)
    assert img.shape == (SCREEN_H, SCREEN_W, 3)


def test_guard_test_no_match_reports_cleanly() -> None:
    s = _server()
    frame = np.full((SCREEN_H, SCREEN_W, 3), 90, dtype=np.uint8)  # flat gray: no red

    guard = {
        "id": "color-guard", "name": "RedBlob", "method": "color",
        "hsv_low": [0, 120, 120], "hsv_high": [10, 255, 255],
        "region": [0.0, 0.0, 100.0, 100.0], "min_area": 40,
    }
    res = s._guard_test_on_frame(guard, frame)

    assert res["ok"] is False
    assert res["matched"] == 0
    assert res["found_x"] == -1 and res["found_y"] == -1
    assert res["confidence"] == 0.0
    assert res["message"] == "no match"
    assert res["preview"], "a no-match still returns the region-box preview"


# ── 3. editor setup pickers ──────────────────────────────────────────────────
# The interactive Win32 overlays (sample_color / pick_region_rect / the region
# picker's live pick) cannot run headless, so each is stubbed; what these prove
# is the dispatch + result-assembly the sidecar adds around them — cancellation
# shapes, the px result wrapping, and the real crop/save/thumbnail math.
def test_guard_pick_color_wraps_the_sample() -> None:
    s = VisionServer()
    sample = {"x": 100, "y": 200, "hsv": [10, 200, 200],
              "hsv_low": [5, 120, 120], "hsv_high": [15, 255, 255], "hex": "#c8461e"}
    with patch("clawmation_vision.color_sampler.sample_color", return_value=sample):
        r = s.handle({"id": 1, "method": "guard_pick_color", "params": {}})
    assert r["ok"] is True
    assert r["result"] == {"ok": True, **sample}

    with patch("clawmation_vision.color_sampler.sample_color", return_value=None):
        r = s.handle({"id": 2, "method": "guard_pick_color", "params": {}})
    assert r["result"] == {"ok": False, "error": "cancelled"}


def test_guard_pick_region_wraps_the_rect() -> None:
    s = VisionServer()
    with patch("clawmation_vision.region_picker.pick_region_rect", return_value=(10, 20, 30, 40)):
        r = s.handle({"id": 1, "method": "guard_pick_region", "params": {}})
    assert r["result"] == {"ok": True, "x": 10, "y": 20, "w": 30, "h": 40}

    with patch("clawmation_vision.region_picker.pick_region_rect", return_value=None):
        r = s.handle({"id": 2, "method": "guard_pick_region", "params": {}})
    assert r["result"] == {"ok": False, "error": "cancelled"}


class _StubCapture:
    """Stands in for ScreenCapture: hands back one fixed frame, never touches a screen."""
    def __init__(self, frame: np.ndarray) -> None:
        self._frame = frame

    def __call__(self, *args, **kwargs) -> "_StubCapture":
        return self  # patched in as the class, so calling it yields the instance

    def grab(self) -> np.ndarray:
        return self._frame

    def close(self) -> None:
        pass


class _StubPicker:
    """Stands in for NativeRegionPicker: returns a fixed (x, y, w, h) rectangle."""
    def __init__(self, rect) -> None:
        self._rect = rect

    def __call__(self, *args, **kwargs) -> "_StubPicker":
        return self

    def pick(self):
        return self._rect


def test_capture_template_crops_saves_and_thumbs() -> None:
    s = VisionServer()
    frame = np.full((200, 200, 3), 127, np.uint8)
    with tempfile.TemporaryDirectory() as td:
        with patch("clawmation_vision.server.ScreenCapture", _StubCapture(frame)), \
             patch("clawmation_vision.region_picker.NativeRegionPicker", _StubPicker((40, 50, 60, 70))):
            r = s.handle({"id": 1, "method": "capture_template", "params": {"templates_dir": td}})
        res = r["result"]
        assert res["ok"] is True
        assert res["w"] == 60 and res["h"] == 70
        assert Path(res["path"]).exists()
        assert Path(res["path"]).parent == Path(td)
        assert res["thumb"], "a base64 PNG thumbnail is returned"
        saved = cv2.imread(res["path"])
        assert saved.shape[:2] == (70, 60), "the saved crop is the requested size"

    # A too-small selection is rejected with the desktop app's exact message,
    # before any file is written.
    with patch("clawmation_vision.server.ScreenCapture", _StubCapture(frame)), \
         patch("clawmation_vision.region_picker.NativeRegionPicker", _StubPicker((0, 0, 4, 4))):
        r = s.handle({"id": 2, "method": "capture_template", "params": {"templates_dir": "."}})
    assert r["result"] == {"ok": False, "error": "Selection too small"}

    # A cancelled drag returns the cancellation shape, not an error envelope.
    with patch("clawmation_vision.server.ScreenCapture", _StubCapture(frame)), \
         patch("clawmation_vision.region_picker.NativeRegionPicker", _StubPicker(None)):
        r = s.handle({"id": 3, "method": "capture_template", "params": {"templates_dir": "."}})
    assert r["result"] == {"ok": False, "error": "cancelled"}


def test_add_template_image_copies_and_thumbs() -> None:
    s = VisionServer()

    class _FakeTk:
        def withdraw(self) -> None: pass
        def attributes(self, *a, **k) -> None: pass
        def destroy(self) -> None: pass

    with tempfile.TemporaryDirectory() as td:
        src = Path(td) / "button.png"
        cv2.imwrite(str(src), np.full((30, 40, 3), 200, np.uint8))
        dest = Path(td) / "templates"
        with patch("tkinter.Tk", _FakeTk), \
             patch("tkinter.filedialog.askopenfilename", return_value=str(src)):
            r = s.handle({"id": 1, "method": "add_template_image", "params": {"templates_dir": str(dest)}})
        res = r["result"]
        assert res["ok"] is True
        assert res["w"] == 40 and res["h"] == 30
        assert Path(res["path"]).exists()
        assert Path(res["path"]).parent == dest
        assert res["thumb"]

    with patch("tkinter.Tk", _FakeTk), \
         patch("tkinter.filedialog.askopenfilename", return_value=""):
        r = s.handle({"id": 2, "method": "add_template_image", "params": {"templates_dir": "."}})
    assert r["result"] == {"ok": False, "error": "cancelled"}


def test_surgical_capture_saves_crop_strokes_and_thumb() -> None:
    # SurgicalPicker.pick() returns (crop_x, crop_y, crop_w, crop_h, strokes);
    # the reused _StubPicker hands back whatever tuple we seed it with. What this
    # proves is the crop/save/thumbnail math and the click-field assembly the
    # sidecar wraps around the overlay — the overlay itself can't run headless.
    s = VisionServer()
    frame = np.full((200, 200, 3), 127, np.uint8)
    with tempfile.TemporaryDirectory() as td:
        with patch("clawmation_vision.capture.ScreenCapture", _StubCapture(frame)), \
             patch("clawmation_vision.surgical_picker.SurgicalPicker",
                   _StubPicker((10, 20, 40, 30, [[2, 3, 12, 15]]))):
            r = s.handle({"id": 1, "method": "surgical_capture", "params": {"templates_dir": td}})
        res = r["result"]
        assert res["ok"] is True
        assert res["w"] == 40 and res["h"] == 30
        assert Path(res["path"]).exists()
        assert Path(res["path"]).parent == Path(td)
        assert res["click_lines"] == [[2, 3, 12, 15]]
        assert res["click_line"] == [2, 3, 12, 15]  # first stroke, single-target field
        assert res["offset"] == [2, 3]              # first stroke's start point
        assert res["thumb"], "a base64 PNG thumbnail with the strokes drawn is returned"
        saved = cv2.imread(res["path"])
        assert saved.shape[:2] == (30, 40), "the saved crop is the requested size"

    # A cancelled overlay (Esc) returns the cancellation shape, not an error envelope.
    with patch("clawmation_vision.capture.ScreenCapture", _StubCapture(frame)), \
         patch("clawmation_vision.surgical_picker.SurgicalPicker", _StubPicker(None)):
        r = s.handle({"id": 2, "method": "surgical_capture", "params": {"templates_dir": "."}})
    assert r["result"] == {"ok": False, "error": "cancelled"}


# ── 4. checkpoint detection poll ─────────────────────────────────────────────
# `detect_checkpoint` is the per-poll detect half of `MacroPlayer._run_checkpoint`
# — the orchestration (wait/timeout/click) stays in the Rust play loop. It reuses
# the guard seam but with the checkpoint defaults (label "checkpoint", min_area
# 40, template threshold 0.75). The frame source is stubbed so no screen is hit.
def test_detect_checkpoint_routes_color_and_template() -> None:
    s = _server()

    # Color branch, with `method` defaulted (the cfg omits it → "color").
    frame = np.full((SCREEN_H, SCREEN_W, 3), 90, dtype=np.uint8)
    frame[300:360, 500:600] = (0, 0, 255)  # BGR red blob
    s._capture = _StubCapture(frame)
    dets = s._detect_checkpoint({
        "hsv_low": [0, 120, 120], "hsv_high": [10, 255, 255],
        "region": [0, 0, 100, 100],
    })
    assert dets, "checkpoint color poll should find the red blob"
    assert dets[0]["label"] == "checkpoint"
    assert 500 <= dets[0]["x"] <= 600 and 300 <= dets[0]["y"] <= 360

    # Template branch: a seeded-noise patch composited at a known spot, referenced
    # by file path (the checkpoint stores templates by path → ensure_template_path).
    tw, th, px, py = 96, 64, 400, 250
    tpl = _noise_template(tw, th, seed=11)
    with tempfile.TemporaryDirectory() as d:
        tpl_path = Path(d) / "cp_tpl.png"
        cv2.imwrite(str(tpl_path), tpl)
        tframe = np.full((SCREEN_H, SCREEN_W, 3), 90, dtype=np.uint8)
        tframe[py:py + th, px:px + tw] = tpl
        s._capture = _StubCapture(tframe)
        dets = s._detect_checkpoint({
            "method": "template", "template": str(tpl_path), "threshold": 0.6,
            "region": [0, 0, 100, 100],
        })
    assert dets, "checkpoint template poll should localize the patch"
    gt_cx, gt_cy = px + tw // 2, py + th // 2
    err = ((dets[0]["x"] - gt_cx) ** 2 + (dets[0]["y"] - gt_cy) ** 2) ** 0.5
    assert err <= 12, f"localization off by {err:.1f}px"


def test_detect_checkpoint_swallows_bad_config_to_empty() -> None:
    """A malformed config (missing hsv keys) yields no matches, not a raise — the
    in-process closure caught its own exceptions to [], and the handler mirrors it."""
    s = _server()
    s._capture = _StubCapture(np.full((SCREEN_H, SCREEN_W, 3), 90, dtype=np.uint8))
    assert s._detect_checkpoint({"region": [0, 0, 100, 100]}) == []


# ── 5. AI steps: per-step detect + the step editor's dry-run ─────────────────
# `ai_detect` is the run loop's per-step probe (Rust polls it for find_click /
# wait_for); `ai_test_step` is the editor's Test button (result + preview image).
# Both reuse the same verbatim seam as the desktop `AIExecutor`. Only the `color`
# detect mode actually matches — step templates are never loaded into the cache —
# so these exercise color, the monolith's one working detection path.
def test_ai_detect_color_step_finds_blob_and_messages() -> None:
    s = _server()
    frame = np.full((SCREEN_H, SCREEN_W, 3), 90, dtype=np.uint8)
    frame[300:360, 500:600] = (0, 0, 255)  # BGR red blob
    s._capture = _StubCapture(frame)

    res = s._ai_detect({"step": {
        "id": "s1", "type": "find_click", "detect_mode": "color",
        "hsv_low": [0, 120, 120], "hsv_high": [10, 255, 255],
        "region": [0.0, 0.0, 100.0, 100.0], "min_area": 40,
    }})
    assert res["matches"], "the color step should localize the red blob"
    m = res["matches"][0]
    assert 500 <= m["x"] <= 600 and 300 <= m["y"] <= 360
    assert m["confidence"] > 0
    assert res["message"].endswith("color match(es)")


def test_ai_test_step_color_finds_and_previews() -> None:
    s = _server()
    frame = np.full((SCREEN_H, SCREEN_W, 3), 90, dtype=np.uint8)
    frame[300:360, 500:600] = (0, 0, 255)  # BGR red blob

    step = Step.from_dict({
        "id": "s1", "type": "find_click", "detect_mode": "color",
        "hsv_low": [0, 120, 120], "hsv_high": [10, 255, 255],
        "region": [0.0, 0.0, 100.0, 100.0], "min_area": 40,
    })
    res = s._ai_test_step_on_frame(step, frame)

    assert res["ok"] is True
    assert res["matched"] >= 1
    assert 500 <= res["found_x"] <= 600 and 300 <= res["found_y"] <= 360
    assert res["confidence"] > 0
    assert res["message"].startswith("would click")
    assert " — " in res["message"]  # em dash separator, verbatim from test_step
    assert "color match(es)" in res["message"]
    # The preview is a valid JPEG of the frame, base64-encoded.
    img = cv2.imdecode(np.frombuffer(base64.b64decode(res["preview"]), np.uint8), cv2.IMREAD_COLOR)
    assert img.shape == (SCREEN_H, SCREEN_W, 3)


def test_ai_test_step_no_match_reports_cleanly() -> None:
    s = _server()
    frame = np.full((SCREEN_H, SCREEN_W, 3), 90, dtype=np.uint8)  # flat gray: no red

    step = Step.from_dict({
        "id": "s1", "type": "find_click", "detect_mode": "color",
        "hsv_low": [0, 120, 120], "hsv_high": [10, 255, 255],
        "region": [0.0, 0.0, 100.0, 100.0], "min_area": 40,
    })
    res = s._ai_test_step_on_frame(step, frame)

    assert res["ok"] is False
    assert res["matched"] == 0
    assert res["found_x"] == -1 and res["found_y"] == -1
    assert res["confidence"] == 0.0
    # Byte-exact message, em dash included (U+2014, not a hyphen).
    assert res["message"] == "nothing found — 0 color match(es)"
    assert res["preview"], "a no-match still returns the region-box preview"


def test_ai_test_step_bad_step_is_reported_not_raised() -> None:
    s = _server()
    # A step dict missing the required `id` fails Step.from_dict before any grab,
    # so the editor gets "Bad step: …" (parse-first order, unlike guard_test).
    res = s._ai_test_step({"step": {"type": "click"}})
    assert res["ok"] is False
    assert res["error"].startswith("Bad step:")


def test_ai_test_step_unknown_type_message_omits_the_name() -> None:
    # The TEST path reports a bare "unknown step type" (no name), unlike the RUN
    # path's "unknown step type '<t>'" — a source asymmetry, preserved verbatim.
    s = _server()
    frame = np.full((SCREEN_H, SCREEN_W, 3), 90, dtype=np.uint8)
    step = Step.from_dict({"id": "s1", "type": "bogus"})
    res = s._ai_test_step_on_frame(step, frame)
    assert res["ok"] is False
    assert res["message"] == "unknown step type"


# ── plain-script runner (no pytest needed) ───────────────────────────────────
def _main() -> int:
    tests = [v for k, v in sorted(globals().items()) if k.startswith("test_")]
    failed = 0
    for t in tests:
        try:
            t()
            print(f"PASS {t.__name__}")
        except Exception as e:  # noqa: BLE001
            failed += 1
            print(f"FAIL {t.__name__}: {type(e).__name__}: {e}")
    print(f"\n{len(tests) - failed}/{len(tests)} passed")
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(_main())
