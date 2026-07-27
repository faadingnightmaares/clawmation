import { describe, expect, it } from "vitest";

import { NAV, PRIMARY_VIEWS } from "./nav";

describe("main navigation", () => {
  it("keeps Nodes visible but unavailable until it is ready", () => {
    expect(PRIMARY_VIEWS.map((view) => view.id)).toEqual([
      "dashboard",
      "macros",
      "vision",
      "autopilot",
      "nodes",
    ]);
    expect(NAV.find((view) => view.id === "nodes")).toMatchObject({
      label: "Nodes",
      disabled: true,
      badge: "Soon",
    });
  });
});
