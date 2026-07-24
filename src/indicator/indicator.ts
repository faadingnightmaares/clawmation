/**
 * The recording-indicator overlay page — a render-only port of `overlay.py`'s
 * `_render_cat`. Rust (`shell::indicator`) owns the window and shows/hides it via
 * `set_mode`; this script just polls `get_status` every 500ms and paints the
 * pixel-cat: brown outline, cream ears/nose, the elapsed seconds as two pixel
 * digits in the eyes (gold recording / green playing / amber paused), and a chin
 * dot that blinks while recording and holds steady while playing or paused.
 *
 * Documented, deliberate divergence from Python: while paused the eyes FREEZE on
 * the last recording second rather than counting the pause duration up. `get_status`
 * reports elapsed 0 while paused (a semantic `TopBar`/`MacrosView` depend on), so
 * reproducing Python's pause-clock here would mean bending that shared status; the
 * frozen count is the obscure, purely cosmetic price.
 */
import { invoke } from "@tauri-apps/api/core";
import type { Status } from "../api";

// ── Palette (overlay.py) ────────────────────────────────────────────────────
const OUTLINE = "rgb(26,24,22)";
const CREAM = "rgb(194,163,112)";
const EYE_BG = "rgb(12,11,10)";
const DIGIT_REC = "rgb(194,163,112)"; // gold while recording
const DIGIT_PLAY = "rgb(93,176,136)"; // green while playing
const DIGIT_PAUSED = "rgb(214,158,92)"; // amber while recording-paused
const DOT_REC = "rgb(194,163,112)";
const DOT_PLAY = "rgb(93,176,136)";
const DOT_PAUSED = "rgb(214,158,92)";

// Sprite canvas size; the CSS scales it 2× with pixelated rendering.
const W = 72;
const H = 66;

// 3×5 pixel font for the elapsed-seconds digits drawn in the eyes.
const DIGITS: Record<string, number[]> = {
  "0": [1, 1, 1, 1, 0, 1, 1, 0, 1, 1, 0, 1, 1, 1, 1],
  "1": [0, 1, 0, 1, 1, 0, 0, 1, 0, 0, 1, 0, 1, 1, 1],
  "2": [1, 1, 1, 0, 0, 1, 1, 1, 1, 1, 0, 0, 1, 1, 1],
  "3": [1, 1, 1, 0, 0, 1, 1, 1, 1, 0, 0, 1, 1, 1, 1],
  "4": [1, 0, 1, 1, 0, 1, 1, 1, 1, 0, 0, 1, 0, 0, 1],
  "5": [1, 1, 1, 1, 0, 0, 1, 1, 1, 0, 0, 1, 1, 1, 1],
  "6": [1, 1, 1, 1, 0, 0, 1, 1, 1, 1, 0, 1, 1, 1, 1],
  "7": [1, 1, 1, 0, 0, 1, 0, 1, 0, 0, 1, 0, 0, 1, 0],
  "8": [1, 1, 1, 1, 0, 1, 1, 1, 1, 1, 0, 1, 1, 1, 1],
  "9": [1, 1, 1, 1, 0, 1, 1, 1, 1, 0, 0, 1, 1, 1, 1],
};

function fillRect(
  ctx: CanvasRenderingContext2D,
  color: string,
  x: number,
  y: number,
  w: number,
  h: number,
): void {
  ctx.fillStyle = color;
  ctx.fillRect(x, y, w, h);
}

// One pixel-font digit at (x, y) sized w×h — `_draw_digit`.
function drawDigit(
  ctx: CanvasRenderingContext2D,
  ch: string,
  x: number,
  y: number,
  w: number,
  h: number,
  color: string,
): void {
  const pat = DIGITS[ch];
  if (!pat) return;
  const cw = w / 3;
  const chH = h / 5;
  ctx.fillStyle = color;
  for (let r = 0; r < 5; r++) {
    for (let c = 0; c < 3; c++) {
      if (pat[r * 3 + c]) {
        ctx.fillRect(
          Math.trunc(x + c * cw),
          Math.trunc(y + r * chH),
          Math.max(1, Math.round(cw)),
          Math.max(1, Math.round(chH)),
        );
      }
    }
  }
}

// A stepped triangle ear that widens toward the bottom — `_draw_ear`.
function drawEar(
  ctx: CanvasRenderingContext2D,
  baseX: number,
  topY: number,
  width: number,
  height: number,
  steps: number,
  color: string,
): void {
  const stepH = height / steps;
  ctx.fillStyle = color;
  for (let i = 0; i < steps; i++) {
    const w = width * ((i + 1) / steps);
    const x = baseX + (width - w) / 2;
    const y = topY + i * stepH;
    ctx.fillRect(Math.trunc(x), Math.trunc(y), Math.round(w), Math.round(stepH) + 1);
  }
}

// Draw the whole cat into the transparent canvas — `_render_cat`.
function renderCat(
  ctx: CanvasRenderingContext2D,
  state: string,
  elapsed: number,
  blinkOn: boolean,
): void {
  ctx.clearRect(0, 0, W, H); // fully transparent ground — no background plate
  const play = state === "playing";
  const paused = state === "paused";
  const earLean = play ? 4 : 0;

  // Ears — outline, then cream inner
  drawEar(ctx, 4 - earLean, 2, 22, 20, 4, OUTLINE);
  drawEar(ctx, 46 + earLean, 2, 22, 20, 4, OUTLINE);
  drawEar(ctx, 8 - earLean, 6, 13, 13, 3, CREAM);
  drawEar(ctx, 51 + earLean, 6, 13, 13, 3, CREAM);

  // Head block, then round the corners by clearing 5×5 back to transparent
  const hx = 8;
  const hy = 14;
  const hw = 56;
  const hh = 46;
  fillRect(ctx, OUTLINE, hx, hy, hw, hh);
  const corners: Array<[number, number]> = [
    [hx, hy],
    [hx + hw - 5, hy],
    [hx, hy + hh - 5],
    [hx + hw - 5, hy + hh - 5],
  ];
  for (const [cx, cy] of corners) ctx.clearRect(cx, cy, 5, 5);

  // Eyes
  const eyeY = hy + 12;
  const eyeW = 16;
  const eyeH = 18;
  const leftX = hx + 7;
  const rightX = hx + hw - 7 - eyeW;
  fillRect(ctx, EYE_BG, leftX, eyeY, eyeW, eyeH);
  fillRect(ctx, EYE_BG, rightX, eyeY, eyeW, eyeH);

  // Elapsed seconds as two pixel digits inside the eyes
  const secs = Math.trunc(Math.max(0, elapsed)) % 100;
  const digitColor = play ? DIGIT_PLAY : paused ? DIGIT_PAUSED : DIGIT_REC;
  drawDigit(ctx, String(Math.trunc(secs / 10)), leftX + 3, eyeY + 3, eyeW - 6, eyeH - 6, digitColor);
  drawDigit(ctx, String(secs % 10), rightX + 3, eyeY + 3, eyeW - 6, eyeH - 6, digitColor);

  // Nose
  fillRect(ctx, CREAM, hx + Math.trunc(hw / 2) - 3, eyeY + eyeH + 4, 6, 5);

  // Mouth — flat line normally, two little fangs while playing
  if (play) {
    fillRect(ctx, EYE_BG, hx + Math.trunc(hw / 2) - 10, eyeY + eyeH + 11, 5, 3);
    fillRect(ctx, EYE_BG, hx + Math.trunc(hw / 2) + 5, eyeY + eyeH + 11, 5, 3);
  } else {
    fillRect(ctx, EYE_BG, hx + Math.trunc(hw / 2) - 8, eyeY + eyeH + 11, 16, 3);
  }

  // Chin dot — blinks while recording, steady while playing/paused
  const dotColor = play ? DOT_PLAY : paused ? DOT_PAUSED : blinkOn ? DOT_REC : null;
  if (dotColor) {
    const dx = Math.trunc(W / 2) - 3;
    const dy = H - 8;
    ctx.fillStyle = dotColor;
    for (let yy = dy; yy < dy + 6; yy++) {
      for (let xx = dx; xx < dx + 6; xx++) {
        if ((xx - dx - 2.5) ** 2 + (yy - dy - 2.5) ** 2 <= 9) {
          ctx.fillRect(xx, yy, 1, 1);
        }
      }
    }
  }
}

// ── Poll loop ───────────────────────────────────────────────────────────────
const canvas = document.getElementById("cat") as HTMLCanvasElement;
const ctx = canvas.getContext("2d")!;

let blinkOn = true;
// Held so paused freezes on the last recording second (get_status reports 0 while
// paused). Reset at idle so the next recording starts counting from 00.
let heldElapsed = 0;

async function tick(): Promise<void> {
  blinkOn = !blinkOn;
  let status: Status;
  try {
    status = await invoke<Status>("get_status");
  } catch {
    return; // backend not up yet — try again next tick
  }
  const mode = status.mode ?? "idle";
  const live = status.elapsed ?? 0;
  if (mode === "recording" || mode === "playing") heldElapsed = live;
  else if (mode === "idle") heldElapsed = 0;
  const shown = mode === "paused" ? heldElapsed : live;
  renderCat(ctx, mode, shown, blinkOn);
}

// Steady 500ms cadence (matches Python's SetTimer), plus an immediate repaint the
// instant Rust shows the window: while hidden the webview throttles its timer to
// ~1s, so without this the first second of a recording would show a stale frame.
void tick();
setInterval(() => void tick(), 500);
document.addEventListener("visibilitychange", () => {
  if (document.visibilityState === "visible") void tick();
});
