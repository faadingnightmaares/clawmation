"""Clawmation vision sidecar — cv2/easyocr detection compute, driven over stdio
JSON-RPC by the Rust/Tauri backend. `detection.py` and `capture.py` are the
original app's vision modules copied verbatim; `guard.py` is the `detect_guard`
seam; `server.py` is the RPC shell.
"""

__version__ = "0.1.0"
