# Clawmation vision sidecar

The Clawmation backend is Rust + Tauri. This sidecar is the one piece that stays
Python: the computer-vision compute — OpenCV template/colour matching and
easyocr text detection. The Rust backend owns everything else (state, macros,
input, recorder, player, guard/vision orchestration, the ~90 Tauri commands the
UI calls) and drives this process over a small JSON-RPC protocol.

Keeping detection in Python is a deliberate fidelity choice: `detection.py` and
`capture.py` are the original app's modules **copied verbatim** (byte-for-byte),
so matching behaviour is identical rather than merely "equivalent" — no
re-implementation of CLAHE, masked `TM_CCOEFF_NORMED`, ORB/FLANN, or easyocr in
another language.

## Layout

```
clawmation_vision/
  detection.py   # verbatim — PixelDetector: HSV colour, template match, OCR
  capture.py     # verbatim — ScreenCapture: dxcam / mss backends
  config.py      # the HSVRange / Region / DEFAULT_RESOLUTION the two import,
                 # decoupled from the original module's path globals
  guard.py       # verbatim Guard + detect_guard — the detection seam
  server.py      # the JSON-RPC-over-stdio shell (this file is the new code)
tests/
  test_protocol.py
```

The process holds **one** long-lived `PixelDetector` and **one**
`ScreenCapture` for its whole lifetime — the detector is stateful (per-template
cache, temporal-coherence `_last_match`, lazy easyocr model load), exactly as the
original app shared a single detector + capture across its GuardEngine,
VisionAgent, and guard-test paths.

## Protocol

Newline-delimited JSON on stdin/stdout, one object per line. Request:

```json
{"id": 1, "method": "detect_guards", "params": {"guards": [ ... ]}}
```

Response (exactly one per request, same `id`):

```json
{"id": 1, "ok": true,  "result": { "<guard-id>": [ {"label": "...", "x": 812, "y": 440, "w": 96, "h": 64, "confidence": 0.98, "roi_offset": [0, 0]} ] }}
{"id": 1, "ok": false, "error": "RuntimeError: capture returned no frame"}
```

stdout carries only protocol JSON; all logging goes to stderr.

Every detection method **grabs its own frame** from the capture — a frame never
crosses the process boundary, matching the original where the frame provider and
the detector lived in one process (the ~11 MB array was never serialized).

### Methods

| method          | params                                                        | result |
|-----------------|---------------------------------------------------------------|--------|
| `ping`          | –                                                             | `{pong, ready}` |
| `init`          | `screen_w`, `screen_h`, `capture_backend`                     | echoes the config |
| `load_template` | `name`, `path`                                                | `{loaded}` |
| `detect_guards` | `guards` (list of guard dicts)                                | `{guard_id: [detection]}` — one frame for the whole batch |
| `detect_guard`  | `guard` (one guard dict)                                      | `[detection]` |
| `detect_color`  | `hsv_low`, `hsv_high`, `region?`, `min_area?`, `label?`       | `[detection]` |
| `color_present` | `hsv_low`, `hsv_high`, `region`, `min_pixel_ratio?`           | `{present}` |
| `ocr_find`      | `text`, `region?`                                             | `[detection]` |
| `capture_fps`   | –                                                             | `{fps}` |
| `shutdown`      | –                                                             | `{bye}` (ends the loop) |

`detect_guards` takes the whole active-guard list and runs them against a single
grabbed frame, preserving the poll loop's one-frame-per-cycle temporal
consistency (every guard in a cycle sees the same frame). Guard JSON matches the
Rust `Guard` model field-for-field.

## Running

Dev, from this directory:

```sh
uv pip install -e .            # core: template + colour guards, mss capture
uv pip install -e '.[capture]' # + dxcam (fast DXGI primary backend)
uv pip install -e '.[ocr]'     # + easyocr (method="ocr" guards; pulls torch)

python -m clawmation_vision.server   # then feed JSON-RPC lines on stdin
```

In production the Rust backend spawns this via Tauri's shell plugin (a
PyInstaller-frozen executable so `git clone && cargo build` plus the bundled
sidecar needs no system Python).

## Tests

```sh
python tests/test_protocol.py   # plain script, no test runner required
# or: pytest
```

Proves the RPC shell (readiness, unknown-method / malformed-line errors, id
echo, shutdown) and that a real guard localizes through the verbatim
`detect_guard` → `match_robust` path on a supplied frame. The detection test uses
a seeded-noise template, so it is deterministic and needs no external assets and
no easyocr.
