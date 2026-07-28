import { describe, expect, it } from "vitest";
import {
  H,
  INDICATOR_COLORS,
  W,
  counterDigits,
  indicatorColor,
  renderCat,
  type Frame,
} from "./cat";

interface DrawCall {
  color: string;
  x: number;
  y: number;
  width: number;
  height: number;
}

function recordingContext() {
  const fills: DrawCall[] = [];
  const clears: Omit<DrawCall, "color">[] = [];
  const context = {
    fillStyle: "",
    fillRect(x: number, y: number, width: number, height: number) {
      fills.push({ color: String(this.fillStyle), x, y, width, height });
    },
    clearRect(x: number, y: number, width: number, height: number) {
      clears.push({ x, y, width, height });
    },
  };
  return {
    ctx: context as unknown as CanvasRenderingContext2D,
    fills,
    clears,
  };
}

const baseFrame: Frame = {
  mode: "recording",
  elapsed: 0,
  blinkOn: true,
  phase: 0,
};

describe("hanging cat indicator", () => {
  it("keeps one elapsed digit in each eye and safely wraps at 100", () => {
    expect(counterDigits(0)).toEqual(["0", "0"]);
    expect(counterDigits(9.9)).toEqual(["0", "9"]);
    expect(counterDigits(42)).toEqual(["4", "2"]);
    expect(counterDigits(107)).toEqual(["0", "7"]);
    expect(counterDigits(-12)).toEqual(["0", "0"]);
    expect(counterDigits(Number.NaN)).toEqual(["0", "0"]);
  });

  it("preserves a distinct counter color for every active state", () => {
    expect(indicatorColor("recording")).toBe(INDICATOR_COLORS.recording);
    expect(indicatorColor("playing")).toBe(INDICATOR_COLORS.playing);
    expect(indicatorColor("paused")).toBe(INDICATOR_COLORS.paused);
    expect(new Set(Object.values(INDICATOR_COLORS))).toHaveLength(3);
  });

  it("renders the compact canvas and deliberately clips both grips at the top", () => {
    const { ctx, fills, clears } = recordingContext();
    renderCat(ctx, baseFrame);

    expect([W, H]).toEqual([96, 88]);
    expect(clears).toEqual([{ x: 0, y: 0, width: W, height: H }]);
    expect(fills.some((call) => call.y < 0)).toBe(true);
    expect(fills.some((call) => call.color === INDICATOR_COLORS.recording)).toBe(true);
  });

  it.each(["recording", "playing", "paused"])("draws the %s state color", (mode) => {
    const { ctx, fills } = recordingContext();
    renderCat(ctx, { ...baseFrame, mode });
    expect(fills.some((call) => call.color === indicatorColor(mode))).toBe(true);
  });
});
