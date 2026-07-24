import { describe, it, expect } from "vitest";

import { fmtDur, fmtAgo, repsFor, repeatToIndex, fmtHotkey, hsvToCss, STOPS } from "./format";

describe("fmtDur", () => {
  it("formats minutes and zero-padded seconds", () => {
    expect(fmtDur(0)).toBe("0:00");
    expect(fmtDur(9)).toBe("0:09");
    expect(fmtDur(65)).toBe("1:05");
    expect(fmtDur(600)).toBe("10:00");
  });
});

describe("fmtAgo", () => {
  it("returns empty for a falsy epoch", () => {
    expect(fmtAgo(0)).toBe("");
  });
  it("buckets by elapsed time", () => {
    const now = Math.floor(Date.now() / 1000);
    expect(fmtAgo(now - 5)).toBe("just now");
    expect(fmtAgo(now - 120)).toBe("2m ago");
    expect(fmtAgo(now - 2 * 3600)).toBe("2h ago");
    expect(fmtAgo(now - 3 * 86400)).toBe("3d ago");
  });
});

describe("repsFor / repeatToIndex", () => {
  it("maps a slider index to a rep count", () => {
    expect(repsFor(0)).toBe(0); // ∞
    expect(repsFor(1)).toBe(1);
    expect(repsFor(5)).toBe(10);
  });
  it("inverts a saved loop setting to a slider index", () => {
    expect(repeatToIndex(true, 0)).toBe(0); // infinite
    expect(repeatToIndex(false, 1)).toBe(1); // play once
    expect(repeatToIndex(true, 5)).toBe(4); // STOPS index of "5"
    expect(repeatToIndex(true, 7)).toBe(1); // unknown count → play once
  });
  it("round-trips every finite stop", () => {
    STOPS.forEach((_, i) => {
      if (i === 0) expect(repeatToIndex(true, 0)).toBe(0);
      else expect(repeatToIndex(true, repsFor(i))).toBe(i);
    });
  });
});

describe("fmtHotkey", () => {
  it("title-cases modifiers and upper-cases single keys", () => {
    expect(fmtHotkey("")).toBe("");
    expect(fmtHotkey("f6")).toBe("F6");
    expect(fmtHotkey("ctrl+r")).toBe("Ctrl+R");
    expect(fmtHotkey("ctrl+shift+f4")).toBe("Ctrl+Shift+F4");
  });
});

describe("hsvToCss", () => {
  it("maps an HSV midpoint to hsl()", () => {
    expect(hsvToCss([0, 0, 0], [179, 255, 255])).toBe("hsl(179, 50%, 50%)");
  });
  it("returns transparent when either bound is missing", () => {
    expect(hsvToCss(undefined, undefined)).toBe("transparent");
  });
});
