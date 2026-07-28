import { describe, expect, it } from "vitest";
import { type Guard } from "@/api";
import {
  accuracyOf,
  describeTrigger,
  draftFromGuard,
  guardFromDraft,
  isAnywhere,
  isTriggerReady,
  lookOf,
  newTriggerDraft,
  newWatchDraft,
  repeatStopOf,
  resumeStopOf,
} from "./triggers";

// The exact 18-field shape both the old MacrosView and VisionView wrote on save.
// tsc cannot police these field names (Guard has a string index signature), so
// this file is the real guard: it proves the converters preserve every field.
function storedGuard(over: Partial<Guard> = {}): Guard {
  return {
    id: "g1700000000000",
    name: "Reconnect",
    method: "color",
    action: "click",
    key: "",
    hsv_low: [10, 120, 90],
    hsv_high: [24, 255, 255],
    template_path: "",
    ocr_text: "",
    threshold: 0.75,
    region: [40, 60, 60, 80],
    min_area: 40,
    resume_delay: 10,
    cooldown: 2,
    enabled: true,
    click_offset: [],
    click_line: [],
    click_lines: [],
    ...over,
  };
}

const PERSISTED = [
  "id", "name", "method", "action", "key", "hsv_low", "hsv_high",
  "template_path", "ocr_text", "threshold", "region", "min_area",
  "resume_delay", "cooldown", "enabled", "click_offset", "click_line", "click_lines",
].sort();

describe("guard ↔ draft round-trip", () => {
  it("preserves a colour guard exactly, non-canonical threshold and all", () => {
    const g = storedGuard();
    expect(guardFromDraft(draftFromGuard(g))).toEqual(g);
  });

  it("preserves an image (template) guard exactly", () => {
    const g = storedGuard({
      method: "template",
      template_path: "C:/templates/reconnect.png",
      threshold: 0.9,
      hsv_low: [0, 0, 0],
      hsv_high: [179, 255, 255],
    });
    expect(guardFromDraft(draftFromGuard(g))).toEqual(g);
  });

  it("preserves an OCR (text) guard exactly", () => {
    const g = storedGuard({ method: "ocr", ocr_text: "Reconnect", action: "key", key: "space" });
    expect(guardFromDraft(draftFromGuard(g))).toEqual(g);
  });

  it("preserves a nudge trigger exactly", () => {
    const g = storedGuard({ action: "nudge" });
    expect(guardFromDraft(draftFromGuard(g))).toEqual(g);
  });

  it("preserves a surgical click line and offset", () => {
    const g = storedGuard({
      method: "template",
      template_path: "x.png",
      click_offset: [4, -2],
      click_line: [100, 200, 140, 200],
      click_lines: [[100, 200, 140, 200], [10, 10, 20, 20]],
    });
    expect(guardFromDraft(draftFromGuard(g))).toEqual(g);
  });

  it("does NOT force enabled back to true (fixes the old re-save quirk)", () => {
    const g = storedGuard({ enabled: false });
    expect(guardFromDraft(draftFromGuard(g)).enabled).toBe(false);
  });

  it("shuttles unknown stored keys through _extra and back", () => {
    const g = storedGuard({ note: "hand-edited", weight: 7 } as Partial<Guard>);
    const d = draftFromGuard(g);
    expect(d._extra).toEqual({ note: "hand-edited", weight: 7 });
    expect(guardFromDraft(d)).toEqual(g);
  });

  it("emits exactly the persisted key set: no macro_sequence, no transient leakage", () => {
    const out = guardFromDraft(draftFromGuard(storedGuard()));
    expect(Object.keys(out).sort()).toEqual(PERSISTED);
  });

  it("a known field always wins over a same-named _extra key", () => {
    const d = draftFromGuard(storedGuard());
    d._extra = { name: "SHOULD NOT WIN", method: "template" };
    const out = guardFromDraft(d);
    expect(out.name).toBe("Reconnect");
    expect(out.method).toBe("color");
  });

  it("forTest drops the surgical click geometry", () => {
    const d = draftFromGuard(storedGuard({ click_line: [1, 2, 3, 4] }));
    const out = guardFromDraft(d, { forTest: true });
    expect("click_line" in out).toBe(false);
    expect("click_offset" in out).toBe(false);
    expect("click_lines" in out).toBe(false);
    expect(out.method).toBe("color");
  });
});

describe("plain-language helpers", () => {
  it("maps persisted method to a look and back", () => {
    expect(lookOf("color")).toBe("color");
    expect(lookOf("template")).toBe("image");
    expect(lookOf("ocr")).toBe("text");
    expect(lookOf("text")).toBe("text"); // legacy alias tolerated
    expect(lookOf("garbage")).toBe("color");
  });

  it("buckets a threshold into an accuracy stop", () => {
    expect(accuracyOf(0.62)).toBe("loose");
    expect(accuracyOf(0.8)).toBe("balanced");
    expect(accuracyOf(0.9)).toBe("strict");
    expect(accuracyOf(0.95)).toBe("strict");
  });

  it("snaps a resume delay to the nearest labelled stop", () => {
    expect(resumeStopOf(0)).toBe(0);
    expect(resumeStopOf(3)).toBe(3);
    expect(resumeStopOf(10)).toBe(12);
    expect(resumeStopOf(5)).toBe(6);
  });

  it("snaps a cooldown to the nearest repeat stop", () => {
    expect(repeatStopOf(0)).toBe(0);
    expect(repeatStopOf(1)).toBe(1);
    expect(repeatStopOf(2)).toBe(1);
    expect(repeatStopOf(4)).toBe(5);
    expect(repeatStopOf(60)).toBe(30);
  });

  it("a fresh Watch trigger waits for nothing between repeats", () => {
    // The watcher exists to keep pressing a thing the moment it returns…
    expect(newWatchDraft().cooldown).toBe(0);
    // …while a guard firing mid-playback stays on the steadier default.
    expect(newTriggerDraft().cooldown).toBe(2);
  });

  it("judges readiness by look", () => {
    // An untouched draft carries the whole colour spectrum, which matches every
    // pixel on screen as one blob. That is "nothing picked yet", not a trigger.
    expect(isTriggerReady(newTriggerDraft())).toBe(false);
    const picked = { ...newTriggerDraft(), hsv_low: [20, 80, 80], hsv_high: [40, 255, 255] };
    expect(isTriggerReady(picked)).toBe(true);
    const img = { ...newTriggerDraft(), method: "template", template_path: "" };
    expect(isTriggerReady(img)).toBe(false);
    expect(isTriggerReady({ ...img, template_path: "x.png" })).toBe(true);
    const txt = { ...newTriggerDraft(), method: "ocr", ocr_text: "  " };
    expect(isTriggerReady(txt)).toBe(false);
    expect(isTriggerReady({ ...txt, ocr_text: "Play" })).toBe(true);
  });

  it("requires the selected key instead of silently turning Press into Click", () => {
    const press = {
      ...newTriggerDraft(),
      method: "template",
      template_path: "button.png",
      action: "key",
      key: "   ",
    };
    expect(isTriggerReady(press)).toBe(false);
    expect(isTriggerReady({ ...press, key: " space " })).toBe(true);
    expect(guardFromDraft({ ...press, key: " space " }).key).toBe("space");
  });

  it("mints unique ids for fresh drafts", () => {
    expect(newTriggerDraft().id).not.toBe(newTriggerDraft().id);
  });

  it("recognises the whole-screen region", () => {
    expect(isAnywhere([0, 0, 100, 100])).toBe(true);
    expect(isAnywhere([10, 10, 90, 90])).toBe(false);
    expect(isAnywhere([0, 0, 100])).toBe(false);
  });

  it("describes each trigger kind in plain words", () => {
    const base = newTriggerDraft("x");
    expect(describeTrigger(base)).toBe("When a colour appears, click it.");
    expect(describeTrigger({ ...base, action: "nudge" })).toBe("When a colour appears, nudge the mouse.");
    expect(describeTrigger({ ...base, method: "ocr", ocr_text: "Reconnect", action: "key", key: "space" })).toBe(
      "When “Reconnect” appears, press space.",
    );
    expect(describeTrigger({ ...base, method: "template", region: [10, 10, 40, 40] })).toBe(
      "When the picture appears in a set area, click it.",
    );
  });
});
