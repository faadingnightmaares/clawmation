"""Screen capture — dxcam (primary), WGC (windows-capture), mss (fallback).

dxcam uses Desktop Duplication API for ~120+ FPS captures on Windows.
WGC (windows-capture) uses Windows.Graphics.Capture for per-window capture.
mss is the cross-platform GDI fallback at ~30-60 FPS.

Key fix: dxcam MUST be started with video_mode=True and frames read via
get_latest_frame(). The old code only called start() when a region was set,
leaving full-screen capture in one-shot mode where grab() returns None
whenever the screen is static. That was the #1 reliability problem.
"""
from __future__ import annotations

import logging
import threading
import time
from typing import Optional

import numpy as np

from .config import DEFAULT_RESOLUTION

logger = logging.getLogger(__name__)


class ScreenCapture:
    """Unified screen capture with backend auto-selection.

    Thread-safe: dxcam and WGC run capture on dedicated threads.
    Callers use grab() which reads the latest frame from a ring buffer.
    """

    def __init__(self, backend: str = "dxcam", region: tuple[int, int, int, int] | None = None):
        self._backend_name = backend
        self._region = region  # (x1, y1, x2, y2) pixels or None for full screen
        self._impl: Optional[object] = None
        self._last_frame: Optional[np.ndarray] = None
        self._frame_count = 0
        self._fps_window: list[float] = []
        self._lock = threading.Lock()
        self._init_backend()

    def _init_backend(self) -> None:
        # WGC (windows-capture) segfaults on some machines — a segfault kills
        # the entire process and CANNOT be caught by try/except. So WGC is
        # NEVER used in the auto-fallback chain. Only dxcam → mss is safe.
        # If the user explicitly picks WGC in Settings, we warn and use mss.
        if self._backend_name == "wgc":
            logger.warning(
                "WGC backend is unstable (segfaults on this machine). "
                "Falling back to mss. Use DXGI for best performance."
            )
            self._backend_name = "mss"

        if self._backend_name == "dxcam":
            if self._init_dxcam():
                return
            logger.warning("dxcam failed, falling back to mss")
            self._backend_name = "mss"

        # mss fallback — always works, just slower
        self._init_mss()

    def _init_dxcam(self) -> bool:
        try:
            import dxcam
            # Set region at CREATE time to avoid per-grab staging surface rebuilds.
            # BGR output is OpenCV-ready with no conversion.
            self._impl = dxcam.create(
                output_color="BGR",
                region=self._region,
                max_buffer_len=4,
            )
            # ALWAYS start threaded capture with video_mode=True:
            # - Runs capture on a dedicated thread (thread-safe reads)
            # - video_mode reuses last frame when screen is static → NEVER returns None
            # - target_fps=60 matches typical macro detection rate
            # This was the critical bug: start() was only called when region was set,
            # leaving full-screen capture in one-shot mode (grab() returns None).
            self._impl.start(
                target_fps=60,
                video_mode=True,
            )
            logger.info("dxcam backend started (region=%s, video_mode=True, 60fps)", self._region)
            return True
        except Exception as e:
            logger.warning("dxcam init failed: %s", e)
            return False

    def _init_wgc(self) -> bool:
        """Windows Graphics Capture via the windows-capture Rust library."""
        try:
            from windows_capture import WindowsCapture, Frame, InternalCaptureControl

            kwargs = {"cursor_capture": False, "draw_border": False}
            # monitor_index is 1-based (1 = primary). Default to primary monitor;
            # window-specific capture (window_hwnd) can be added later.
            kwargs["monitor_index"] = 1

            capture = WindowsCapture(**kwargs)
            self._wgc_lock = threading.Lock()
            self._wgc_frame: Optional[np.ndarray] = None

            @capture.event
            def on_frame_arrived(frame: Frame, capture_control: InternalCaptureControl):
                # BGRA → BGR (drop alpha)
                bgr = frame.frame_buffer[:, :, :3].copy()
                if self._region:
                    x1, y1, x2, y2 = self._region
                    h, w = bgr.shape[:2]
                    x1, y1 = max(0, x1), max(0, y1)
                    x2, y2 = min(w, x2), min(h, y2)
                    if x2 > x1 and y2 > y1:
                        bgr = bgr[y1:y2, x1:x2]
                with self._wgc_lock:
                    self._wgc_frame = bgr
                    self._frame_count += 1

            @capture.event
            def on_closed():
                logger.info("WGC capture session closed")

            self._wgc_control = capture.start_free_threaded()
            self._impl = capture
            logger.info("WGC backend started (monitor=0)")
            return True
        except ImportError:
            logger.info("windows-capture not installed (pip install windows-capture)")
            return False
        except Exception as e:
            logger.warning("WGC init failed: %s", e)
            return False

    def _init_mss(self) -> None:
        import mss
        self._impl = mss.mss()
        logger.info("mss backend initialized (GDI fallback)")

    @property
    def backend(self) -> str:
        return self._backend_name

    def grab(self) -> np.ndarray | None:
        """Capture a single frame as BGR numpy array. Returns None on failure."""
        t0 = time.perf_counter()

        if self._backend_name == "dxcam":
            # Use get_latest_frame() — blocks until a frame is available.
            # With video_mode=True this NEVER returns None (reuses last frame).
            try:
                frame = self._impl.get_latest_frame()
            except Exception:
                # Fallback to grab() if get_latest_frame isn't available
                frame = self._impl.grab(region=self._region)
            if frame is None:
                # Last resort: return cached frame
                return self._last_frame
            self._last_frame = frame
            self._track_fps(t0)
            return frame

        if self._backend_name == "wgc":
            with self._wgc_lock:
                frame = self._wgc_frame
            if frame is not None:
                self._last_frame = frame
                self._track_fps(t0)
                return frame.copy()
            return self._last_frame

        # mss path
        import mss
        if self._region:
            x1, y1, x2, y2 = self._region
            monitor = {"left": x1, "top": y1, "width": x2 - x1, "height": y2 - y1}
        else:
            monitor = self._impl.monitors[1]  # primary monitor

        shot = self._impl.grab(monitor)
        frame = np.array(shot)[:, :, :3]  # drop alpha, BGRA→BGR
        self._last_frame = frame
        self._track_fps(t0)
        return frame

    def grab_rgb(self) -> np.ndarray | None:
        """Capture as RGB (for OCR / non-OpenCV consumers)."""
        frame = self.grab()
        if frame is not None:
            return frame[:, :, ::-1].copy()
        return None

    def _track_fps(self, t0: float) -> None:
        dt = time.perf_counter() - t0
        if dt > 0:
            self._fps_window.append(1.0 / dt)
            if len(self._fps_window) > 60:
                self._fps_window.pop(0)

    @property
    def fps(self) -> float:
        if not self._fps_window:
            return 0.0
        return sum(self._fps_window) / len(self._fps_window)

    def close(self) -> None:
        if self._backend_name == "dxcam" and self._impl:
            try:
                self._impl.stop()
                self._impl.release()
            except Exception:
                pass
        elif self._backend_name == "wgc":
            try:
                self._wgc_control.stop()
            except Exception:
                pass
        elif hasattr(self._impl, "close"):
            self._impl.close()
        logger.info("Capture closed (backend=%s)", self._backend_name)

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.close()
