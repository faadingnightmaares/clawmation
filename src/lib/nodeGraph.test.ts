import { describe, expect, it, vi } from "vitest";

import type { NodeGraph } from "@/api";
import {
  OUTPUTS,
  createGraphNode,
  embedMacroInNode,
  flowToGraph,
  graphToFlow,
  validateGraphClient,
} from "./nodeGraph";

describe("node graph adapters", () => {
  it("uses outcome-based branch labels", () => {
    expect(OUTPUTS.branch).toEqual([
      { id: "true", label: "If works" },
      { id: "false", label: "If fails" },
    ]);
    expect(OUTPUTS.action).toEqual([
      { id: "next", label: "If works" },
      { id: "error", label: "If fails" },
    ]);
  });

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

  it("creates chain nodes with real chain references", () => {
    vi.stubGlobal("crypto", { randomUUID: () => "12345678-0000-0000-0000-000000000000" });
    const node = createGraphNode("chain", { x: 10, y: 20 });
    expect(node.type).toBe("chain");
    expect(node.config.chain_id).toBe("");
    vi.unstubAllGlobals();
  });

  it("creates a real start node for blank canvases", () => {
    vi.stubGlobal("crypto", { randomUUID: () => "12345678-0000-0000-0000-000000000000" });
    const node = createGraphNode("start", { x: 10, y: 20 });
    expect(node.type).toBe("start");
    expect(node.label).toBe("Start");
    expect(node.config).toEqual({});
    vi.unstubAllGlobals();
  });

  it("creates disconnected canvas notes without validation warnings", () => {
    vi.stubGlobal("crypto", { randomUUID: () => "12345678-0000-0000-0000-000000000000" });
    const note = createGraphNode("note", { x: 10, y: 20 });
    const graph: NodeGraph = {
      version: 1,
      name: "demo",
      entry: "start",
      nodes: [
        {
          id: "start",
          type: "start",
          label: "Start",
          enabled: true,
          position: { x: 0, y: 0 },
          config: {},
        },
        note,
      ],
      edges: [],
    };

    expect(note.type).toBe("note");
    expect(note.config.text).toBe("");
    expect(validateGraphClient(graph, [], []).warnings).toEqual([]);
    vi.unstubAllGlobals();
  });

  it("embeds an imported macro as an independent node snapshot", () => {
    const node = createGraphNode("sub_macro", { x: 10, y: 20 });
    const step = {
      id: "click-1",
      type: "click",
      enabled: true,
      label: "Click",
      x: 100,
      y: 200,
      key: "",
      text: "",
      delay: 0,
      scroll_amount: 0,
      detect_mode: "color",
      hsv_low: [0, 0, 0],
      hsv_high: [179, 255, 255],
      template: "",
      region: [0, 0, 100, 100],
      min_area: 40,
      timeout: 10,
      confidence: 0.8,
    };

    const steps = [step];
    const imported = embedMacroInNode(
      node,
      { name: "Farm", events: 42, duration: 9.5, resolution: "1920x1080" },
      steps,
    );

    expect(imported.label).toBe("Farm");
    expect(imported.config.macro_name).toBe("Farm");
    expect(imported.config.embedded_steps).toEqual([step]);
    expect(imported.config.embedded_steps).not.toBe(steps);
    expect(imported.config.repeat).toBe(1);
    expect(imported.config.source_events).toBe(42);
  });

  it("finds incomplete paths and missing references before a round trip", () => {
    const graph: NodeGraph = {
      version: 1,
      name: "demo",
      entry: "start",
      nodes: [
        { id: "start", type: "start", label: "Start", enabled: true, position: { x: 0, y: 0 }, config: {} },
        { id: "branch", type: "branch", label: "Branch", enabled: true, position: { x: 1, y: 0 }, config: { condition: "last_ok" } },
        { id: "chain", type: "chain", label: "Chain", enabled: true, position: { x: 2, y: 0 }, config: { chain_id: "missing" } },
      ],
      edges: [
        { id: "a", from: "start", output: "next", to: "branch" },
        { id: "b", from: "branch", output: "true", to: "chain" },
      ],
    };

    const issues = validateGraphClient(graph, ["demo"], []);

    expect(issues.errors).toContain("Branch “Branch” needs its If fails path.");
    expect(issues.errors).toContain("Chain “Chain” references a chain that no longer exists.");
  });
});
