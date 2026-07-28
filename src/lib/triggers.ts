// A "trigger" is one guard: a thing Clawmation watches for on screen, plus what
// it does when it appears. Guards (per-macro safety) and Watch triggers share
// the exact same persisted shape; this module is the single source of truth for
// converting between that stored shape and the editor's working draft.
//
// The stored guard is a closed set of 18 fields (see the byte-identical `clean`
// objects the old MacrosView/VisionView both wrote). The `Guard` type carries a
// `[key: string]: unknown` index signature, so a mistyped field name assigns
// cleanly and silently no-ops on save; tsc cannot catch it. To stay honest the
// draft reuses the persisted field names verbatim as pure pass-throughs, and the
// whole plain-language reframe (Image/Color/Text, Strict/Balanced/Loose, "wait
// after" outcomes) lives in the editor UI, derived from these fields by the pure
// helpers below. triggers.test.ts round-trips the converters to guarantee it.

import { type Guard } from "@/api";

export type LookKind = "image" | "color" | "text";

export interface TriggerPreview {
  src: string; // full data URI
  ok: boolean;
  msg?: string;
}

/** The editor's working copy of one trigger. A superset of the persisted guard:
 *  the same field names (so conversion is near-identity) plus `_extra` for any
 *  unknown stored keys and the transient `_preview` the Test button fills in. */
export interface TriggerDraft {
  id: string;
  name: string;
  method: string; // "color" | "template" | "ocr"
  action: string; // "click" | "key" | "nudge"
  key: string;
  hsv_low: number[];
  hsv_high: number[];
  template_path: string;
  ocr_text: string;
  threshold: number;
  region: number[];
  min_area: number;
  resume_delay: number;
  cooldown: number;
  enabled: boolean;
  click_offset: number[];
  click_line: number[];
  click_lines: number[][];
  /** Any stored key not in the closed persisted set, preserved across a round-trip. */
  _extra: Record<string, unknown>;
  _preview?: TriggerPreview | null;
}

/** The exact set of fields the backend persists for a guard/trigger. Anything on
 *  a stored guard outside this set is unknown and gets shuttled through `_extra`. */
const PERSISTED_KEYS = [
  "id",
  "name",
  "method",
  "action",
  "key",
  "hsv_low",
  "hsv_high",
  "template_path",
  "ocr_text",
  "threshold",
  "region",
  "min_area",
  "resume_delay",
  "cooldown",
  "enabled",
  "click_offset",
  "click_line",
  "click_lines",
] as const;

// ── Look ↔ detection method ────────────────────────────────────────────────
// Persisted `method` is one of color | template | ocr. The UI speaks in plain
// nouns: a Color, an Image (a captured button picture), or on-screen Text.
const METHOD_TO_LOOK: Record<string, LookKind> = {
  color: "color",
  template: "image",
  ocr: "text",
  text: "text", // tolerate a legacy alias on load
};
const LOOK_TO_METHOD: Record<LookKind, string> = {
  color: "color",
  image: "template",
  text: "ocr",
};

export function lookOf(method: string): LookKind {
  return METHOD_TO_LOOK[method] ?? "color";
}
export function methodFor(look: LookKind): string {
  return LOOK_TO_METHOD[look];
}

// ── Accuracy ↔ match threshold (Image only) ────────────────────────────────
// The old editor exposed a raw 0.50 to 0.99 "Match ≥" slider. We replace the number
// with three named stops; the raw threshold still round-trips untouched for any
// value a stop doesn't sit exactly on.
export const ACCURACY_STOPS: { key: "loose" | "balanced" | "strict"; label: string; desc: string; threshold: number }[] = [
  { key: "loose", label: "Forgiving", desc: "Catches it even if it looks a little different", threshold: 0.62 },
  { key: "balanced", label: "Balanced", desc: "A sensible middle ground", threshold: 0.8 },
  { key: "strict", label: "Exact", desc: "Only an almost-perfect match counts", threshold: 0.9 },
];

export function accuracyOf(threshold: number): "loose" | "balanced" | "strict" {
  const t = threshold ?? 0.8;
  if (t >= 0.87) return "strict";
  if (t >= 0.72) return "balanced";
  return "loose";
}

// ── "Wait after it acts" ↔ resume_delay seconds ────────────────────────────
// Reframes the raw seconds field as an outcome the user recognises. Picking a
// stop writes its canonical seconds; a stored value between stops still shows the
// nearest label and round-trips its own number.
export const RESUME_STOPS: { seconds: number; label: string; hint: string }[] = [
  { seconds: 0, label: "Carry on right away", hint: "No pause" },
  { seconds: 3, label: "Give it a moment", hint: "A short breath before resuming" },
  { seconds: 6, label: "Wait for a load", hint: "Loading screens, brief reconnects" },
  { seconds: 12, label: "Long reconnect", hint: "Rejoining a world takes a while" },
];

/** The canonical seconds of the stop nearest a stored delay, used to highlight
 *  the right chip without mutating the stored value. */
export function resumeStopOf(seconds: number): number {
  const s = seconds ?? 0;
  let best = RESUME_STOPS[0];
  for (const stop of RESUME_STOPS) {
    if (Math.abs(stop.seconds - s) < Math.abs(best.seconds - s)) best = stop;
  }
  return best.seconds;
}

// ── "How often can it fire" ↔ cooldown seconds ─────────────────────────────
// The standalone watcher's only pacing control: after a trigger fires it waits
// this long before looking for the same thing again. Zero is a real setting, not
// a degenerate one (collecting something that keeps reappearing should go as
// fast as the screen can be read), so it leads the list.
export const REPEAT_STOPS: { seconds: number; label: string; hint: string }[] = [
  { seconds: 0, label: "As fast as it can", hint: "No wait at all, best for collecting things quickly" },
  { seconds: 1, label: "About once a second", hint: "A steady tap rather than a burst" },
  { seconds: 5, label: "Every few seconds", hint: "For something that comes back slowly" },
  { seconds: 30, label: "Once in a while", hint: "Half a minute between repeats" },
];

/** The canonical seconds of the repeat stop nearest a stored cooldown. */
export function repeatStopOf(seconds: number): number {
  const s = seconds ?? 0;
  let best = REPEAT_STOPS[0];
  for (const stop of REPEAT_STOPS) {
    if (Math.abs(stop.seconds - s) < Math.abs(best.seconds - s)) best = stop;
  }
  return best.seconds;
}

/** True when the watch area is the whole screen (the stored "anywhere" default). */
export function isAnywhere(region: number[]): boolean {
  return region.length === 4 && region[0] === 0 && region[1] === 0 && region[2] === 100 && region[3] === 100;
}

/** The untouched HSV range a fresh draft carries: hue 0-179, saturation 0-255,
 *  value 0-255, which is to say every colour there is. It has to be told apart
 *  from a picked colour, because a trigger saved on it matches the entire screen
 *  as one blob and clicks the middle of it. */
export function isUnpickedColor(low: number[], high: number[]): boolean {
  return (
    low.length === 3 &&
    high.length === 3 &&
    low[0] === 0 &&
    low[1] === 0 &&
    low[2] === 0 &&
    high[0] === 179 &&
    high[1] === 255 &&
    high[2] === 255
  );
}

/** A one-line, jargon-free summary of what a trigger does, for list rows. */
export function describeTrigger(d: TriggerDraft): string {
  const look = lookOf(d.method);
  const what =
    look === "color"
      ? "a colour appears"
      : look === "image"
        ? "the picture appears"
        : d.ocr_text.trim()
          ? `“${d.ocr_text.trim()}” appears`
          : "the text appears";
  const where = isAnywhere(d.region) ? "" : " in a set area";
  const act =
    d.action === "key"
      ? `press ${d.key || "a key"}`
      : d.action === "nudge"
        ? "nudge the mouse"
        : "click it";
  return `When ${what}${where}, ${act}.`;
}

/** Is this trigger fully specified enough to save and run? A colour needs one
 *  actually picked, an image needs a captured picture, text needs words to find.
 *
 *  The colour arm used to accept any three-element range, which let a brand-new
 *  draft save on its untouched full-spectrum default. That trigger matches every
 *  pixel on screen as a single blob and clicks its centre, on repeat. The
 *  default now reads as "nothing chosen yet", which is what it is. */
export function isTriggerReady(d: TriggerDraft): boolean {
  if (d.action === "key" && !d.key.trim()) return false;
  const look = lookOf(d.method);
  if (look === "color") {
    return Array.isArray(d.hsv_low) && d.hsv_low.length === 3 && !isUnpickedColor(d.hsv_low, d.hsv_high);
  }
  if (look === "image") return !!d.template_path;
  return d.ocr_text.trim().length > 0;
}

let seq = 0;
function newId(): string {
  const c = (globalThis as { crypto?: Crypto }).crypto;
  if (c && typeof c.randomUUID === "function") return "t-" + c.randomUUID();
  // Deterministic fallback (test/older runtimes): unique within a session.
  seq += 1;
  return "t-" + seq.toString(36) + "-" + PERSISTED_KEYS.length;
}

/** A fresh trigger. Defaults to watching for a Color (the reliable path with a
 *  pick-on-screen picker). The old code defaulted new Watch triggers to an
 *  empty Image, a documented dead end. */
export function newTriggerDraft(name = ""): TriggerDraft {
  return {
    id: newId(),
    name,
    method: "color",
    action: "click",
    key: "",
    hsv_low: [0, 0, 0],
    hsv_high: [179, 255, 255],
    template_path: "",
    ocr_text: "",
    threshold: 0.8,
    region: [0, 0, 100, 100],
    min_area: 40,
    resume_delay: 3,
    cooldown: 2,
    enabled: true,
    click_offset: [],
    click_line: [],
    click_lines: [],
    _extra: {},
    _preview: null,
  };
}

/** A fresh Watch trigger starts on the visual capture path shown first in the
 *  guided Watch workspace, with no wait between repeats. Per-macro guards keep
 *  the steadier colour default and two-second pace. */
export function newWatchDraft(name = ""): TriggerDraft {
  return {
    ...newTriggerDraft(name),
    method: methodFor("image"),
    cooldown: 0,
  };
}

const num = (v: unknown, d: number): number => (typeof v === "number" && !Number.isNaN(v) ? v : d);
const str = (v: unknown, d: string): string => (typeof v === "string" ? v : d);
const arr = <T,>(v: unknown, d: T): T => (Array.isArray(v) ? (v as T) : d);

/** Stored guard → editor draft. Reads each persisted field with a safe default
 *  and shuttles any unknown key into `_extra` so nothing is lost on save. */
export function draftFromGuard(g: Guard): TriggerDraft {
  const known = new Set<string>(PERSISTED_KEYS);
  const _extra: Record<string, unknown> = {};
  for (const k of Object.keys(g)) {
    if (!known.has(k) && !k.startsWith("_")) _extra[k] = g[k];
  }
  return {
    id: str(g.id, "") || newId(),
    name: str(g.name, ""),
    method: str(g.method, "color"),
    action: str(g.action, "click"),
    key: str(g.key, ""),
    hsv_low: arr<number[]>(g.hsv_low, [0, 0, 0]),
    hsv_high: arr<number[]>(g.hsv_high, [179, 255, 255]),
    template_path: str(g.template_path, ""),
    ocr_text: str(g.ocr_text, ""),
    threshold: num(g.threshold, 0.8),
    region: arr<number[]>(g.region, [0, 0, 100, 100]),
    min_area: num(g.min_area, 40),
    resume_delay: num(g.resume_delay, 3),
    cooldown: num(g.cooldown, 2),
    enabled: g.enabled !== false,
    click_offset: arr<number[]>(g.click_offset, []),
    click_line: arr<number[]>(g.click_line, []),
    click_lines: arr<number[][]>(g.click_lines, []),
    _extra,
    _preview: null,
  };
}

/** Editor draft → stored guard. Emits exactly the 18 persisted fields (unknown
 *  keys spread first so a known field always wins). `forTest` drops the surgical
 *  click geometry, matching the old Test payload, so a Test never repositions a
 *  saved click line. */
export function guardFromDraft(d: TriggerDraft, opts?: { forTest?: boolean }): Guard {
  const out: Guard = {
    ...d._extra,
    id: d.id,
    name: d.name || "Trigger",
    method: d.method,
    action: d.action,
    key: d.key.trim(),
    hsv_low: d.hsv_low,
    hsv_high: d.hsv_high,
    template_path: d.template_path || "",
    ocr_text: d.ocr_text || "",
    threshold: d.threshold,
    region: d.region,
    min_area: d.min_area,
    resume_delay: d.resume_delay,
    cooldown: d.cooldown,
    enabled: d.enabled,
  };
  if (!opts?.forTest) {
    out.click_offset = d.click_offset;
    out.click_line = d.click_line;
    out.click_lines = d.click_lines;
  }
  return out;
}
