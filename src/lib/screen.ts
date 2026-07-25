// The native region/color pickers return *physical* pixel coordinates; guards
// and checkpoints store the watch area as screen percentages so a saved area
// survives a resolution change. The Python UI divided by a hardcoded 2560×1440
// — wrong on every other display. Derive the real physical size instead.

function screenSize(): { w: number; h: number } {
  const dpr = window.devicePixelRatio || 1;
  return {
    w: Math.round(window.screen.width * dpr),
    h: Math.round(window.screen.height * dpr),
  };
}

// Two decimals, not whole percent: one percent is 25.6 px on a 2560-wide screen,
// so rounding a box drawn tightly around a line of text can shift or clip it far
// enough that OCR reads nothing. The backend stores the region as f64 either way.
const clampPct = (n: number) => Math.max(0, Math.min(100, Math.round(n * 100) / 100));

/** A picked pixel rect `{x,y,w,h}` → `[x1,y1,x2,y2]` percentages (0–100). */
export function pxRectToPct(x: number, y: number, w: number, h: number): number[] {
  const s = screenSize();
  return [
    clampPct((x / s.w) * 100),
    clampPct((y / s.h) * 100),
    clampPct(((x + w) / s.w) * 100),
    clampPct(((y + h) / s.h) * 100),
  ];
}
