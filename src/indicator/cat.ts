/**
 * The recording indicator's artwork: a cat tail dropping in from off-screen whose
 * tip is the cat's own face, counting the elapsed seconds in her eyes.
 *
 * Pure drawing: it takes a canvas context and a frame description and nothing
 * else, so the art can be rendered outside Tauri while it is being worked on.
 * `indicator.ts` owns the window's poll loop and calls in here.
 *
 * The face keeps the pixel cat this replaced (a port of `overlay.py`'s
 * `_render_cat`): same 3x5 digit font, same eye and head proportions, same state
 * colours. What is new is the tail holding it up, and a cream rim on every
 * shape: the old cat was ink-on-transparent and vanished over a dark game.
 */

/** Runtime state the drawing reacts to. `mode` is `get_status`'s vocabulary. */
export interface Frame {
  mode: string;
  /** Seconds to show in the eyes; only the last two digits fit. */
  elapsed: number;
  /** Drives the recording light. Toggled by the caller, not by time here. */
  blinkOn: boolean;
  /** Radians. Advances continuously so the tail keeps swaying. */
  phase: number;
}

// ── Palette (inherited from overlay.py, which took it from the brand ramp) ────
const OUTLINE = "rgb(26,24,22)";
const CREAM = "rgb(194,163,112)";
const EYE_BG = "rgb(12,11,10)";
const DIGIT_REC = "rgb(194,163,112)"; // gold while recording
const DIGIT_PLAY = "rgb(93,176,136)"; // green while playing
const DIGIT_PAUSED = "rgb(214,158,92)"; // amber while recording-paused

/** Sprite size. The page scales it 2x with `image-rendering: pixelated`, so every
 *  number in this file is a hard pixel; there is no sub-pixel anything.
 *
 *  Tall rather than square: the window sits flush against the top of the screen,
 *  so this height *is* how far the tail falls before the face arrives. Change it
 *  and `HEIGHT` in `shell/indicator.rs` and `#cat` in `indicator.html` change with
 *  it, or the canvas and its window stop agreeing. */
export const W = 112;
export const H = 104;

// ── Head geometry ────────────────────────────────────────────────────────────
// Parked at the right so the tail has the left two-thirds to travel through, but
// far enough in that the ears still fit when they lean out. Low enough that the
// tail above it reads as a length of tail rather than a hook.
const HX = 52;
const HY = 48;
const HW = 52;
const HH = 46;
const EYE_W = 16;
const EYE_H = 18;
const EYE_Y = HY + 12;

// 3x5 pixel font for the two digits drawn in the eyes.
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

// ── The tail ─────────────────────────────────────────────────────────────────

/** Centre line of the tail, base first, as Catmull-Rom control points. The first
 *  sits above the canvas and the last inside the head: the tail must read as
 *  arriving from off-screen and ending *under* the face, not butting against it.
 *
 *  The base is deliberately cut off by the top of the canvas: with the window
 *  flush to the screen edge, that cut *is* the tail disappearing over the top of
 *  the screen, and the illusion only holds while the spine starts above y=0. */
const SPINE: ReadonlyArray<readonly [number, number]> = [
  [22, -16],
  [16, 14],
  [11, 40],
  [12, 64],
  [24, 82],
  [42, 92],
  [58, 88],
];

const TAIL_BASE_R = 5.4;
const TAIL_TIP_R = 2.8;
/** How far the middle of the tail swings, in pixels either side. Scaled with the
 *  tail's length; the same throw over a longer tail reads as a stiffer one. */
const SWAY = 3.2;
/** Width of the cream rim around the whole silhouette. The cat is ink-coloured,
 *  which is invisible over a dark game; the rim is what makes her read on any
 *  background instead of only on light ones. */
const RIM = 1.4;

function catmullRom(p0: number, p1: number, p2: number, p3: number, t: number): number {
  const t2 = t * t;
  const t3 = t2 * t;
  return (
    0.5 *
    (2 * p1 + (p2 - p0) * t + (2 * p0 - 5 * p1 + 4 * p2 - p3) * t2 + (3 * p1 - p0 - 3 * p2 + p3) * t3)
  );
}

/** Point at `t` along the whole spine, with the sway applied. The sway is scaled
 *  by `sin(pi*t)` so both ends stay pinned: the base is anchored off-screen and
 *  the tip is anchored to the head, and a tail that slid either would look
 *  detached rather than alive. */
function spineAt(t: number, wobble: number): [number, number] {
  const segs = SPINE.length - 1;
  const scaled = Math.min(t, 0.999999) * segs;
  const i = Math.floor(scaled);
  const local = scaled - i;
  const at = (k: number) => SPINE[Math.max(0, Math.min(segs, k))];
  const [x0, y0] = at(i - 1);
  const [x1, y1] = at(i);
  const [x2, y2] = at(i + 1);
  const [x3, y3] = at(i + 2);
  return [
    catmullRom(x0, x1, x2, x3, local) + wobble * Math.sin(Math.PI * t),
    catmullRom(y0, y1, y2, y3, local),
  ];
}

/** A filled circle drawn as one rect per scanline: pixel-exact and cheap enough
 *  to redraw the whole tail every frame. */
function stampDisc(ctx: CanvasRenderingContext2D, cx: number, cy: number, r: number): void {
  for (let y = Math.round(cy - r); y <= Math.round(cy + r); y++) {
    const dy = y + 0.5 - cy;
    const half = Math.sqrt(Math.max(0, r * r - dy * dy));
    const x0 = Math.round(cx - half);
    const x1 = Math.round(cx + half);
    if (x1 > x0) ctx.fillRect(x0, y, x1 - x0, 1);
  }
}

/** Stamp the spine from `t0` to `t1` at the local radius plus `grow`. */
function stampSpan(
  ctx: CanvasRenderingContext2D,
  color: string,
  t0: number,
  t1: number,
  grow: number,
  wobble: number,
): void {
  ctx.fillStyle = color;
  const steps = Math.max(2, Math.round((t1 - t0) * 90));
  for (let s = 0; s <= steps; s++) {
    const t = t0 + ((t1 - t0) * s) / steps;
    const [x, y] = spineAt(t, wobble);
    const r = TAIL_BASE_R + (TAIL_TIP_R - TAIL_BASE_R) * t + grow;
    if (r > 0) stampDisc(ctx, x, y, r);
  }
}

function drawTail(ctx: CanvasRenderingContext2D, wobble: number): void {
  stampSpan(ctx, CREAM, 0, 1, RIM, wobble);
  stampSpan(ctx, OUTLINE, 0, 1, 0, wobble);
}

// ── The head ─────────────────────────────────────────────────────────────────

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

/** One pixel-font digit at (x, y) sized w x h (`_draw_digit`). */
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

/** A stepped triangle ear that widens toward the bottom (`_draw_ear`). */
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

/** The same ear with a 2px cream rim, made by stamping cream copies around an ink
 *  one. Growing the triangle instead would thicken only its base; the rim has to
 *  survive all the way to the tip, which is the part that says "ear". */
function outlinedEar(
  ctx: CanvasRenderingContext2D,
  baseX: number,
  topY: number,
  width: number,
  height: number,
  steps: number,
): void {
  for (const [dx, dy] of [
    [-2, 0],
    [2, 0],
    [0, -2],
    [-2, -2],
    [2, -2],
  ]) {
    drawEar(ctx, baseX + dx, topY + dy, width, height, steps, CREAM);
  }
  drawEar(ctx, baseX, topY, width, height, steps, OUTLINE);
}

/** Round a rectangle's corners into a diagonal stair. `color` of `null` cuts them
 *  out of the silhouette; a colour paints them instead, which is what inner shapes
 *  need: clearing an eye would punch a hole clean through the head behind it. */
function stairCorners(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  w: number,
  h: number,
  n: number,
  color: string | null,
): void {
  if (color !== null) ctx.fillStyle = color;
  const put = (px: number, py: number, pw: number) =>
    color === null ? ctx.clearRect(px, py, pw, 1) : ctx.fillRect(px, py, pw, 1);
  for (let i = 0; i < n; i++) {
    const cut = n - i;
    for (const row of [y + i, y + h - 1 - i]) {
      put(x, row, cut);
      put(x + w - cut, row, cut);
    }
  }
}

function drawHead(ctx: CanvasRenderingContext2D, frame: Frame): void {
  const play = frame.mode === "playing";
  const paused = frame.mode === "paused";

  // Ears, aligned so their widest step lands exactly on the head's rim edges.
  // A pixel either way and the bottom step juts out as a tab on the head's side.
  outlinedEar(ctx, HX, HY - 14, 18, 18, 5);
  outlinedEar(ctx, HX + HW - 18, HY - 14, 18, 18, 5);

  // Head block over its cream rim. The rim's corners are cut out of the
  // silhouette; the block's are painted cream, so the rim keeps going round the
  // curve instead of leaving the chip that cutting both of them left.
  fillRect(ctx, CREAM, HX - 2, HY - 2, HW + 4, HH + 4);
  stairCorners(ctx, HX - 2, HY - 2, HW + 4, HH + 4, 4, null);
  fillRect(ctx, OUTLINE, HX, HY, HW, HH);
  stairCorners(ctx, HX, HY, HW, HH, 3, CREAM);

  // Eyes, and the elapsed seconds inside them. Cream-rimmed for the same reason
  // the body is: over a dark game the ink head disappears, and without the rims
  // the digits would hang in space with no face around them.
  const leftX = HX + 7;
  const rightX = HX + HW - 7 - EYE_W;
  for (const ex of [leftX, rightX]) {
    fillRect(ctx, CREAM, ex - 1, EYE_Y - 1, EYE_W + 2, EYE_H + 2);
    stairCorners(ctx, ex - 1, EYE_Y - 1, EYE_W + 2, EYE_H + 2, 2, OUTLINE);
    fillRect(ctx, EYE_BG, ex, EYE_Y, EYE_W, EYE_H);
    stairCorners(ctx, ex, EYE_Y, EYE_W, EYE_H, 1, CREAM);
  }
  const secs = Math.trunc(Math.max(0, frame.elapsed)) % 100;
  const digit = play ? DIGIT_PLAY : paused ? DIGIT_PAUSED : DIGIT_REC;
  const dw = EYE_W - 6;
  const dh = EYE_H - 6;
  drawDigit(ctx, String(Math.trunc(secs / 10)), leftX + 3, EYE_Y + 3, dw, dh, digit);
  drawDigit(ctx, String(secs % 10), rightX + 3, EYE_Y + 3, dw, dh, digit);

  // Nose
  const midX = HX + Math.trunc(HW / 2);
  fillRect(ctx, CREAM, midX - 3, EYE_Y + EYE_H + 4, 6, 5);

  // Mouth: a flat line normally, two little fangs while playing
  if (play) {
    fillRect(ctx, CREAM, midX - 10, EYE_Y + EYE_H + 11, 5, 3);
    fillRect(ctx, CREAM, midX + 5, EYE_Y + EYE_H + 11, 5, 3);
  } else {
    fillRect(ctx, CREAM, midX - 8, EYE_Y + EYE_H + 11, 16, 3);
  }

  // The recording light, on the chin. Blinks while recording; holds while playing
  // or paused, which is the difference between "capturing" and "busy".
  const lit = play ? DIGIT_PLAY : paused ? DIGIT_PAUSED : frame.blinkOn ? DIGIT_REC : null;
  if (lit) {
    // Rimmed like everything else: gold on a pale desktop is otherwise a smudge.
    ctx.fillStyle = OUTLINE;
    stampDisc(ctx, midX, HY + HH + 5, 4.4);
    ctx.fillStyle = lit;
    stampDisc(ctx, midX, HY + HH + 5, 3.0);
  }
}

// ── The whole picture ────────────────────────────────────────────────────────

/** Sway speed per state: a slow breath while recording, brisker while a macro is
 *  playing, dead still while paused, so the tail alone tells you which. */
function swayFor(mode: string): number {
  if (mode === "paused") return 0;
  return mode === "playing" ? SWAY : SWAY * 0.7;
}

export function renderCat(ctx: CanvasRenderingContext2D, frame: Frame): void {
  ctx.clearRect(0, 0, W, H); // transparent ground; the overlay has no plate
  drawTail(ctx, Math.sin(frame.phase) * swayFor(frame.mode));
  drawHead(ctx, frame);
}
