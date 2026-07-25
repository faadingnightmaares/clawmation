/**
 * The recording-indicator overlay page. Rust (`shell::indicator`) owns the window
 * and shows/hides it via `set_mode`; this script owns the two clocks behind the
 * artwork in `./cat`: a 500ms `get_status` poll that feeds it mode and elapsed
 * seconds, and a faster frame tick that advances the tail's sway in between.
 *
 * Documented, deliberate divergence from Python: while paused the eyes FREEZE on
 * the last recording second rather than counting the pause duration up. `get_status`
 * reports elapsed 0 while paused (a semantic `TopBar`/`MacrosView` depend on), so
 * reproducing Python's pause-clock here would mean bending that shared status; the
 * frozen count is the obscure, purely cosmetic price.
 */
import { invoke } from "@tauri-apps/api/core";
import type { Status } from "../api";
import { renderCat, W, H } from "./cat";

const canvas = document.getElementById("cat") as HTMLCanvasElement;
// Sized from the art's own constants so the drawing surface can never disagree
// with it. The window and CSS sizes are 2x these; see `shell::indicator`.
canvas.width = W;
canvas.height = H;
const ctx = canvas.getContext("2d")!;

/** Status cadence. Matches Python's SetTimer, and one blink per two ticks. */
const POLL_MS = 500;
/** Frame cadence. Fast enough that the sway reads as motion, slow enough that an
 *  always-on-top overlay costs the game nothing. */
const FRAME_MS = 80;
const SWAY_PERIOD_MS = 2500;
const PHASE_STEP = (2 * Math.PI * FRAME_MS) / SWAY_PERIOD_MS;

let mode = "idle";
let elapsed = 0;
let blinkOn = true;
let phase = 0;
// Held so paused freezes on the last recording second (get_status reports 0 while
// paused). Reset at idle so the next recording starts counting from 00.
let heldElapsed = 0;

function paint(): void {
  renderCat(ctx, { mode, elapsed, blinkOn, phase });
}

async function poll(): Promise<void> {
  blinkOn = !blinkOn;
  let status: Status;
  try {
    status = await invoke<Status>("get_status");
  } catch {
    return; // backend not up yet; try again next tick
  }
  mode = status.mode ?? "idle";
  const live = status.elapsed ?? 0;
  if (mode === "recording" || mode === "playing") heldElapsed = live;
  else if (mode === "idle") heldElapsed = 0;
  elapsed = mode === "paused" ? heldElapsed : live;
  paint();
}

// Poll immediately as well as on the interval: while the window is hidden the
// webview throttles its timers to ~1s, so without this the first second of a
// recording would show a stale frame.
void poll();
setInterval(() => void poll(), POLL_MS);
document.addEventListener("visibilitychange", () => {
  if (document.visibilityState === "visible") void poll();
});

setInterval(() => {
  // Nothing moves at idle (hidden) or paused (sway is zero), so the repaint would
  // produce the identical frame.
  if (mode === "idle" || mode === "paused") return;
  phase = (phase + PHASE_STEP) % (2 * Math.PI);
  paint();
}, FRAME_MS);
