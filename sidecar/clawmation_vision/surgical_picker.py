"""Surgical capture — drag a tight box, adjust it, then mark the EXACT pixels.

Two-phase native Win32 overlay (single app, no taskbar button):

  Phase 1 (select + adjust): fullscreen frozen frame, darkened; drag a tight
    box around the element. On release the box enters *adjust* mode: drag
    corners to resize, drag inside to move, arrow keys nudge (Shift = 10 px),
    right-click resets, Enter confirms. Dimension readout shown live.

  Phase 2 (target): the cropped element is shown ZOOMED (nearest-neighbor so
    pixels stay crisp) and centered on a dark backdrop, with a 1px grid when
    the zoom is large enough, a live crosshair + coordinate readout that
    follows the cursor, and a zoom-level indicator. Click to place a point,
    drag to draw a line. Draw as many strokes as needed; Enter commits them
    all. Backspace with no strokes returns to Phase 1 adjust.

Returns (crop_path, offset_x, offset_y, crop_w, crop_h, thumb_b64) or None.

Verbatim port of `anime_macro/surgical_picker.py` into the vision sidecar. The
one departure from the desktop original: the save directory is passed in by
value rather than defaulting to a `TEMPLATES_DIR` global, keeping the sidecar
free of path globals like its sibling pickers (`region_picker`, `color_sampler`).
"""

from __future__ import annotations

import base64
import ctypes
import logging
import threading
import time
from ctypes import wintypes
from pathlib import Path

import cv2
import numpy as np

logger = logging.getLogger(__name__)

user32 = ctypes.windll.user32
gdi32 = ctypes.windll.gdi32
kernel32 = ctypes.windll.kernel32

# ── Win32 structs / prototypes (self-contained, mirrors region_picker) ────────
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
user32.SetWindowTextW.argtypes = [wintypes.HWND, wintypes.LPCWSTR]
user32.SetWindowTextW.restype = wintypes.BOOL
user32.GetKeyState.argtypes = [ctypes.c_int]
user32.GetKeyState.restype = ctypes.c_short

gdi32.SelectObject.argtypes = [wintypes.HDC, wintypes.HANDLE]
gdi32.SelectObject.restype = wintypes.HANDLE
gdi32.DeleteObject.argtypes = [wintypes.HANDLE]
gdi32.DeleteObject.restype = wintypes.BOOL
gdi32.CreatePen.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_uint]
gdi32.CreatePen.restype = wintypes.HPEN
gdi32.CreateSolidBrush.argtypes = [wintypes.COLORREF]
gdi32.CreateSolidBrush.restype = wintypes.HBRUSH
gdi32.Rectangle.argtypes = [wintypes.HDC, ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_int]
gdi32.Rectangle.restype = wintypes.BOOL
gdi32.MoveToEx.argtypes = [wintypes.HDC, ctypes.c_int, ctypes.c_int, wintypes.LPVOID]
gdi32.MoveToEx.restype = wintypes.BOOL
gdi32.LineTo.argtypes = [wintypes.HDC, ctypes.c_int, ctypes.c_int]
gdi32.LineTo.restype = wintypes.BOOL
gdi32.GetStockObject.argtypes = [ctypes.c_int]
gdi32.GetStockObject.restype = wintypes.HANDLE
gdi32.StretchDIBits.argtypes = [
    wintypes.HDC,
    ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_int,
    ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_int,
    ctypes.c_void_p, ctypes.POINTER(BITMAPINFO), ctypes.c_uint, ctypes.c_uint,
]
gdi32.StretchDIBits.restype = ctypes.c_int
gdi32.SetBkMode.argtypes = [wintypes.HDC, ctypes.c_int]
gdi32.SetBkMode.restype = ctypes.c_int
gdi32.SetTextColor.argtypes = [wintypes.HDC, ctypes.c_uint]
gdi32.SetTextColor.restype = ctypes.c_uint
gdi32.TextOutW.argtypes = [wintypes.HDC, ctypes.c_int, ctypes.c_int, wintypes.LPCWSTR, ctypes.c_int]
gdi32.TextOutW.restype = wintypes.BOOL

# Double-buffering: draw to a memory DC, then one BitBlt to the screen.
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
WM_MOUSEMOVE = 0x0200
WM_LBUTTONDOWN = 0x0201
WM_LBUTTONUP = 0x0202
WM_RBUTTONDOWN = 0x0204
WM_MOUSEWHEEL = 0x020A
WM_KEYDOWN = 0x0100
VK_ESCAPE = 0x1B
VK_RETURN = 0x0D
IDC_CROSS = 32515
DIB_RGB_COLORS = 0
BI_RGB = 0
SRCCOPY = 0x00CC0020
NULL_BRUSH = 5
TRANSPARENT = 1

WNDCLASS_NAME = "ClawmationSurgicalPicker"
DARKEN = 0.42
GOLD = (74, 168, 192)      # BGR — selection outline
GRID = (60, 60, 60)        # BGR — pixel grid
CROSS = (74, 168, 192)     # BGR — crosshair
LABEL_BG = (30, 28, 26)    # BGR — readout chip


def _rgb(bgr: tuple[int, int, int]) -> int:
    b, g, r = bgr
    return (b << 16) | (g << 8) | r


def _to_bgra_bottomup(img: np.ndarray) -> np.ndarray:
    h, w = img.shape[:2]
    bgra = np.empty((h, w, 4), dtype=np.uint8)
    bgra[:, :, :3] = img
    bgra[:, :, 3] = 255
    return np.ascontiguousarray(np.flipud(bgra))


class SurgicalPicker:
    """Two-phase surgical capture overlay drawn with GDI (no webview)."""

    def __init__(self, frame: np.ndarray):
        self.frame = frame
        self.h, self.w = frame.shape[:2]
        self.dark = (frame.astype(np.float32) * DARKEN).clip(0, 255).astype(np.uint8)
        self._dark_bits = _to_bgra_bottomup(self.dark)

        self.hwnd = 0
        self._thread: threading.Thread | None = None
        self._ready = threading.Event()
        self._done = threading.Event()
        # result: (crop_x, crop_y, crop_w, crop_h, strokes)
        # strokes = list of (sx, sy, ex, ey) offsets relative to crop top-left.
        # Each is a drawn line; a single click yields a zero-length stroke.
        self.result: tuple[int, int, int, int, list] | None = None

        # Phase 1 state (select region)
        self._phase = 1
        self._dragging = False
        self._select_mode = 'drag'   # 'drag' or 'adjust'
        self._moving = False         # moving selection in adjust mode
        self._move_dx = 0            # offset from cursor to selection origin
        self._move_dy = 0
        self._resize_corner = -1     # -1=none, 0=TL, 1=TR, 2=BL, 3=BR
        self._sx = self._sy = 0
        self._cx = self._cy = 0

        # Phase 2 state (draw the exact pixels to hit)
        self._crop = None          # np.ndarray crop
        self._crop_rect = None     # (x, y, w, h) in screen coords
        self._zoom = 1
        self._disp_w = self._disp_h = 0   # zoomed display size
        self._ox = self._oy = 0    # display top-left on screen
        self._zoom_bits = None
        self._mx = self._my = -1   # cursor in screen coords
        # Drawn strokes (offsets in crop space). Each stroke is (sx, sy, ex, ey).
        # The user draws as many strokes as they want; Enter commits them all.
        self._strokes: list[tuple[int, int, int, int]] = []
        self._drawing = False
        self._line_start: tuple[int, int] | None = None
        self._line_end: tuple[int, int] | None = None
        # On-screen "Done" button rect (screen coords), recomputed each paint.
        self._done_rect: tuple[int, int, int, int] | None = None
        # Grid toggle (G key) — show/hide the pixel grid in phase 2.
        self._show_grid = True
        self._wndproc = WNDPROC(self._on_message)

    # ── Public ───────────────────────────────────────────────────────────────

    def pick(self, timeout: float = 180.0) -> tuple[int, int, int, int, int, int] | None:
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
            logger.warning("Surgical picker window failed: %s", e)
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

        sw = user32.GetSystemMetrics(0)
        sh = user32.GetSystemMetrics(1)
        self.hwnd = user32.CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            WNDCLASS_NAME,
            "Clawmation — Drag a tight box around the element",
            WS_POPUP | WS_VISIBLE,
            0, 0, sw, sh,
            None, None, hinstance, None,
        )
        user32.ShowWindow(self.hwnd, 8)
        user32.UpdateWindow(self.hwnd)
        user32.SetForegroundWindow(self.hwnd)

    # ── Message handler ──────────────────────────────────────────────────────

    def _on_message(self, hwnd, msg, wparam, lparam):
        if msg == WM_PAINT:
            if self._phase == 1:
                self._paint_select(hwnd)
            else:
                self._paint_target(hwnd)
            return 0
        # Suppress background erase — prevents white flash between frames.
        # We draw the entire frame via BitBlt, so the system erase is wasted.
        if msg == WM_ERASEBKGND:
            return 1

        if self._phase == 1:
            return self._handle_select(hwnd, msg, wparam, lparam)
        return self._handle_target(hwnd, msg, wparam, lparam)

    # ── Phase 1: select region + adjust ─────────────────────────────────────

    def _hit_corner(self, cx, cy, margin=10) -> int:
        """Return corner index (0=TL,1=TR,2=BL,3=BR) if cursor is near one, else -1."""
        x, y, w, h = self._selection_rect()
        corners = [(x, y), (x + w, y), (x, y + h), (x + w, y + h)]
        for i, (hx, hy) in enumerate(corners):
            if abs(cx - hx) <= margin and abs(cy - hy) <= margin:
                return i
        return -1

    def _clamp_selection(self) -> None:
        """Keep the selection inside the frame bounds."""
        x, y, w, h = self._selection_rect()
        w = max(6, min(w, self.w))
        h = max(6, min(h, self.h))
        x = max(0, min(x, self.w - w))
        y = max(0, min(y, self.h - h))
        self._sx, self._sy = x, y
        self._cx, self._cy = x + w, y + h

    def _nudge_selection(self, dx, dy) -> None:
        """Move the whole selection by (dx, dy), clamped to frame."""
        x, y, w, h = self._selection_rect()
        x = max(0, min(x + dx, self.w - w))
        y = max(0, min(y + dy, self.h - h))
        self._sx, self._sy = x, y
        self._cx, self._cy = x + w, y + h
        user32.InvalidateRect(self.hwnd, None, False)

    def _back_to_adjust(self, hwnd) -> None:
        """Return from Phase 2 to Phase 1 adjust mode (crop was wrong)."""
        self._phase = 1
        self._select_mode = 'adjust'
        self._strokes = []
        self._drawing = False
        self._line_start = self._line_end = None
        self._crop = None
        self._crop_rect = None
        self._zoom_bits = None
        user32.SetWindowTextW(hwnd, "Clawmation — Adjust the box (Enter to confirm, Esc to cancel)")
        user32.InvalidateRect(hwnd, None, False)

    def _handle_select(self, hwnd, msg, wparam, lparam):
        cx = lparam & 0xFFFF
        cy = (lparam >> 16) & 0xFFFF

        if msg == WM_LBUTTONDOWN:
            if self._select_mode == 'drag':
                # Fresh drag from scratch
                self._sx = self._cx = cx
                self._sy = self._cy = cy
                self._dragging = True
                user32.InvalidateRect(hwnd, None, False)
            else:
                # Adjust mode: check corners first, then inside-move
                corner = self._hit_corner(cx, cy)
                if corner >= 0:
                    self._resize_corner = corner
                    self._dragging = True
                else:
                    x, y, w, h = self._selection_rect()
                    if x <= cx <= x + w and y <= cy <= y + h:
                        self._moving = True
                        self._move_dx = cx - x
                        self._move_dy = cy - y
                user32.InvalidateRect(hwnd, None, False)
            return 0

        elif msg == WM_MOUSEMOVE:
            if self._select_mode == 'drag' and self._dragging:
                self._cx = cx
                self._cy = cy
                user32.InvalidateRect(hwnd, None, False)
            elif self._select_mode == 'adjust':
                if self._resize_corner >= 0 and self._dragging:
                    # Resize: the opposite corner stays fixed
                    x, y, w, h = self._selection_rect()
                    if self._resize_corner == 0:      # TL
                        self._sx, self._sy = cx, cy
                        self._cx, self._cy = x + w, y + h
                    elif self._resize_corner == 1:    # TR
                        self._sx, self._sy = x, cy
                        self._cx, self._cy = cx, y + h
                    elif self._resize_corner == 2:    # BL
                        self._sx, self._sy = cx, y
                        self._cx, self._cy = x + w, cy
                    else:                             # BR
                        self._sx, self._sy = x, y
                        self._cx, self._cy = cx, cy
                    self._clamp_selection()
                    user32.InvalidateRect(hwnd, None, False)
                elif self._moving:
                    x, y, w, h = self._selection_rect()
                    nx = max(0, min(cx - self._move_dx, self.w - w))
                    ny = max(0, min(cy - self._move_dy, self.h - h))
                    self._sx, self._sy = nx, ny
                    self._cx, self._cy = nx + w, ny + h
                    user32.InvalidateRect(hwnd, None, False)
            return 0

        elif msg == WM_LBUTTONUP:
            if self._select_mode == 'drag' and self._dragging:
                self._dragging = False
                self._cx = cx
                self._cy = cy
                x, y, w, h = self._selection_rect()
                if w >= 6 and h >= 6:
                    # Enter adjust mode — don't jump to Phase 2 yet
                    self._select_mode = 'adjust'
                    self._clamp_selection()
                    user32.SetWindowTextW(hwnd,
                        "Clawmation — Adjust the box (Enter to confirm, Esc to cancel)")
                user32.InvalidateRect(hwnd, None, False)
            elif self._select_mode == 'adjust':
                self._dragging = False
                self._resize_corner = -1
                self._moving = False
                self._clamp_selection()
                user32.InvalidateRect(hwnd, None, False)
            return 0

        elif msg == WM_RBUTTONDOWN:
            if self._select_mode == 'adjust':
                # Reset to fresh drag
                self._select_mode = 'drag'
                self._dragging = False
                self._resize_corner = -1
                self._moving = False
                user32.SetWindowTextW(hwnd,
                    "Clawmation — Drag a tight box around the element")
                user32.InvalidateRect(hwnd, None, False)
            return 0

        elif msg == WM_KEYDOWN:
            if wparam == VK_ESCAPE:
                self.result = None
                self._finish(hwnd)
            elif wparam == VK_RETURN and self._select_mode == 'adjust':
                x, y, w, h = self._selection_rect()
                if w >= 6 and h >= 6:
                    self._enter_target_phase(hwnd, x, y, w, h)
            elif self._select_mode == 'adjust':
                # Arrow-key nudge; Shift = 10 px
                step = 10 if (user32.GetKeyState(0x10) & 0x8000) else 1
                if wparam == 0x25:      # Left
                    self._nudge_selection(-step, 0)
                elif wparam == 0x27:    # Right
                    self._nudge_selection(step, 0)
                elif wparam == 0x26:    # Up
                    self._nudge_selection(0, -step)
                elif wparam == 0x28:    # Down
                    self._nudge_selection(0, step)
            return 0

        elif msg == WM_DESTROY:
            user32.PostQuitMessage(0)
            return 0
        return user32.DefWindowProcW(hwnd, msg, wparam, lparam)

    def _enter_target_phase(self, hwnd, x, y, w, h) -> None:
        # Clamp to frame
        x = max(0, min(x, self.w - 1))
        y = max(0, min(y, self.h - 1))
        w = min(w, self.w - x)
        h = min(h, self.h - y)
        self._crop_rect = (x, y, w, h)
        self._crop = self.frame[y:y + h, x:x + w].copy()

        # Compute zoom: fit the crop into ~70% of the screen, integer-ish, min 4x
        sw = user32.GetSystemMetrics(0)
        sh = user32.GetSystemMetrics(1)
        max_disp_w = int(sw * 0.72)
        max_disp_h = int(sh * 0.72)
        zoom = max(4, min(max_disp_w // max(1, w), max_disp_h // max(1, h)))
        zoom = min(zoom, 48)  # cap so a 1px crop doesn't blow up absurdly
        self._zoom = zoom
        self._disp_w = w * zoom
        self._disp_h = h * zoom
        self._ox = (sw - self._disp_w) // 2
        self._oy = (sh - self._disp_h) // 2

        # Nearest-neighbor zoom keeps pixels crisp
        zoomed = cv2.resize(self._crop, (self._disp_w, self._disp_h),
                            interpolation=cv2.INTER_NEAREST)
        self._zoom_bits = _to_bgra_bottomup(zoomed)

        self._phase = 2
        user32.SetWindowTextW(hwnd, "Clawmation — DRAG to draw the exact pixels to hit (Esc cancels)")
        user32.InvalidateRect(hwnd, None, False)

    # ── Phase 2: draw the exact pixels to hit ────────────────────────────────

    def _handle_target(self, hwnd, msg, wparam, lparam):
        if msg == WM_LBUTTONDOWN:
            cx = lparam & 0xFFFF
            cy = (lparam >> 16) & 0xFFFF
            # Click the "Done" button → commit all strokes and close.
            if self._done_rect is not None:
                bx0, by0, bx1, by1 = self._done_rect
                if bx0 <= cx <= bx1 and by0 <= cy <= by1:
                    self._commit_all(hwnd)
                    return 0
            # Otherwise start a new stroke: lock its start pixel under the cursor.
            off = self._cursor_to_offset()
            if off is not None:
                self._drawing = True
                self._line_start = off
                self._line_end = off
                user32.InvalidateRect(hwnd, None, False)
            return 0
        elif msg == WM_MOUSEMOVE:
            self._mx = lparam & 0xFFFF
            self._my = (lparam >> 16) & 0xFFFF
            if self._drawing:
                off = self._cursor_to_offset()
                if off is not None:
                    self._line_end = off
            user32.InvalidateRect(hwnd, None, False)
            return 0
        elif msg == WM_LBUTTONUP:
            # Finish the current stroke: append it to the list, keep drawing.
            if self._drawing:
                self._drawing = False
                off = self._cursor_to_offset()
                if off is not None:
                    self._line_end = off
                if self._line_start is not None:
                    end = self._line_end or self._line_start
                    self._strokes.append(
                        (self._line_start[0], self._line_start[1], end[0], end[1])
                    )
                self._line_start = self._line_end = None
                user32.InvalidateRect(hwnd, None, False)
            return 0
        elif msg == WM_RBUTTONDOWN:
            # Right-click: undo the last stroke (or cancel an in-progress one).
            if self._drawing:
                self._drawing = False
                self._line_start = self._line_end = None
            elif self._strokes:
                self._strokes.pop()
            user32.InvalidateRect(hwnd, None, False)
            return 0
        elif msg == WM_KEYDOWN:
            if wparam == VK_ESCAPE:
                self.result = None
                self._finish(hwnd)
            elif wparam == VK_RETURN:
                # Commit every drawn stroke and close.
                self._commit_all(hwnd)
            elif wparam == 0x08:  # Backspace: undo last stroke, or go back if none
                if self._strokes:
                    self._strokes.pop()
                    user32.InvalidateRect(hwnd, None, False)
                else:
                    self._back_to_adjust(hwnd)
            elif wparam == 0x47:  # G key: toggle pixel grid
                self._show_grid = not self._show_grid
                user32.InvalidateRect(hwnd, None, False)
            return 0
        elif msg == WM_MOUSEWHEEL:
            # Scroll to zoom: wheel up = zoom in, wheel down = zoom out.
            # wparam high word = wheel delta (positive = up/forward).
            delta = ctypes.c_short(wparam >> 16).value
            if delta > 0:
                self._zoom = min(48, self._zoom + max(1, self._zoom // 4))
            else:
                self._zoom = max(4, self._zoom - max(1, self._zoom // 4))
            self._rebuild_zoom(hwnd)
            return 0
        elif msg == WM_DESTROY:
            user32.PostQuitMessage(0)
            return 0
        return user32.DefWindowProcW(hwnd, msg, wparam, lparam)

    def _commit_all(self, hwnd) -> None:
        """Commit all drawn strokes into the result and close the overlay."""
        if not self._strokes:
            return
        cx, cy, cw, ch = self._crop_rect
        self.result = (cx, cy, cw, ch, [list(s) for s in self._strokes])
        self._finish(hwnd)

    def _rebuild_zoom(self, hwnd) -> None:
        """Recompute the zoomed crop display after a zoom change."""
        if self._crop is None:
            return
        _, _, cw, ch = self._crop_rect
        sw = user32.GetSystemMetrics(0)
        sh = user32.GetSystemMetrics(1)
        self._disp_w = cw * self._zoom
        self._disp_h = ch * self._zoom
        self._ox = (sw - self._disp_w) // 2
        self._oy = (sh - self._disp_h) // 2
        zoomed = cv2.resize(self._crop, (self._disp_w, self._disp_h),
                            interpolation=cv2.INTER_NEAREST)
        self._zoom_bits = _to_bgra_bottomup(zoomed)
        user32.InvalidateRect(hwnd, None, False)

    def _cursor_to_offset(self) -> tuple[int, int] | None:
        """Map the screen-space cursor to a (col, row) offset inside the crop."""
        if self._mx < 0:
            return None
        px = (self._mx - self._ox) // self._zoom
        py = (self._my - self._oy) // self._zoom
        _, _, cw, ch = self._crop_rect
        if 0 <= px < cw and 0 <= py < ch:
            return px, py
        return None

    # ── Drawing ──────────────────────────────────────────────────────────────

    def _dib_info(self, w: int, h: int) -> BITMAPINFO:
        bmi = BITMAPINFO()
        bmi.bmiHeader.biSize = ctypes.sizeof(BITMAPINFOHEADER)
        bmi.bmiHeader.biWidth = w
        bmi.bmiHeader.biHeight = h
        bmi.bmiHeader.biPlanes = 1
        bmi.bmiHeader.biBitCount = 32
        bmi.bmiHeader.biCompression = BI_RGB
        return bmi

    def _selection_rect(self) -> tuple[int, int, int, int]:
        x = min(self._sx, self._cx)
        y = min(self._sy, self._cy)
        w = abs(self._cx - self._sx)
        h = abs(self._cy - self._sy)
        return x, y, w, h

    def _paint_buffered(self, hwnd, draw_fn) -> None:
        """Draw everything into an off-screen memory DC, then blit to the screen
        in a single operation. This is what keeps the overlay stable like
        Lightshot — no per-primitive flicker, no partial redraws on mouse move."""
        ps = PAINTSTRUCT()
        hdc = user32.BeginPaint(hwnd, ctypes.byref(ps))
        sw = user32.GetSystemMetrics(0)
        sh = user32.GetSystemMetrics(1)
        mem_dc = gdi32.CreateCompatibleDC(hdc)
        bmp = gdi32.CreateCompatibleBitmap(hdc, sw, sh)
        old_bmp = gdi32.SelectObject(mem_dc, bmp)
        try:
            draw_fn(mem_dc, sw, sh)
            gdi32.BitBlt(hdc, 0, 0, sw, sh, mem_dc, 0, 0, SRCCOPY)
        finally:
            gdi32.SelectObject(mem_dc, old_bmp)
            gdi32.DeleteObject(bmp)
            gdi32.DeleteDC(mem_dc)
            user32.EndPaint(hwnd, ctypes.byref(ps))

    def _paint_select(self, hwnd) -> None:
        self._paint_buffered(hwnd, self._draw_select)

    def _draw_select(self, hdc, sw, sh) -> None:
        bmi = self._dib_info(self.w, self.h)
        gdi32.StretchDIBits(
            hdc, 0, 0, self.w, self.h, 0, 0, self.w, self.h,
            self._dark_bits.ctypes.data, ctypes.byref(bmi), DIB_RGB_COLORS, SRCCOPY,
        )
        x, y, w, h = self._selection_rect()
        show = (self._dragging and w > 0 and h > 0) or self._select_mode == 'adjust'
        if not show:
            return
        # Bright crop
        crop = self.frame[y:y + h, x:x + w]
        bits = _to_bgra_bottomup(crop)
        bmi2 = self._dib_info(w, h)
        gdi32.StretchDIBits(
            hdc, x, y, w, h, 0, 0, w, h,
            bits.ctypes.data, ctypes.byref(bmi2), DIB_RGB_COLORS, SRCCOPY,
        )
        # Gold outline
        pen = gdi32.CreatePen(0, 2, _rgb(GOLD))
        gdi32.SelectObject(hdc, pen)
        gdi32.SelectObject(hdc, gdi32.GetStockObject(NULL_BRUSH))
        gdi32.Rectangle(hdc, x, y, x + w, y + h)
        gdi32.DeleteObject(pen)

        if self._select_mode == 'adjust':
            # Corner handles (small filled squares)
            hs = 5
            brush = gdi32.CreateSolidBrush(_rgb(GOLD))
            gdi32.SelectObject(hdc, brush)
            for hx, hy in [(x, y), (x + w, y), (x, y + h), (x + w, y + h)]:
                gdi32.Rectangle(hdc, hx - hs, hy - hs, hx + hs, hy + hs)
            gdi32.DeleteObject(brush)
            # Dimension readout below the box
            dim = f"{w} x {h}"
            gdi32.SetBkMode(hdc, TRANSPARENT)
            gdi32.SetTextColor(hdc, _rgb((120, 200, 220)))
            gdi32.TextOutW(hdc, x, y + h + 6, dim, len(dim))
            # Instruction line at top
            instr = "Enter = confirm   arrows = nudge (Shift=10)   right-click = redo   Esc = cancel"
            gdi32.TextOutW(hdc, max(8, (sw - len(instr) * 8) // 2), 10, instr, len(instr))

    def _paint_target(self, hwnd) -> None:
        self._paint_buffered(hwnd, self._draw_target)

    def _draw_target(self, hdc, sw, sh) -> None:
        # 1) Dark backdrop
        bmi = self._dib_info(self.w, self.h)
        gdi32.StretchDIBits(
            hdc, 0, 0, self.w, self.h, 0, 0, self.w, self.h,
            self._dark_bits.ctypes.data, ctypes.byref(bmi), DIB_RGB_COLORS, SRCCOPY,
        )

        # 2) Zoomed crop
        if self._zoom_bits is not None:
            bmi2 = self._dib_info(self._disp_w, self._disp_h)
            gdi32.StretchDIBits(
                hdc, self._ox, self._oy, self._disp_w, self._disp_h,
                0, 0, self._disp_w, self._disp_h,
                self._zoom_bits.ctypes.data, ctypes.byref(bmi2), DIB_RGB_COLORS, SRCCOPY,
            )

        # 3) Pixel grid (only when zoom is large enough to see AND grid is enabled)
        if self._show_grid and self._zoom >= 8:
            _, _, cw, ch = self._crop_rect
            gpen = gdi32.CreatePen(0, 1, _rgb(GRID))
            gdi32.SelectObject(hdc, gpen)
            for i in range(0, cw + 1):
                gx = self._ox + i * self._zoom
                gdi32.MoveToEx(hdc, gx, self._oy, None)
                gdi32.LineTo(hdc, gx, self._oy + self._disp_h)
            for j in range(0, ch + 1):
                gy = self._oy + j * self._zoom
                gdi32.MoveToEx(hdc, self._ox, gy, None)
                gdi32.LineTo(hdc, self._ox + self._disp_w, gy)
            gdi32.DeleteObject(gpen)

        # 4) Gold border around the zoomed crop
        bpen = gdi32.CreatePen(0, 2, _rgb(GOLD))
        gdi32.SelectObject(hdc, bpen)
        gdi32.SelectObject(hdc, gdi32.GetStockObject(NULL_BRUSH))
        gdi32.Rectangle(hdc, self._ox, self._oy,
                        self._ox + self._disp_w, self._oy + self._disp_h)
        gdi32.DeleteObject(bpen)

        # 5) The drawn strokes — the exact pixels to hit. Every committed stroke
        #    is a thick gold line with endpoint dots; the in-progress stroke
        #    (while dragging) follows the cursor live.
        lpen = gdi32.CreatePen(0, max(2, self._zoom // 3), _rgb(CROSS))
        epen = gdi32.CreatePen(0, 2, _rgb((255, 255, 255)))
        r = max(3, self._zoom // 2)

        def draw_stroke(sx, sy, ex, ey):
            x1 = self._ox + sx * self._zoom + self._zoom // 2
            y1 = self._oy + sy * self._zoom + self._zoom // 2
            x2 = self._ox + ex * self._zoom + self._zoom // 2
            y2 = self._oy + ey * self._zoom + self._zoom // 2
            gdi32.SelectObject(hdc, lpen)
            gdi32.MoveToEx(hdc, x1, y1, None)
            gdi32.LineTo(hdc, x2, y2)
            gdi32.SelectObject(hdc, epen)
            gdi32.SelectObject(hdc, gdi32.GetStockObject(NULL_BRUSH))
            gdi32.Rectangle(hdc, x1 - r, y1 - r, x1 + r, y1 + r)
            gdi32.Rectangle(hdc, x2 - r, y2 - r, x2 + r, y2 + r)

        for (sx, sy, ex, ey) in self._strokes:
            draw_stroke(sx, sy, ex, ey)

        # In-progress stroke (live, while dragging)
        if self._drawing and self._line_start is not None:
            end = self._line_end or self._line_start
            draw_stroke(self._line_start[0], self._line_start[1], end[0], end[1])

        gdi32.DeleteObject(lpen)
        gdi32.DeleteObject(epen)

        # Live crosshair + coordinate readout under the cursor when idle
        if not self._drawing:
            off = self._cursor_to_offset()
            if off is not None:
                px, py = off
                midx = self._ox + px * self._zoom + self._zoom // 2
                midy = self._oy + py * self._zoom + self._zoom // 2
                cpen = gdi32.CreatePen(0, 1, _rgb(CROSS))
                gdi32.SelectObject(hdc, cpen)
                gdi32.MoveToEx(hdc, midx, self._oy, None)
                gdi32.LineTo(hdc, midx, self._oy + self._disp_h)
                gdi32.MoveToEx(hdc, self._ox, midy, None)
                gdi32.LineTo(hdc, self._ox + self._disp_w, midy)
                gdi32.DeleteObject(cpen)
                # Coordinate chip near cursor
                coord = f"{px},{py}"
                gdi32.SetBkMode(hdc, TRANSPARENT)
                gdi32.SetTextColor(hdc, _rgb((255, 255, 255)))
                gdi32.TextOutW(hdc, midx + 10, midy - 18, coord, len(coord))

        # Zoom indicator — bottom-right of the crop
        zoom_txt = f"{self._zoom}x"
        gdi32.SetBkMode(hdc, TRANSPARENT)
        gdi32.SetTextColor(hdc, _rgb((100, 180, 200)))
        gdi32.TextOutW(hdc, self._ox + self._disp_w - 36,
                       self._oy + self._disp_h + 4, zoom_txt, len(zoom_txt))

        self._draw_label(hdc, sw)

    def _draw_label(self, hdc, sw: int) -> None:
        _, _, cw, ch = self._crop_rect
        n = len(self._strokes)
        if self._drawing:
            text = f"drawing stroke {n + 1}...   release to add it"
        elif n == 0:
            text = f"crop {cw}x{ch}   click = point  ·  drag = line  ·  scroll = zoom  ·  G = grid  ·  Backspace = back"
        else:
            text = f"{n} stroke{'s' if n != 1 else ''}   right-click/Backspace = undo  ·  Enter = done  ·  Esc = cancel"
        gdi32.SetBkMode(hdc, TRANSPARENT)
        gdi32.SetTextColor(hdc, _rgb((120, 200, 220)))  # light gold-ish, BGR
        tx = max(8, (sw - len(text) * 8) // 2)
        ty = max(8, self._oy - 26)
        gdi32.TextOutW(hdc, tx, ty, text, len(text))

        # "Done" button — only once at least one stroke is drawn. Centered below
        # the zoomed crop. Clicking it commits all strokes (same as Enter).
        self._done_rect = None
        if n > 0 and not self._drawing:
            label = "DONE  (Enter)"
            bw, bh = len(label) * 9 + 28, 34
            bx = (sw - bw) // 2
            by = self._oy + self._disp_h + 18
            self._done_rect = (bx, by, bx + bw, by + bh)
            # Filled gold button
            brush = gdi32.CreateSolidBrush(_rgb(GOLD))
            gdi32.SelectObject(hdc, brush)
            bpen = gdi32.CreatePen(0, 1, _rgb(GOLD))
            gdi32.SelectObject(hdc, bpen)
            gdi32.Rectangle(hdc, bx, by, bx + bw, by + bh)
            gdi32.DeleteObject(brush)
            gdi32.DeleteObject(bpen)
            # Label (dark text on gold)
            gdi32.SetTextColor(hdc, _rgb((20, 18, 12)))
            gdi32.TextOutW(hdc, bx + 14, by + 9, label, len(label))

    def _finish(self, hwnd) -> None:
        try:
            user32.DestroyWindow(hwnd)
        except Exception:
            pass
        self._done.set()


def surgical_capture(save_dir: Path) -> dict | None:
    """Run the two-phase surgical capture.

    Phase 1: drag a tight box around the element.
    Phase 2: DRAG to draw the exact pixels to hit. Draw as many strokes as you
             want, then press Enter to lock them all (right-click/Backspace
             undoes the last stroke, Esc cancels).

    Returns a dict:
      {
        "path": str,            # saved crop (template image)
        "click_lines": [[sx,sy,ex,ey], ...],  # every drawn stroke, crop offsets
        "click_line": [sx,sy,ex,ey],  # first stroke (back-compat single field)
        "offset": [sx, sy],     # first stroke's start (back-compat point field)
        "w": int, "h": int,     # crop size
        "thumb": str,           # base64 PNG thumbnail with the strokes drawn
      }
    or None if cancelled.
    """
    from .capture import ScreenCapture

    save_dir.mkdir(parents=True, exist_ok=True)

    cap = ScreenCapture(backend="mss")
    frame = cap.grab()
    cap.close()
    if frame is None:
        logger.error("Could not capture screen for surgical picker")
        return None

    picker = SurgicalPicker(frame)
    res = picker.pick()
    if res is None:
        logger.info("Surgical capture cancelled")
        return None

    cx, cy, cw, ch, strokes = res
    crop = frame[cy:cy + ch, cx:cx + cw]
    if crop.size == 0:
        logger.error("Empty surgical crop")
        return None

    path = save_dir / f"surgical_{int(time.time())}_{cw}x{ch}.png"
    cv2.imwrite(str(path), crop)

    # Thumbnail for the UI, with every drawn stroke rendered on it.
    thumb = crop.copy()
    th = 96
    scale = th / max(1, crop.shape[0])
    thumb = cv2.resize(crop, (max(1, int(crop.shape[1] * scale)), th),
                       interpolation=cv2.INTER_AREA)
    xscale = thumb.shape[1] / max(1, crop.shape[1])
    yscale = thumb.shape[0] / max(1, crop.shape[0])
    lw = max(2, int(3 * min(xscale, yscale)))
    for (sx, sy, ex, ey) in strokes:
        p1 = (int(sx * xscale), int(sy * yscale))
        p2 = (int(ex * xscale), int(ey * yscale))
        cv2.line(thumb, p1, p2, (74, 168, 192), lw)
        cv2.circle(thumb, p1, max(3, int(4 * xscale)), (255, 255, 255), -1)
        cv2.circle(thumb, p2, max(3, int(4 * xscale)), (74, 168, 192), -1)
    ok, buf = cv2.imencode(".png", thumb)
    thumb_b64 = base64.b64encode(buf.tobytes()).decode("ascii") if ok else ""

    first = strokes[0]
    logger.info("Surgical capture: %s (%dx%d) %d stroke(s)",
                path.name, cw, ch, len(strokes))
    return {
        "path": str(path),
        "click_lines": [[int(a), int(b), int(c), int(d)] for (a, b, c, d) in strokes],
        "click_line": [int(first[0]), int(first[1]), int(first[2]), int(first[3])],
        "offset": [int(first[0]), int(first[1])],
        "w": int(cw),
        "h": int(ch),
        "thumb": thumb_b64,
    }
