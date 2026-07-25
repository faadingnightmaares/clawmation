import { describe, expect, it } from "vitest";
import { accelCaps, accelFromEvent, isModifierOnly, type KeyLike } from "./hotkeys";

// The strings here have to survive a round trip through Rust's
// `Shortcut::from_str` (global-hotkey), which only knows a fixed vocabulary and
// fails silently. These cases pin the names that parser accepts.
function press(code: string, mods: Partial<KeyLike> = {}): KeyLike {
  return { code, ctrlKey: false, altKey: false, shiftKey: false, metaKey: false, ...mods };
}

describe("accelFromEvent", () => {
  it("names plain keys the way the backend parser does", () => {
    expect(accelFromEvent(press("F6"))).toBe("f6");
    expect(accelFromEvent(press("KeyR"))).toBe("r");
    expect(accelFromEvent(press("Digit4"))).toBe("4");
    expect(accelFromEvent(press("Escape"))).toBe("esc");
    expect(accelFromEvent(press("ArrowUp"))).toBe("up");
    expect(accelFromEvent(press("Numpad7"))).toBe("num7");
    expect(accelFromEvent(press("BracketLeft"))).toBe("bracketleft");
  });

  it("orders modifiers so one chord always stores one string", () => {
    const chord = { ctrlKey: true, shiftKey: true, altKey: true, metaKey: true };
    expect(accelFromEvent(press("KeyP", chord))).toBe("ctrl+alt+shift+super+p");
    expect(accelFromEvent(press("KeyP", { shiftKey: true, ctrlKey: true }))).toBe("ctrl+shift+p");
  });

  it("refuses keys the backend can't bind instead of storing a dead shortcut", () => {
    expect(accelFromEvent(press("ControlLeft", { ctrlKey: true }))).toBeNull();
    expect(accelFromEvent(press("F25"))).toBeNull();
    expect(accelFromEvent(press("IntlBackslash"))).toBeNull();
    expect(accelFromEvent(press("LaunchMail"))).toBeNull();
  });

  it("knows which codes are modifiers", () => {
    expect(isModifierOnly("ShiftRight")).toBe(true);
    expect(isModifierOnly("KeyA")).toBe(false);
  });
});

describe("accelCaps", () => {
  it("splits a stored shortcut into caps", () => {
    expect(accelCaps("ctrl+shift+r")).toEqual(["Ctrl", "Shift", "R"]);
    expect(accelCaps("f12")).toEqual(["F12"]);
    expect(accelCaps("super+up")).toEqual(["Win", "↑"]);
  });

  it("shows a hand-edited value as-is rather than hiding it", () => {
    expect(accelCaps("")).toEqual([]);
    expect(accelCaps("mystery")).toEqual(["mystery"]);
  });
});
