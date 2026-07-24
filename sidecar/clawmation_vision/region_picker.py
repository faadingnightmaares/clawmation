"""Lightshot-style region capture — native Win32 overlay (single app, no taskbar).

Flow:
  1. Grab a frozen full-screen frame.
  2. Show it in a fullscreen, frameless, always-on-top, tool-window overlay.
     Everything is drawn darkened; as the user drags, the selected region is
     drawn at full brightness with a cyan outline (the classic Lightshot look).
  3. On mouse release the selected region is returned to the caller (which
     crops the frozen frame); the overlay closes. Esc cancels.

The overlay is transient and lives in the same process as the main window —
it is NOT a second app and never shows a taskbar button (WS_EX_TOOLWINDOW).
"""

from __future__ import annotations

import ctypes
import logging
import threading
from ctypes import wintypes

import numpy as np

logger = logging.getLogger(__name__)

user32 = ctypes.windll.user32
gdi32 = ctypes.windll.gdi32
kernel32 = ctypes.windll.kernel32

# ── Win32 structs / prototypes (self-contained) ──────────────────────────────
WNDPROC = ctypes.WINFUNCTYPE(
    ctypes.c_long, wintypes.HWND, ctypes.c_uint, wintypes.WPARAM, wintypes.LPARAM
)


class WNDCLASSW(ctypes.Structure):
    _fields_ = [
        ("style", ctypes.c_uint),
        ("lpfnWndProc", WNDPROC),
        ("cbClsExtra", ctypes.c_int),
        ("cbWndExtra", ctypes.c_int),
        ("hInstance", wintypes.HINSTANCE),
        ("hIcon", wintypes.HICON),
        ("hCursor", wintypes.HANDLE),
        ("hbrBackground", wintypes.HANDLE),
        ("lpszMenuName", wintypes.LPCWSTR),
        ("lpszClassName", wintypes.LPCWSTR),
    ]


class PAINTSTRUCT(ctypes.Structure):
    _fields_ = [
        ("hdc", wintypes.HDC),
        ("fErase", wintypes.BOOL),
        ("rcPaint", wintypes.RECT),
        ("fRestore", wintypes.BOOL),
        ("fIncUpdate", wintypes.BOOL),
        ("rgbReserved", ctypes.c_byte * 32),
    ]


class BITMAPINFOHEADER(ctypes.Structure):
    _fields_ = [
        ("biSize", ctypes.c_uint),
        ("biWidth", ctypes.c_long),
        ("biHeight", ctypes.c_long),
        ("biPlanes", ctypes.c_ushort),
        ("biBitCount", ctypes.c_ushort),
        ("biCompression", ctypes.c_uint),
        ("biSizeImage", ctypes.c_uint),
        ("biXPelsPerMeter", ctypes.c_long),
        ("biYPelsPerMeter", ctypes.c_long),
        ("biClrUsed", ctypes.c_uint),
        ("biClrImportant", ctypes.c_uint),
    ]


class BITMAPINFO(ctypes.Structure):
    _fields_ = [
        ("bmiHeader", BITMAPINFOHEADER),
        ("bmiColors", ctypes.c_uint * 3),
    ]


LRESULT = ctypes.c_ssize_t

user32.DefWindowProcW.argtypes = [wintypes.HWND, ctypes.c_uint, wintypes.WPARAM, wintypes.LPARAM]
user32.DefWindowProcW.restype = LRESULT
user32.CreateWindowExW.argtypes = [
    ctypes.c_uint, wintypes.LPCWSTR, wintypes.LPCWSTR, ctypes.c_uint,
    ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_int,
    wintypes.HWND, wintypes.HMENU, wintypes.HINSTANCE, wintypes.LPVOID,
]
user32.CreateWindowExW.restype = wintypes.HWND
user32.RegisterClassW.argtypes = [ctypes.POINTER(WNDCLASSW)]
user32.RegisterClassW.restype = ctypes.c_ushort
user32.GetMessageW.argtypes = [ctypes.POINTER(wintypes.MSG), wintypes.HWND, ctypes.c_uint, ctypes.c_uint]
user32.GetMessageW.restype = wintypes.BOOL
user32.PostMessageW.argtypes = [wintypes.HWND, ctypes.c_uint, wintypes.WPARAM, wintypes.LPARAM]
user32.PostMessageW.restype = wintypes.BOOL
user32.BeginPaint.argtypes = [wintypes.HWND, ctypes.POINTER(PAINTSTRUCT)]
user32.BeginPaint.restype = wintypes.HDC
user32.EndPaint.argtypes = [wintypes.HWND, ctypes.POINTER(PAINTSTRUCT)]
user32.EndPaint.restype = wintypes.BOOL
user32.InvalidateRect.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.RECT), wintypes.BOOL]
user32.InvalidateRect.restype = wintypes.BOOL
user32.LoadCursorW.argtypes = [wintypes.HINSTANCE, wintypes.LPCWSTR]
user32.LoadCursorW.restype = wintypes.HANDLE
user32.ShowWindow.argtypes = [wintypes.HWND, ctypes.c_int]
user32.ShowWindow.restype = wintypes.BOOL
user32.UpdateWindow.argtypes = [wintypes.HWND]
user32.UpdateWindow.restype = wintypes.BOOL
user32.GetSystemMetrics.argtypes = [ctypes.c_int]
user32.GetSystemMetrics.restype = ctypes.c_int
user32.SetForegroundWindow.argtypes = [wintypes.HWND]
user32.SetForegroundWindow.restype = wintypes.BOOL
user32.DestroyWindow.argtypes = [wintypes.HWND]
user32.DestroyWindow.restype = wintypes.BOOL
user32.PostQuitMessage.argtypes = [ctypes.c_int]
user32.PostQuitMessage.restype = None

gdi32.SelectObject.argtypes = [wintypes.HDC, wintypes.HANDLE]
gdi32.SelectObject.restype = wintypes.HANDLE
gdi32.DeleteObject.argtypes = [wintypes.HANDLE]
gdi32.DeleteObject.restype = wintypes.BOOL
gdi32.CreatePen.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_uint]
gdi32.CreatePen.restype = wintypes.HPEN
gdi32.Rectangle.argtypes = [wintypes.HDC, ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_int]
gdi32.Rectangle.restype = wintypes.BOOL
gdi32.GetStockObject.argtypes = [ctypes.c_int]
gdi32.GetStockObject.restype = wintypes.HANDLE
gdi32.StretchDIBits.argtypes = [
    wintypes.HDC,
    ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_int,
    ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_int,
    ctypes.c_void_p, ctypes.POINTER(BITMAPINFO), ctypes.c_uint, ctypes.c_uint,
]
gdi32.StretchDIBits.restype = ctypes.c_int

kernel32.GetModuleHandleW.argtypes = [wintypes.LPCWSTR]
kernel32.GetModuleHandleW.restype = wintypes.HINSTANCE

# ── Constants ────────────────────────────────────────────────────────────────
WS_POPUP = 0x80000000
WS_VISIBLE = 0x10000000
WS_EX_TOPMOST = 0x00000008
WS_EX_TOOLWINDOW = 0x00000080
CS_VREDRAW = 0x0001
CS_HREDRAW = 0x0002
WM_DESTROY = 0x0002
WM_PAINT = 0x000F
WM_ERASEBKGND = 0x0014
WM_LBUTTONDOWN = 0x0201
WM_LBUTTONUP = 0x0202
WM_MOUSEMOVE = 0x0200
WM_KEYDOWN = 0x0100
VK_ESCAPE = 0x1B
IDC_CROSS = 32515
DIB_RGB_COLORS = 0
BI_RGB = 0
SRCCOPY = 0x00CC0020
NULL_BRUSH = 5

WNDCLASS_NAME = "ClawmationRegionPicker"
DARKEN = 0.42  # brightness multiplier for the non-selected area


def _rgb(bgr: tuple[int, int, int]) -> int:
    b, g, r = bgr
    return (b << 16) | (g << 8) | r


def _to_bgra_bottomup(img: np.ndarray) -> np.ndarray:
    """Convert a BGR image to contiguous bottom-up 32-bit BGRA for StretchDIBits."""
    h, w = img.shape[:2]
    bgra = np.empty((h, w, 4), dtype=np.uint8)
    bgra[:, :, :3] = img
    bgra[:, :, 3] = 255
    return np.ascontiguousarray(np.flipud(bgra))


class NativeRegionPicker:
    """Fullscreen drag-to-select overlay drawn with GDI (no webview)."""

    def __init__(self, frame: np.ndarray):
        self.frame = frame
        self.h, self.w = frame.shape[:2]
        # Pre-compute the darkened full frame (the "outside selection" look)
        self.dark = (frame.astype(np.float32) * DARKEN).clip(0, 255).astype(np.uint8)
        self._dark_bits = _to_bgra_bottomup(self.dark)
        self._bright_bits = _to_bgra_bottomup(frame)

        self.hwnd = 0
        self._thread: threading.Thread | None = None
        self._ready = threading.Event()
        self._done = threading.Event()
        self.result: tuple[int, int, int, int] | None = None

        self._dragging = False
        self._sx = self._sy = 0  # drag start
        self._cx = self._cy = 0  # current cursor

        self._wndproc = WNDPROC(self._on_message)

    # ── Public ───────────────────────────────────────────────────────────────

    def pick(self, timeout: float = 120.0) -> tuple[int, int, int, int] | None:
        """Open the overlay, block until selection/cancel. Returns (x,y,w,h) or None."""
        self._thread = threading.Thread(target=self._run, daemon=True)
        self._thread.start()
        self._ready.wait(timeout=3.0)
        self._done.wait(timeout=timeout)
        return self.result

    # ── Window thread ────────────────────────────────────────────────────────

    def _run(self) -> None:
        try:
            self._create_window()
        except Exception as e:
            logger.warning("Region picker window failed: %s", e)
            self._ready.set()
            self._done.set()
            return
        self._ready.set()

        msg = wintypes.MSG()
        while True:
            ret = user32.GetMessageW(ctypes.byref(msg), None, 0, 0)
            if ret <= 0:
                break
            user32.TranslateMessage(ctypes.byref(msg))
            user32.DispatchMessageW(ctypes.byref(msg))

    def _create_window(self) -> None:
        hinstance = kernel32.GetModuleHandleW(None)

        wc = WNDCLASSW()
        wc.style = CS_HREDRAW | CS_VREDRAW
        wc.lpfnWndProc = self._wndproc
        wc.hInstance = hinstance
        wc.hCursor = user32.LoadCursorW(
            None, ctypes.cast(ctypes.c_void_p(IDC_CROSS), wintypes.LPCWSTR)
        )
        wc.hbrBackground = gdi32.GetStockObject(NULL_BRUSH)
        wc.lpszClassName = WNDCLASS_NAME
        user32.RegisterClassW(ctypes.byref(wc))

        sw = user32.GetSystemMetrics(0)  # SM_CXSCREEN
        sh = user32.GetSystemMetrics(1)  # SM_CYSCREEN

        self.hwnd = user32.CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            WNDCLASS_NAME,
            "Clawmation — Select Region",
            WS_POPUP | WS_VISIBLE,
            0, 0, sw, sh,
            None, None, hinstance, None,
        )
        user32.ShowWindow(self.hwnd, 8)  # SW_SHOWNA
        user32.UpdateWindow(self.hwnd)
        user32.SetForegroundWindow(self.hwnd)  # needed to receive Esc key

    # ── Message handler ──────────────────────────────────────────────────────

    def _on_message(self, hwnd, msg, wparam, lparam):
        if msg == WM_PAINT:
            self._paint(hwnd)
            return 0
        # Suppress background erase — prevents white flash between frames.
        if msg == WM_ERASEBKGND:
            return 1

        elif msg == WM_LBUTTONDOWN:
            self._sx = self._cx = lparam & 0xFFFF
            self._sy = self._cy = (lparam >> 16) & 0xFFFF
            self._dragging = True
            user32.InvalidateRect(hwnd, None, False)
            return 0

        elif msg == WM_MOUSEMOVE:
            if self._dragging:
                self._cx = lparam & 0xFFFF
                self._cy = (lparam >> 16) & 0xFFFF
                user32.InvalidateRect(hwnd, None, False)
            return 0

        elif msg == WM_LBUTTONUP:
            if self._dragging:
                self._dragging = False
                self._cx = lparam & 0xFFFF
                self._cy = (lparam >> 16) & 0xFFFF
                x, y, w, h = self._selection_rect()
                if w >= 8 and h >= 8:
                    self.result = (x, y, w, h)
                    self._finish(hwnd)
                else:
                    user32.InvalidateRect(hwnd, None, False)
            return 0

        elif msg == WM_KEYDOWN:
            if wparam == VK_ESCAPE:
                self.result = None
                self._finish(hwnd)
            return 0

        elif msg == WM_DESTROY:
            user32.PostQuitMessage(0)
            return 0

        return user32.DefWindowProcW(hwnd, msg, wparam, lparam)

    def _finish(self, hwnd) -> None:
        try:
            user32.DestroyWindow(hwnd)
        except Exception:
            pass
        self._done.set()

    def _selection_rect(self) -> tuple[int, int, int, int]:
        x = min(self._sx, self._cx)
        y = min(self._sy, self._cy)
        w = abs(self._cx - self._sx)
        h = abs(self._cy - self._sy)
        return x, y, w, h

    # ── Drawing ──────────────────────────────────────────────────────────────

    def _dib_info(self, w: int, h: int) -> BITMAPINFO:
        bmi = BITMAPINFO()
        bmi.bmiHeader.biSize = ctypes.sizeof(BITMAPINFOHEADER)
        bmi.bmiHeader.biWidth = w
        bmi.bmiHeader.biHeight = h  # positive = bottom-up (matches our flipped bits)
        bmi.bmiHeader.biPlanes = 1
        bmi.bmiHeader.biBitCount = 32
        bmi.bmiHeader.biCompression = BI_RGB
        return bmi

    def _paint(self, hwnd) -> None:
        ps = PAINTSTRUCT()
        hdc = user32.BeginPaint(hwnd, ctypes.byref(ps))

        # 1) Full darkened screenshot
        bmi = self._dib_info(self.w, self.h)
        gdi32.StretchDIBits(
            hdc, 0, 0, self.w, self.h,
            0, 0, self.w, self.h,
            self._dark_bits.ctypes.data, ctypes.byref(bmi), DIB_RGB_COLORS, SRCCOPY,
        )

        # 2) Bright original crop inside the selection
        x, y, w, h = self._selection_rect()
        if self._dragging and w > 0 and h > 0:
            crop = self.frame[y:y + h, x:x + w]
            bits = _to_bgra_bottomup(crop)
            bmi2 = self._dib_info(w, h)
            gdi32.StretchDIBits(
                hdc, x, y, w, h,
                0, 0, w, h,
                bits.ctypes.data, ctypes.byref(bmi2), DIB_RGB_COLORS, SRCCOPY,
            )

            # 3) Cyan selection outline
            pen = gdi32.CreatePen(0, 2, _rgb((192, 168, 74)))  # #4aa8c0 in BGR
            gdi32.SelectObject(hdc, pen)
            gdi32.SelectObject(hdc, gdi32.GetStockObject(NULL_BRUSH))
            gdi32.Rectangle(hdc, x, y, x + w, y + h)
            gdi32.DeleteObject(pen)

        user32.EndPaint(hwnd, ctypes.byref(ps))


def pick_region_rect() -> tuple[int, int, int, int] | None:
    """Capture the screen, let the user drag a region, return (x, y, w, h).

    Unlike capture_region(), this does NOT save a crop — it just returns the
    rectangle. Used for guard setup where we only need the bounds.
    """
    from .capture import ScreenCapture

    cap = ScreenCapture(backend="mss")  # one-shot, reliable
    frame = cap.grab()
    cap.close()
    if frame is None:
        logger.error("Could not capture screen for region picker")
        return None

    picker = NativeRegionPicker(frame)
    region = picker.pick()
    if region is None:
        logger.info("Region selection cancelled")
        return None

    x, y, w, h = region
    fh, fw = frame.shape[:2]
    x = max(0, min(x, fw - 1))
    y = max(0, min(y, fh - 1))
    w = min(w, fw - x)
    h = min(h, fh - y)
    return x, y, w, h
