import { describe, expect, it } from "vitest";

import { orthogonalPoints, roundedOrthogonalPath } from "./ProCanvasEdge";

describe("ProCanvasEdge routing", () => {
  it("builds a stable orthogonal route without waypoints", () => {
    const points = orthogonalPoints(
      { x: 0, y: 20 },
      { x: 300, y: 140 },
      [],
    );
    expect(points).toEqual([
      { x: 0, y: 20 },
      { x: 30, y: 20 },
      { x: 150, y: 20 },
      { x: 150, y: 140 },
      { x: 270, y: 140 },
      { x: 300, y: 140 },
    ]);
    expect(roundedOrthogonalPath(points)).toMatch(/^M 0 20/);
    expect(roundedOrthogonalPath(points)).toContain("Q");
  });

  it("routes through persisted reroute points", () => {
    const points = orthogonalPoints(
      { x: -100, y: 40 },
      { x: 300, y: 140 },
      [{ x: 80, y: -20 }],
    );
    expect(points).toContainEqual({ x: 80, y: -20 });
    expect(points.every((point) => Number.isFinite(point.x) && Number.isFinite(point.y))).toBe(true);
  });
});
