// Small pure formatting helpers, ported 1:1 from the Python UI's script block
// (anime_macro/ui/index.html). Kept in one module so the macro / vision / chain
// views share a single copy.

/** Repeat-slider stops, low → high. Index 0 (`∞`) means loop forever. */
export const STOPS = ["∞", "1", "2", "3", "5", "10"] as const;

/** `sec` → `m:ss` (seconds zero-padded). Mirrors `fmtDur`. */
export function fmtDur(sec: number): string {
  const m = Math.floor(sec / 60);
  const s = Math.round(sec % 60);
  return m + ":" + String(s).padStart(2, "0");
}

/**
 * A Unix epoch (seconds) → `just now` / `Nm ago` / `Nh ago` / `Nd ago`, or `""`
 * for a falsy epoch. Mirrors `fmtAgo`.
 */
export function fmtAgo(epoch: number): string {
  if (!epoch) return "";
  const diff = Math.floor(Date.now() / 1000 - epoch);
  if (diff < 60) return "just now";
  if (diff < 3600) return Math.floor(diff / 60) + "m ago";
  if (diff < 86400) return Math.floor(diff / 3600) + "h ago";
  return Math.floor(diff / 86400) + "d ago";
}

/** Repeat count for a slider index: index 0 (`∞`) → 0 (forever), else the number. */
export function repsFor(repeatIndex: number): number {
  const v = STOPS[repeatIndex]!;
  return v === "∞" ? 0 : parseInt(v, 10);
}

/**
 * Inverse of {@link repsFor}: a saved `(loop, loop_count)` pair → slider index.
 * `loop` with count 0 is infinite (0); no loop is play-once (1); otherwise the
 * matching stop, falling back to play-once. Mirrors `repeatToIndex`.
 */
export function repeatToIndex(loop: boolean, loopCount: number): number {
  if (loop && loopCount === 0) return 0;
  if (!loop) return 1;
  const idx = STOPS.findIndex((x) => x === String(loopCount));
  return idx >= 0 ? idx : 1;
}

/** Normalize a hotkey string for display: `f6`→`F6`, `ctrl+r`→`Ctrl+R`. */
export function fmtHotkey(k: string): string {
  if (!k) return "";
  return String(k)
    .split("+")
    .map((p) => (p.length === 1 ? p.toUpperCase() : p[0].toUpperCase() + p.slice(1).toLowerCase()))
    .join("+");
}

/**
 * Approximate the midpoint of an HSV range as a CSS colour for a swatch. OpenCV
 * hue is 0–179; saturation/value 0–255. Returns `transparent` when either bound
 * is missing. Mirrors the live `hsvToCss` (index.html:3570). The source defines
 * an earlier same-named method (index.html:2694, halved lightness + gold
 * fallback) that this later definition shadows on the class prototype, so 2694
 * never runs — this ports the one that actually does.
 */
export function hsvToCss(low?: number[], high?: number[]): string {
  if (!low || !high) return "transparent";
  const h = Math.round(((low[0] + high[0]) / 2) * 2);
  const s = Math.round((((low[1] + high[1]) / 2) / 255) * 100);
  const l = Math.round((((low[2] + high[2]) / 2) / 255) * 100);
  return `hsl(${h}, ${s}%, ${l}%)`;
}
