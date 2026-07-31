import { describe, expect, it, vi } from "vitest";

import type { NodeGraph } from "@/api";
import {
  OUTPUTS,
  createGraphNode,
  effectiveFailureMode,
  embedMacroInNode,
  flowToGraph,
  graphToFlow,
  visibleNodeOutputs,
  validateGraphClient,
} from "./nodeGraph";
import {
  deleteNodeAndReconnect,
  ensureRepeatReturnEdges,
  filterNodePalette,
  insertNodeOnEdge,
  repeatReturnEdge,
  repeatReturnEdgeIds,
  wrapSelectionInRepeat,
} from "./nodeGraphAuthoring";

describe("node graph adapters", () => {
  it("uses contextual outcome labels", () => {
    expect(OUTPUTS.branch).toEqual([
      { id: "true", label: "Matches" },
      { id: "false", label: "Otherwise" },
    ]);
    expect(OUTPUTS.action).toEqual([
      { id: "next", label: "Continue" },
      { id: "error", label: "On failure" },
    ]);
    expect(OUTPUTS.loop).toEqual([
      { id: "body", label: "Do" },
      { id: "done", label: "Then" },
    ]);
  });

  it("keeps recovery ports hidden unless recovery is configured or already wired", () => {
    const action = createGraphNode("click", { x: 0, y: 0 });
    expect(effectiveFailureMode(action, [])).toBe("stop");
    expect(visibleNodeOutputs(action, [])).toEqual([{ id: "next", label: "Continue" }]);

    const errorEdge = {
      id: "recover",
      from: action.id,
      output: "error",
      to: "stop",
    };
    expect(effectiveFailureMode(action, [errorEdge])).toBe("recovery");
    expect(visibleNodeOutputs(action, [errorEdge])).toEqual(OUTPUTS.action);

    action.config.failure_mode = "continue";
    expect(effectiveFailureMode(action, [errorEdge])).toBe("continue");
    action.config.failure_mode = "recovery";
    expect(visibleNodeOutputs(action, [])).toEqual(OUTPUTS.action);
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

  it("round-trips optional edge reroute points without changing old edges", () => {
    const graph: NodeGraph = {
      version: 1,
      name: "demo",
      entry: "start",
      nodes: [
        { id: "start", type: "start", label: "Start", enabled: true, position: { x: 0, y: 0 }, config: {} },
        { id: "stop", type: "stop", label: "Stop", enabled: true, position: { x: 400, y: 0 }, config: { success: true } },
      ],
      edges: [
        {
          id: "edge",
          from: "start",
          output: "next",
          to: "stop",
          waypoints: [{ x: 180, y: 90 }],
        },
      ],
    };

    const flow = graphToFlow(graph);
    expect(flow.edges[0].data?.waypoints).toEqual([{ x: 180, y: 90 }]);
    expect(flowToGraph("demo", "start", flow.nodes, flow.edges).edges[0]).toEqual(
      graph.edges[0],
    );
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
      templates: [],
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

    expect(issues.errors).toContain("Branch “Branch” needs its Otherwise path.");
    expect(issues.errors).toContain("Chain “Chain” references a chain that no longer exists.");
  });

  it("allows a Repeat to finish the workflow without a Then path", () => {
    const graph: NodeGraph = {
      version: 1,
      name: "terminal repeat",
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
        {
          id: "repeat",
          type: "loop",
          label: "Repeat",
          enabled: true,
          position: { x: 200, y: 0 },
          config: { count: 3 },
        },
      ],
      edges: [
        { id: "enter", from: "start", output: "next", to: "repeat" },
        { id: "body", from: "repeat", output: "body", to: "repeat" },
      ],
    };

    expect(validateGraphClient(graph, [], []).errors).toEqual([]);
  });
});

describe("node graph authoring helpers", () => {
  it("ranks fuzzy palette matches and can filter out Start", () => {
    const results = filterNodePalette("fclick", { allowStart: false });
    expect(results[0]?.kind).toBe("find_click");
    expect(results.some((item) => item.kind === "start")).toBe(false);
  });

  it("inserts a compatible node on an existing edge atomically", () => {
    const inserted = createGraphNode("delay", { x: 180, y: 0 });
    const result = insertNodeOnEdge(
      [createGraphNode("start", { x: 0, y: 0 }), createGraphNode("stop", { x: 360, y: 0 })]
        .map((node, index) => ({ ...node, id: index === 0 ? "start" : "stop" })),
      [{ id: "edge", from: "start", output: "next", to: "stop" }],
      "edge",
      inserted,
    );

    expect(result?.nodes).toContainEqual(inserted);
    expect(result?.edges).toEqual([
      expect.objectContaining({ from: "start", output: "next", to: inserted.id }),
      expect.objectContaining({ from: inserted.id, output: "next", to: "stop" }),
    ]);
  });

  it("deletes a one-in one-out node and reconnects the surrounding flow", () => {
    const result = deleteNodeAndReconnect(
      [
        { ...createGraphNode("start", { x: 0, y: 0 }), id: "start" },
        { ...createGraphNode("delay", { x: 180, y: 0 }), id: "wait" },
        { ...createGraphNode("stop", { x: 360, y: 0 }), id: "stop" },
      ],
      [
        { id: "before", from: "start", output: "next", to: "wait" },
        { id: "after", from: "wait", output: "next", to: "stop" },
      ],
      "wait",
    );

    expect(result?.nodes.map((node) => node.id)).toEqual(["start", "stop"]);
    expect(result?.edges).toEqual([
      expect.objectContaining({ from: "start", output: "next", to: "stop" }),
    ]);
  });

  it("wraps one continuous selection in a Repeat with a compact return edge", () => {
    const nodes = [
      { ...createGraphNode("start", { x: 0, y: 0 }), id: "start" },
      { ...createGraphNode("click", { x: 180, y: 0 }), id: "click" },
      { ...createGraphNode("delay", { x: 360, y: 0 }), id: "wait" },
      { ...createGraphNode("loop", { x: 120, y: 160 }), id: "repeat" },
      { ...createGraphNode("stop", { x: 540, y: 0 }), id: "stop" },
    ];
    const result = wrapSelectionInRepeat(
      nodes,
      [
        { id: "a", from: "start", output: "next", to: "click" },
        { id: "b", from: "click", output: "next", to: "wait" },
        { id: "c", from: "wait", output: "next", to: "stop" },
      ],
      "repeat",
      ["click", "wait"],
    );

    expect(result?.edges).toEqual(
      expect.arrayContaining([
        expect.objectContaining({ from: "start", output: "next", to: "repeat" }),
        expect.objectContaining({ from: "repeat", output: "body", to: "click" }),
        expect.objectContaining({ from: "wait", output: "next", to: "repeat" }),
        expect.objectContaining({ from: "repeat", output: "done", to: "stop" }),
      ]),
    );
    expect(repeatReturnEdge(result!.nodes, result!.edges, "repeat")).toEqual(
      expect.objectContaining({ from: "wait", to: "repeat" }),
    );
    const railIds = repeatReturnEdgeIds(result!.nodes, result!.edges);
    expect(
      result!.edges.filter((edge) => railIds.has(edge.id)),
    ).toEqual([
      expect.objectContaining({ from: "wait", to: "repeat" }),
    ]);
  });

  it("automatically closes a simple Repeat body and stays idempotent", () => {
    const nodes = [
      { ...createGraphNode("start", { x: 0, y: 0 }), id: "start" },
      { ...createGraphNode("loop", { x: 180, y: 0 }), id: "repeat" },
      { ...createGraphNode("wait_for", { x: 360, y: 0 }), id: "vision" },
    ];
    const edges = [
      { id: "enter", from: "start", output: "next", to: "repeat" },
      { id: "body", from: "repeat", output: "body", to: "vision" },
    ];

    const repaired = ensureRepeatReturnEdges(nodes, edges);
    expect(repaired).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          from: "vision",
          output: "found",
          to: "repeat",
        }),
      ]),
    );
    expect(ensureRepeatReturnEdges(nodes, repaired)).toEqual(repaired);
  });
});
