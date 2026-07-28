/**
 * Compact top-edge indicator. The cat is drawn partly above the transparent
 * canvas, so the physical screen boundary becomes the ledge its paws grip.
 * Everything is pixel art and the page scales it 2× without smoothing.
 */

export interface Frame {
  mode: string;
  /** Seconds shown as one upright digit in each eye. */
  elapsed: number;
  /** Recording lamp cadence, owned by indicator.ts. */
  blinkOn: boolean;
  /** Radians used for the tiny tail-tip motion. */
  phase: number;
}

const OUTLINE = "rgb(26,24,22)";
const CREAM = "rgb(194,163,112)";
const EYE_BG = "rgb(12,11,10)";

export const INDICATOR_COLORS = {
  recording: "rgb(194,163,112)",
  playing: "rgb(93,176,136)",
  paused: "rgb(214,158,92)",
} as const;

/** 96×88 logical pixels, displayed at 2× by indicator.html. */
export const W = 96;
export const H = 88;

const HEAD_X = 20;
const HEAD_Y = 20;
const HEAD_W = 56;
const HEAD_H = 47;
const EYE_W = 16;
const EYE_H = 18;
const EYE_Y = 38;

const DIGITS: Readonly<Record<string, readonly number[]>> = {
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

export function counterDigits(elapsed: number): readonly [string, string] {
  const seconds = Number.isFinite(elapsed) ? Math.trunc(Math.max(0, elapsed)) % 100 : 0;
  return [String(Math.trunc(seconds / 10)), String(seconds % 10)];
}

export function indicatorColor(mode: string): string {
  if (mode === "playing") return INDICATOR_COLORS.playing;
  if (mode === "paused") return INDICATOR_COLORS.paused;
  return INDICATOR_COLORS.recording;
}

function fillRect(
  ctx: CanvasRenderingContext2D,
  color: string,
  x: number,
  y: number,
  width: number,
  height: number,
): void {
  ctx.fillStyle = color;
  ctx.fillRect(x, y, width, height);
}

/** Pixel-round block that paints only its silhouette and never clears art behind it. */
function fillRoundRect(
  ctx: CanvasRenderingContext2D,
  color: string,
  x: number,
  y: number,
  width: number,
  height: number,
  radius: number,
): void {
  ctx.fillStyle = color;
  for (let row = 0; row < height; row++) {
    const edge = Math.min(row, height - 1 - row);
    const inset = Math.max(0, radius - edge);
    ctx.fillRect(x + inset, y + row, width - inset * 2, 1);
  }
}

function stampDisc(ctx: CanvasRenderingContext2D, cx: number, cy: number, radius: number): void {
  for (let y = Math.round(cy - radius); y <= Math.round(cy + radius); y++) {
    const dy = y + 0.5 - cy;
    const half = Math.sqrt(Math.max(0, radius * radius - dy * dy));
    const x0 = Math.round(cx - half);
    const x1 = Math.round(cx + half);
    if (x1 > x0) ctx.fillRect(x0, y, x1 - x0, 1);
  }
}

function drawDigit(
  ctx: CanvasRenderingContext2D,
  digit: string,
  x: number,
  y: number,
  width: number,
  height: number,
  color: string,
): void {
  const pattern = DIGITS[digit];
  if (!pattern) return;
  const cellWidth = width / 3;
  const cellHeight = height / 5;
  ctx.fillStyle = color;
  for (let row = 0; row < 5; row++) {
    for (let column = 0; column < 3; column++) {
      if (!pattern[row * 3 + column]) continue;
      ctx.fillRect(
        Math.trunc(x + column * cellWidth),
        Math.trunc(y + row * cellHeight),
        Math.max(1, Math.round(cellWidth)),
        Math.max(1, Math.round(cellHeight)),
      );
    }
  }
}

/** An ear whose wide base joins the inverted head and whose tip points down. */
function drawDownEar(
  ctx: CanvasRenderingContext2D,
  color: string,
  x: number,
  y: number,
  width: number,
  height: number,
): void {
  ctx.fillStyle = color;
  for (let row = 0; row < height; row++) {
    const rowWidth = Math.max(2, Math.round(width * (1 - row / height)));
    const inset = Math.trunc((width - rowWidth) / 2);
    ctx.fillRect(x + inset, y + row, rowWidth, 1);
  }
}

function drawEars(ctx: CanvasRenderingContext2D): void {
  drawDownEar(ctx, CREAM, HEAD_X, 59, 20, 26);
  drawDownEar(ctx, CREAM, HEAD_X + HEAD_W - 20, 59, 20, 26);
  drawDownEar(ctx, OUTLINE, HEAD_X + 2, 61, 16, 21);
  drawDownEar(ctx, OUTLINE, HEAD_X + HEAD_W - 18, 61, 16, 21);

  // Small inner-ear marks keep the silhouette readable over dark games.
  drawDownEar(ctx, CREAM, HEAD_X + 7, 64, 6, 11);
  drawDownEar(ctx, CREAM, HEAD_X + HEAD_W - 13, 64, 6, 11);
}

/**
 * The paws start above y=0 and are intentionally clipped by the canvas. There is
 * no artificial ledge: at runtime the top of the monitor completes the illusion.
 */
function drawGrip(ctx: CanvasRenderingContext2D, centerX: number): void {
  // Forearm behind the head.
  fillRoundRect(ctx, CREAM, centerX - 5, 5, 10, 22, 2);
  fillRoundRect(ctx, OUTLINE, centerX - 3, 5, 6, 21, 1);

  // Broad hooked paw crossing the real screen edge.
  fillRoundRect(ctx, CREAM, centerX - 10, -6, 20, 20, 4);
  fillRoundRect(ctx, OUTLINE, centerX - 8, -5, 16, 17, 3);

  // Two tiny separations read as three toes without making the paw noisy.
  fillRect(ctx, CREAM, centerX - 3, 0, 1, 5);
  fillRect(ctx, CREAM, centerX + 2, 0, 1, 5);
}

/** Short curled tail peeking over the same edge; only its tip moves by one pixel. */
function drawTail(ctx: CanvasRenderingContext2D, phase: number, moving: boolean): void {
  const wobble = moving ? Math.round(Math.sin(phase)) : 0;
  const points: ReadonlyArray<readonly [number, number]> = [
    [79, -5],
    [84, -1],
    [88, 3],
    [89, 8],
    [87, 13],
    [82 + wobble, 15],
    [79 + wobble, 11],
  ];

  for (const [color, radius] of [
    [CREAM, 4],
    [OUTLINE, 2.5],
  ] as const) {
    ctx.fillStyle = color;
    for (let index = 0; index < points.length - 1; index++) {
      const [x0, y0] = points[index];
      const [x1, y1] = points[index + 1];
      const steps = Math.max(Math.abs(x1 - x0), Math.abs(y1 - y0), 1);
      for (let step = 0; step <= steps; step++) {
        const progress = step / steps;
        stampDisc(ctx, x0 + (x1 - x0) * progress, y0 + (y1 - y0) * progress, radius);
      }
    }
  }
}

function drawEyes(ctx: CanvasRenderingContext2D, frame: Frame): void {
  const eyeXs = [HEAD_X + 5, HEAD_X + HEAD_W - 5 - EYE_W];
  for (const eyeX of eyeXs) {
    fillRoundRect(ctx, CREAM, eyeX - 1, EYE_Y - 1, EYE_W + 2, EYE_H + 2, 3);
    fillRoundRect(ctx, EYE_BG, eyeX, EYE_Y, EYE_W, EYE_H, 2);
  }

  const [leftDigit, rightDigit] = counterDigits(frame.elapsed);
  const color = indicatorColor(frame.mode);
  drawDigit(ctx, leftDigit, eyeXs[0] + 3, EYE_Y + 3, EYE_W - 6, EYE_H - 6, color);
  drawDigit(ctx, rightDigit, eyeXs[1] + 3, EYE_Y + 3, EYE_W - 6, EYE_H - 6, color);
}

function drawFace(ctx: CanvasRenderingContext2D, frame: Frame): void {
  const centerX = HEAD_X + HEAD_W / 2;

  // Rotated facial order: mouth and nose are above the counter eyes.
  fillRect(ctx, CREAM, centerX - 5, 24, 10, 2);
  fillRect(ctx, CREAM, centerX - 1, 22, 2, 2);
  fillRect(ctx, CREAM, centerX - 3, 28, 6, 2);
  fillRect(ctx, CREAM, centerX - 2, 27, 4, 1);
  fillRect(ctx, CREAM, centerX - 1, 26, 2, 1);

  // Short whiskers fit inside the compact silhouette.
  fillRect(ctx, CREAM, HEAD_X + 4, 29, 9, 1);
  fillRect(ctx, CREAM, HEAD_X + 6, 32, 7, 1);
  fillRect(ctx, CREAM, HEAD_X + HEAD_W - 13, 29, 9, 1);
  fillRect(ctx, CREAM, HEAD_X + HEAD_W - 13, 32, 7, 1);

  drawEyes(ctx, frame);

  // State lamp sits on the inverted forehead, between the downward ears.
  const lit =
    frame.mode === "playing" || frame.mode === "paused"
      ? indicatorColor(frame.mode)
      : frame.blinkOn
        ? INDICATOR_COLORS.recording
        : null;
  if (lit) {
    ctx.fillStyle = CREAM;
    stampDisc(ctx, centerX, 61, 3.5);
    ctx.fillStyle = lit;
    stampDisc(ctx, centerX, 61, 2);
  }
}

function drawHead(ctx: CanvasRenderingContext2D, frame: Frame): void {
  drawEars(ctx);
  fillRoundRect(ctx, CREAM, HEAD_X - 2, HEAD_Y - 2, HEAD_W + 4, HEAD_H + 4, 5);
  fillRoundRect(ctx, OUTLINE, HEAD_X, HEAD_Y, HEAD_W, HEAD_H, 4);
  drawFace(ctx, frame);
}

export function renderCat(ctx: CanvasRenderingContext2D, frame: Frame): void {
  ctx.clearRect(0, 0, W, H);

  const moving = frame.mode !== "paused" && frame.mode !== "idle";
  drawTail(ctx, frame.phase, moving);
  drawGrip(ctx, 32);
  drawGrip(ctx, 64);
  drawHead(ctx, frame);
}
