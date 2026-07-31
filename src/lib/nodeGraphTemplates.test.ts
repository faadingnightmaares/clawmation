import { describe, expect, it } from "vitest";

import { validateGraphClient } from "./nodeGraph";
import {
  LOOP_TEMPLATES,
  createLoopTemplateGraph,
} from "./nodeGraphTemplates";

describe("Loop templates", () => {
  it.each(LOOP_TEMPLATES)(
    "builds $name with valid references and no client errors",
    ({ id }) => {
      const graph = createLoopTemplateGraph(id, "Generated Loop");
      const nodeIds = new Set(graph.nodes.map((node) => node.id));

      expect(graph.name).toBe("Generated Loop");
      expect(nodeIds.has(graph.entry)).toBe(true);
      expect(
        graph.edges.every(
          (edge) => nodeIds.has(edge.from) && nodeIds.has(edge.to),
        ),
      ).toBe(true);
      expect(validateGraphClient(graph, [], []).errors).toEqual([]);
    },
  );

  it("includes visible guidance in the Learn Loops graph", () => {
    const graph = createLoopTemplateGraph("learn-loops", "Learn Loops");

    expect(
      graph.nodes.some(
        (node) =>
          node.type === "note" &&
          node.label.includes("Right-click"),
      ),
    ).toBe(true);
    expect(graph.nodes.some((node) => node.type === "branch")).toBe(true);
    expect(
      graph.nodes.filter((node) => node.type === "stop"),
    ).toHaveLength(2);
  });
});
