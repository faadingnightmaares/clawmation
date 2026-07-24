"""Color sampler — click a point on screen to grab its HSV value.

Shows a fullscreen overlay with a live crosshair. When the user clicks,
returns the HSV value at that point (plus a small 5x5 average for stability).
"""

from __future__ import annotations

import ctypes
import logging
import threading
import time
from ctypes import wintypes
from pathlib import Path

import cv2
import numpy as np

from .capture import ScreenCapture

logger = logging.getLogger(__name__)

user32 = ctypes.windll.user32
gdi32 = ctypes.windll.gdi32
kernel32 = ctypes.windll.kernel32

# ── Win32 structs / prototypes ───────────────────────────────────────────────
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
gdi32.CreateCompatibleDC.argtypes = [wintypes.HDC]
gdi32.CreateCompatibleDC.restype = wintypes.HDC
gdi32.CreateCompatibleBitmap.argtypes = [wintypes.HDC, ctypes.c_int, ctypes.c_int]
gdi32.CreateCompatibleBitmap.restype = wintypes.HBITMAP
gdi32.BitBlt.argtypes = [
    wintypes.HDC, ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_int,
    wintypes.HDC, ctypes.c_int, ctypes.c_int, ctypes.c_uint,
]
gdi32.BitBlt.restype = wintypes.BOOL
gdi32.DeleteDC.argtypes = [wintypes.HDC]
gdi32.DeleteDC.restype = wintypes.BOOL
gdi32.CreatePen.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_uint]
gdi32.CreatePen.restype = wintypes.HPEN
gdi32.GetStockObject.argtypes = [ctypes.c_int]
gdi32.GetStockObject.restype = wintypes.HANDLE
gdi32.StretchDIBits.argtypes = [
    wintypes.HDC,
    ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_int,
    ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_int,
    ctypes.c_void_p, ctypes.POINTER(BITMAPINFO), ctypes.c_uint, ctypes.c_uint,
]
gdi32.StretchDIBits.restype = ctypes.c_int
gdi32.MoveToEx.argtypes = [wintypes.HDC, ctypes.c_int, ctypes.c_int, ctypes.POINTER(wintypes.POINT)]
gdi32.MoveToEx.restype = wintypes.BOOL
gdi32.LineTo.argtypes = [wintypes.HDC, ctypes.c_int, ctypes.c_int]
gdi32.LineTo.restype = wintypes.BOOL
gdi32.SetTextColor.argtypes = [wintypes.HDC, ctypes.c_uint]
gdi32.SetTextColor.restype = ctypes.c_uint
gdi32.SetBkMode.argtypes = [wintypes.HDC, ctypes.c_int]
gdi32.SetBkMode.restype = ctypes.c_int
gdi32.TextOutW.argtypes = [wintypes.HDC, ctypes.c_int, ctypes.c_int, wintypes.LPCWSTR, ctypes.c_int]
gdi32.TextOutW.restype = wintypes.BOOL

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
WM_MOUSEMOVE = 0x0200
WM_KEYDOWN = 0x0100
VK_ESCAPE = 0x1B
IDC_CROSS = 32515
DIB_RGB_COLORS = 0
BI_RGB = 0
SRCCOPY = 0x00CC0020
NULL_BRUSH = 5
TRANSPARENT = 1

WNDCLASS_NAME = "ClawmationColorSampler"


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


class ColorSampler:
    """Fullscreen click-to-sample overlay. Returns HSV at the clicked point."""

    def __init__(self, frame: np.ndarray):
        self.frame = frame
        self.h, self.w = frame.shape[:2]
        self._bits = _to_bgra_bottomup(frame)

        self.hwnd = 0
        self._thread: threading.Thread | None = None
        self._ready = threading.Event()
        self._done = threading.Event()
        self.result: dict | None = None  # {x, y, hsv_low, hsv_high, bgr}

        self._mx = self._my = 0  # current cursor

        self._wndproc = WNDPROC(self._on_message)

    # ── Public ───────────────────────────────────────────────────────────────

    def pick(self, timeout: float = 60.0) -> dict | None:
        """Open the overlay, block until click/cancel. Returns HSV dict or None."""
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
            logger.warning("Color sampler window failed: %s", e)
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
        # Set argtypes locally to avoid conflict with overlay's WNDCLASSW
        user32.RegisterClassW.argtypes = [ctypes.POINTER(WNDCLASSW)]
        user32.RegisterClassW(ctypes.byref(wc))

        sw = user32.GetSystemMetrics(0)  # SM_CXSCREEN
        sh = user32.GetSystemMetrics(1)  # SM_CYSCREEN

        self.hwnd = user32.CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            WNDCLASS_NAME,
            "Clawmation — Pick Color",
            WS_POPUP | WS_VISIBLE,
            0, 0, sw, sh,
            None, None, hinstance, None,
        )
        user32.ShowWindow(self.hwnd, 8)  # SW_SHOWNA
        user32.UpdateWindow(self.hwnd)
        user32.SetForegroundWindow(self.hwnd)

    # ── Message handler ──────────────────────────────────────────────────────

    def _on_message(self, hwnd, msg, wparam, lparam):
        if msg == WM_PAINT:
            self._paint(hwnd)
            return 0
        # Suppress background erase — prevents white flash between frames.
        if msg == WM_ERASEBKGND:
            return 1

        elif msg == WM_MOUSEMOVE:
            self._mx = lparam & 0xFFFF
            self._my = (lparam >> 16) & 0xFFFF
            user32.InvalidateRect(hwnd, None, False)
            return 0

        elif msg == WM_LBUTTONDOWN:
            x = lparam & 0xFFFF
            y = (lparam >> 16) & 0xFFFF
            self._sample(x, y)
            self._finish(hwnd)
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

    def _sample(self, x: int, y: int) -> None:
        """Grab HSV at (x, y) plus a 5x5 average for stability."""
        # Clamp to frame bounds
        x = max(2, min(x, self.w - 3))
        y = max(2, min(y, self.h - 3))

        # 5x5 region around the point
        region = self.frame[y - 2:y + 3, x - 2:x + 3]
        hsv_region = cv2.cvtColor(region, cv2.COLOR_BGR2HSV)

        # Average HSV
        h_avg = int(np.mean(hsv_region[:, :, 0]))
        s_avg = int(np.mean(hsv_region[:, :, 1]))
        v_avg = int(np.mean(hsv_region[:, :, 2]))

        # BGR at the exact point
        b, g, r = self.frame[y, x]

        # Build a tolerance range around the average (±10 H, ±30 S, ±30 V)
        h_tol, s_tol, v_tol = 10, 30, 30
        hsv_low = [
            max(0, h_avg - h_tol),
            max(0, s_avg - s_tol),
            max(0, v_avg - v_tol),
        ]
        hsv_high = [
            min(179, h_avg + h_tol),
            min(255, s_avg + s_tol),
            min(255, v_avg + v_tol),
        ]

        self.result = {
            "x": x,
            "y": y,
            "hsv": [h_avg, s_avg, v_avg],
            "hsv_low": hsv_low,
            "hsv_high": hsv_high,
            "bgr": [int(b), int(g), int(r)],
            "hex": f"#{int(r):02x}{int(g):02x}{int(b):02x}",
        }
        logger.info(f"Sampled color at ({x}, {y}): HSV={self.result['hsv']} BGR={self.result['bgr']}")

    # ── Drawing ──────────────────────────────────────────────────────────────

    def _dib_info(self, w: int, h: int) -> BITMAPINFO:
        bmi = BITMAPINFO()
        bmi.bmiHeader.biSize = ctypes.sizeof(BITMAPINFOHEADER)
        bmi.bmiHeader.biWidth = w
        bmi.bmiHeader.biHeight = h  # positive = bottom-up
        bmi.bmiHeader.biPlanes = 1
        bmi.bmiHeader.biBitCount = 32
        bmi.bmiHeader.biCompression = BI_RGB
        return bmi

    def _paint(self, hwnd) -> None:
        """Double-buffered paint: draw everything into a memory DC, then blit
        to the screen in one operation. Prevents flicker on mouse move."""
        ps = PAINTSTRUCT()
        hdc = user32.BeginPaint(hwnd, ctypes.byref(ps))
        sw = user32.GetSystemMetrics(0)
        sh = user32.GetSystemMetrics(1)

        # Create memory DC + compatible bitmap for off-screen rendering
        mem_dc = gdi32.CreateCompatibleDC(hdc)
        bmp = gdi32.CreateCompatibleBitmap(hdc, sw, sh)
        old_bmp = gdi32.SelectObject(mem_dc, bmp)
        try:
            # 1) Full screenshot
            bmi = self._dib_info(self.w, self.h)
            gdi32.StretchDIBits(
                mem_dc, 0, 0, self.w, self.h,
                0, 0, self.w, self.h,
                self._bits.ctypes.data, ctypes.byref(bmi), DIB_RGB_COLORS, SRCCOPY,
            )

            # 2) Crosshair at cursor
            pen = gdi32.CreatePen(0, 1, _rgb((255, 255, 255)))  # white
            gdi32.SelectObject(mem_dc, pen)
            gdi32.MoveToEx(mem_dc, self._mx - 15, self._my, None)
            gdi32.LineTo(mem_dc, self._mx + 15, self._my)
            gdi32.MoveToEx(mem_dc, self._mx, self._my - 15, None)
            gdi32.LineTo(mem_dc, self._mx, self._my + 15)
            gdi32.DeleteObject(pen)

            # 3) Hint text
            gdi32.SetBkMode(mem_dc, TRANSPARENT)
            gdi32.SetTextColor(mem_dc, _rgb((255, 255, 255)))
            hint = "Click to sample color · Esc to cancel"
            gdi32.TextOutW(mem_dc, 20, 20, hint, len(hint))

            # 4) Single blit to screen
            gdi32.BitBlt(hdc, 0, 0, sw, sh, mem_dc, 0, 0, SRCCOPY)
        finally:
            gdi32.SelectObject(mem_dc, old_bmp)
            gdi32.DeleteObject(bmp)
            gdi32.DeleteDC(mem_dc)
            user32.EndPaint(hwnd, ctypes.byref(ps))


def sample_color() -> dict | None:
    """Capture the screen, let the user click a point, return HSV at that point."""
    cap = ScreenCapture(backend="mss")
    frame = cap.grab()
    cap.close()
    if frame is None:
        logger.error("Could not capture screen for color sampler")
        return None

    sampler = ColorSampler(frame)
    return sampler.pick()
