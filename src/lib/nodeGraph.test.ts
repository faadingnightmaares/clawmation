import { describe, expect, it, vi } from "vitest";

import type { NodeGraph } from "@/api";
import { createGraphNode, flowToGraph, graphToFlow } from "./nodeGraph";

describe("node graph adapters", () => {
  it("round-trips graph ports and positions", () => {
    const graph: NodeGraph = {
      version: 1,
      name: "demo",
      entry: "start",
      nodes: [
        { id: "start", type: "start", label: "Start", enabled: true, position: { x: 1, y: 2 }, config: {} },
        { id: "stop", type: "stop", label: "Stop", enabled: true, position: { x: 3, y: 4 }, config: { success: true } },
      ],
      edges: [{ id: "edge", from: "start", output: "next", to: "stop" }],
    };
    const flow = graphToFlow(graph);
    flow.nodes[1].position = { x: 30, y: 40 };
    const result = flowToGraph("demo", "start", flow.nodes, flow.edges);
    expect(result.edges[0]).toEqual(graph.edges[0]);
    expect(result.nodes[1].position).toEqual({ x: 30, y: 40 });
  });

  it("creates complete step-backed nodes", () => {
    vi.stubGlobal("crypto", { randomUUID: () => "12345678-0000-0000-0000-000000000000" });
    const node = createGraphNode("wait_for", { x: 10, y: 20 });
    expect(node.type).toBe("vision");
    expect((node.config.step as { type: string }).type).toBe("wait_for");
    expect((node.config.step as { timeout: number }).timeout).toBe(10);
    vi.unstubAllGlobals();
  });
});
