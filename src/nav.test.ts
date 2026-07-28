import { describe, expect, it } from "vitest";
import { Repeat } from "iconoir-react";

import { NAV, PRIMARY_VIEWS } from "./nav";

describe("main navigation", () => {
  it("keeps Loops as the rightmost working surface", () => {
    expect(PRIMARY_VIEWS.map((view) => view.id)).toEqual([
      "dashboard",
      "macros",
      "vision",
      "nodes",
    ]);
    expect(NAV.find((view) => view.id === "nodes")).toMatchObject({
      label: "Loops",
      Icon: Repeat,
    });
    expect(NAV.find((view) => view.id === "nodes")?.disabled).toBeUndefined();
    expect(NAV.find((view) => view.id === "nodes")?.badge).toBeUndefined();
  });
});
