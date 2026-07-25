/**
 * The live detection overlay page: the boxes over whatever the triggers are
 * finding, for as long as they are finding it.
 *
 * Rust (`shell::detections`) owns the window: fullscreen, transparent,
 * click-through, and excluded from screen capture, shown only while a detection
 * loop is armed. This script owns what is drawn in it, from one command,
 * `get_detections`, which merges the last pass of every live detector (the
 * playback guard engine, the standing watcher, the checkpoint and step
 * detectors) into one list of boxes in capture pixels, each carrying its age.
 *
 * Two clocks, for the same reason the indicator has two: the backend is polled
 * at a rate that keeps it cheap, and the drawing runs per frame so the fade
 * between passes is smooth rather than stepped. Age is extrapolated locally
 * between polls: the backend's `age_ms` is the anchor, `performance.now()` the
 * interpolation, so a stalled backend fades its boxes out instead of freezing
 * them on screen.
 */
import { invoke } from "@tauri-apps/api/core";

interface Sighting {
  label: string;
  /** Top-left and size, in capture pixels. */
  x: number;
  y: number;
  w: number;
  h: number;
  confidence: number;
  /** Age of the pass that found *this* box. Per box rather than per pass,
   *  because the sources feeding the overlay poll at their own rates. */
  age_ms: number;
}

interface Pass {
  /** Size of the frame the coordinates are in. The overlay scales by this
   *  rather than assuming the capture matched the monitor. */
  screen: [number, number];
  items: Sighting[];
}

/** Backend cadence. Twice the guard engine's own 5Hz poll, so a box appears
 *  within a frame or two of the detection that produced it. */
const POLL_MS = 100;
/** Full strength for a shade longer than the slowest detector's poll interval,
 *  so a match that keeps matching never dips. */
const HOLD_MS = 320;
/** Then out over this long. A detection that stops matching leaves rather than
 *  blinking off, which is what makes a flickering match legible as flickering. */
const FADE_MS = 420;

const GOLD = "#e8c07a";
const INK = "#0b0906";

const canvas = document.getElementById("boxes") as HTMLCanvasElement;
const ctx = canvas.getContext("2d")!;

let pass: Pass | null = null;
/** `performance.now()` when `pass` arrived, and the age it claimed then. */
let passAt = 0;
/** True while the canvas holds ink, so an idle overlay does no work per frame. */
let painted = false;

function resize(): void {
  const dpr = window.devicePixelRatio || 1;
  canvas.width = Math.round(window.innerWidth * dpr);
  canvas.height = Math.round(window.innerHeight * dpr);
  // Draw in CSS pixels; the backing store carries the device pixels.
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  painted = false;
}

function roundRect(x: number, y: number, w: number, h: number, r: number): void {
  ctx.beginPath();
  ctx.moveTo(x + r, y);
  ctx.arcTo(x + w, y, x + w, y + h, r);
  ctx.arcTo(x + w, y + h, x, y + h, r);
  ctx.arcTo(x, y + h, x, y, r);
  ctx.arcTo(x, y, x + w, y, r);
  ctx.closePath();
}

/** One detection: a hairline rectangle, heavier brackets at the corners, and a
 *  name plate. Everything is drawn twice (a dark spread underneath, the gold on
 *  top) because the thing behind it is someone else's game and may be any
 *  colour at all. */
function drawBox(x: number, y: number, w: number, h: number, label: string, alpha: number): void {
  ctx.lineJoin = "round";
  ctx.lineCap = "round";

  ctx.globalAlpha = alpha * 0.5;
  ctx.strokeStyle = INK;
  ctx.lineWidth = 4;
  ctx.strokeRect(x, y, w, h);

  ctx.globalAlpha = alpha;
  ctx.strokeStyle = GOLD;
  ctx.lineWidth = 1;
  ctx.strokeRect(x, y, w, h);

  // Brackets sized off the box, so a small match is not swallowed by its own
  // corners.
  const t = Math.max(4, Math.min(16, Math.min(w, h) * 0.3));
  ctx.lineWidth = 3;
  ctx.beginPath();
  ctx.moveTo(x, y + t);
  ctx.lineTo(x, y);
  ctx.lineTo(x + t, y);
  ctx.moveTo(x + w - t, y);
  ctx.lineTo(x + w, y);
  ctx.lineTo(x + w, y + t);
  ctx.moveTo(x + w, y + h - t);
  ctx.lineTo(x + w, y + h);
  ctx.lineTo(x + w - t, y + h);
  ctx.moveTo(x + t, y + h);
  ctx.lineTo(x, y + h);
  ctx.lineTo(x, y + h - t);
  ctx.stroke();

  ctx.font = "600 12px ui-sans-serif, Segoe UI, system-ui, sans-serif";
  const textW = ctx.measureText(label).width;
  const plateW = textW + 12;
  const plateH = 18;
  // Above the box by preference, tucked inside its top edge when the match sits
  // against the top of the screen and there is no room.
  const plateY = y - plateH - 3 < 0 ? y + 3 : y - plateH - 3;
  const plateX = Math.min(Math.max(0, x), window.innerWidth - plateW);

  ctx.globalAlpha = alpha * 0.82;
  ctx.fillStyle = INK;
  roundRect(plateX, plateY, plateW, plateH, 4);
  ctx.fill();

  ctx.globalAlpha = alpha;
  ctx.fillStyle = GOLD;
  ctx.textBaseline = "middle";
  ctx.fillText(label, plateX + 6, plateY + plateH / 2 + 0.5);
}

function draw(): void {
  const [sw, sh] = pass?.screen ?? [0, 0];
  // The backend's age is the anchor; the time since the poll landed is added
  // locally, so the fade runs per frame rather than in poll-sized steps.
  const since = pass ? performance.now() - passAt : 0;
  const visible = (pass?.items ?? [])
    .map((s) => {
      const age = s.age_ms + since;
      return { s, alpha: age <= HOLD_MS ? 1 : 1 - (age - HOLD_MS) / FADE_MS };
    })
    .filter((v) => v.alpha > 0);

  if (visible.length === 0 || sw <= 0 || sh <= 0) {
    if (painted) {
      ctx.clearRect(0, 0, window.innerWidth, window.innerHeight);
      painted = false;
    }
    return;
  }

  ctx.clearRect(0, 0, window.innerWidth, window.innerHeight);
  const kx = window.innerWidth / sw;
  const ky = window.innerHeight / sh;
  for (const { s, alpha } of visible) {
    const pct = Math.round(s.confidence * 100);
    drawBox(s.x * kx, s.y * ky, s.w * kx, s.h * ky, `${s.label}  ${pct}%`, alpha);
  }
  ctx.globalAlpha = 1;
  painted = true;
}

async function poll(): Promise<void> {
  try {
    const next = await invoke<Pass>("get_detections");
    pass = next;
    passAt = performance.now();
  } catch {
    // Backend not up yet, or on its way down. The boxes age out on their own.
  }
}

resize();
window.addEventListener("resize", resize);

// The window is hidden between runs, and a hidden window's frame callbacks stop:
// whatever was on the canvas when the last macro ended is still on it when the
// next one starts. Wipe it on the way back up, before the first frame is shown.
document.addEventListener("visibilitychange", () => {
  if (document.visibilityState !== "visible") return;
  pass = null;
  draw();
  void poll();
});

void poll();
setInterval(() => void poll(), POLL_MS);

function frame(): void {
  draw();
  requestAnimationFrame(frame);
}
requestAnimationFrame(frame);
