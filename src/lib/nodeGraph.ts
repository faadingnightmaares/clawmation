import type { Edge, Node } from "@xyflow/react";

import type { Chain, GraphEdge, GraphNode, GraphNodeType, NodeGraph, Step } from "@/api";

export interface MacroNodeData extends Record<string, unknown> {
  graphNode: GraphNode;
  invalid?: boolean;
}

export type MacroFlowNode = Node<MacroNodeData, "macro">;

export const OUTPUTS: Record<GraphNodeType, { id: string; label: string }[]> = {
  start: [{ id: "next", label: "Next" }],
  action: [
    { id: "next", label: "If works" },
    { id: "error", label: "If fails" },
  ],
  vision: [
    { id: "found", label: "Found" },
    { id: "missing", label: "Not found" },
  ],
  branch: [
    { id: "true", label: "If works" },
    { id: "false", label: "If fails" },
  ],
  loop: [
    { id: "body", label: "Loop" },
    { id: "done", label: "Done" },
  ],
  sub_macro: [
    { id: "success", label: "If works" },
    { id: "error", label: "If fails" },
  ],
  chain: [
    { id: "success", label: "If works" },
    { id: "error", label: "If fails" },
  ],
  note: [],
  stop: [],
};

export const REQUIRED_OUTPUTS: Partial<Record<GraphNodeType, string[]>> = {
  start: ["next"],
  branch: ["true", "false"],
  loop: ["body", "done"],
};

export function shouldLabelOutput(type: GraphNodeType, output: string): boolean {
  if (output === "next") return false;
  return !(["sub_macro", "chain"].includes(type) && output === "success");
}

const blankStep = (type: string): Step => ({
  id: crypto.randomUUID().slice(0, 8),
  type,
  enabled: true,
  label: "",
  x: 0,
  y: 0,
  key: "",
  text: "",
  delay: type === "delay" ? 1 : 0,
  scroll_amount: type === "scroll" ? 3 : 0,
  detect_mode: ["find_click", "wait_for"].includes(type) ? "template" : "color",
  hsv_low: [0, 0, 0],
  hsv_high: [179, 255, 255],
  template: "",
  region: [0, 0, 100, 100],
  min_area: 40,
  timeout: 10,
  confidence: 0.8,
});

export function createGraphNode(kind: string, position: { x: number; y: number }): GraphNode {
  const id = `node-${crypto.randomUUID().slice(0, 8)}`;
  const stepKind = ["click", "key", "type", "scroll", "delay", "find_click", "wait_for"].includes(kind)
    ? kind
    : null;
  if (stepKind) {
    const vision = stepKind === "find_click" || stepKind === "wait_for";
    const labels: Record<string, string> = {
      click: "Click",
      key: "Press key",
      type: "Type text",
      scroll: "Scroll",
      delay: "Wait",
      find_click: "Find image & click",
      wait_for: "Wait for image",
    };
    const step = blankStep(stepKind);
    step.label = labels[stepKind];
    return {
      id,
      type: vision ? "vision" : "action",
      label: labels[stepKind],
      position,
      enabled: true,
      config: { step },
    };
  }
  const types: Record<string, GraphNodeType> = {
    start: "start",
    branch: "branch",
    loop: "loop",
    sub_macro: "sub_macro",
    chain: "chain",
    note: "note",
    stop: "stop",
  };
  const nodeType = types[kind] ?? "action";
  const labels: Record<GraphNodeType, string> = {
    start: "Start",
    action: "Action",
    vision: "Vision",
    branch: "Branch",
    loop: "Loop",
    sub_macro: "Run macro",
    chain: "Run chain",
    note: "Note",
    stop: "Finish",
  };
  const config: Record<string, unknown> =
    nodeType === "branch"
      ? { condition: "last_ok" }
      : nodeType === "loop"
        ? { count: 3 }
        : nodeType === "sub_macro"
          ? { macro_name: "", embedded_steps: [], repeat: 1 }
          : nodeType === "chain"
            ? { chain_id: "" }
          : nodeType === "note"
            ? { text: "" }
          : nodeType === "stop"
            ? { success: true }
            : {};
  return { id, type: nodeType, label: labels[nodeType], position, enabled: true, config };
}

export interface ClientGraphIssues {
  errors: string[];
  warnings: string[];
  nodeIds: Set<string>;
}

export interface MacroSnapshotSource {
  name: string;
  events: number;
  duration?: number;
  resolution?: string;
}

export function embedMacroInNode(
  node: GraphNode,
  source: MacroSnapshotSource,
  steps: Step[],
): GraphNode {
  const embeddedSteps = steps.map((step) => ({
    ...step,
    hsv_low: [...step.hsv_low],
    hsv_high: [...step.hsv_high],
    region: [...step.region],
  }));
  const repeat = Number(node.config.repeat ?? 1);
  return {
    ...node,
    label: source.name,
    config: {
      ...node.config,
      macro_name: source.name,
      embedded_steps: embeddedSteps,
      repeat: Number.isInteger(repeat) && repeat >= 1 ? repeat : 1,
      source_events: source.events,
      source_duration: source.duration ?? 0,
      source_resolution: source.resolution ?? "",
    },
  };
}

export function validateGraphClient(
  graph: NodeGraph,
  macroNames: string[],
  chains: Chain[],
): ClientGraphIssues {
  const errors: string[] = [];
  const warnings: string[] = [];
  const nodeIds = new Set<string>();
  const macroSet = new Set(macroNames);
  const chainSet = new Set(chains.map((chain) => chain.id).filter(Boolean));
  const edgesByOutput = new Map<string, GraphEdge>();
  const incoming = new Set<string>();

  for (const edge of graph.edges) {
    edgesByOutput.set(`${edge.from}:${edge.output}`, edge);
    incoming.add(edge.to);
  }

  const startNodes = graph.nodes.filter((node) => node.type === "start");
  if (startNodes.length !== 1) {
    errors.push("Graph needs exactly one Start node.");
    for (const node of startNodes) nodeIds.add(node.id);
  } else if (graph.entry !== startNodes[0].id) {
    errors.push("The graph entry must be the Start node.");
    nodeIds.add(startNodes[0].id);
  }

  for (const node of graph.nodes) {
    const required = REQUIRED_OUTPUTS[node.type] ?? [];
    for (const output of required) {
      if (!edgesByOutput.has(`${node.id}:${output}`)) {
        const outputLabel = OUTPUTS[node.type].find((item) => item.id === output)?.label ?? output;
        errors.push(`${node.label || node.type} “${node.label || node.id}” needs its ${outputLabel} path.`);
        nodeIds.add(node.id);
      }
    }

    if (node.type === "sub_macro") {
      const name = String(node.config.macro_name ?? "").trim();
      const embedded = Array.isArray(node.config.embedded_steps)
        ? node.config.embedded_steps
        : [];
      const repeat = Number(node.config.repeat ?? 1);
      if (
        !Number.isInteger(repeat) ||
        repeat < 1 ||
        repeat > 1000
      ) {
        errors.push(`Macro “${node.label || node.id}” repeat must be between 1 and 1000.`);
        nodeIds.add(node.id);
      }
      if (!name || (embedded.length === 0 && !macroSet.has(name))) {
        errors.push(
          `Macro “${node.label || node.id}” references a macro that no longer exists.`,
        );
        nodeIds.add(node.id);
      }
    }
    if (node.type === "vision") {
      const step = node.config.step as Step | undefined;
      if (step?.detect_mode === "template" && !step.template?.trim()) {
        warnings.push(`Vision “${node.label || node.id}” is waiting for an image.`);
      }
    }
    if (node.type === "chain") {
      const chainId = String(node.config.chain_id ?? "").trim();
      if (!chainId || !chainSet.has(chainId)) {
        errors.push(`Chain “${node.label || node.id}” references a chain that no longer exists.`);
        nodeIds.add(node.id);
      }
    }
    if (node.type !== "note" && node.id !== graph.entry && !incoming.has(node.id)) {
      warnings.push(`${node.label || node.id} is not reachable.`);
      nodeIds.add(node.id);
    }
  }

  return { errors, warnings, nodeIds };
}

export function graphToFlow(graph: NodeGraph): { nodes: MacroFlowNode[]; edges: Edge[] } {
  const nodesById = new Map(graph.nodes.map((node) => [node.id, node]));
  return {
    nodes: graph.nodes.map((node) => ({
      id: node.id,
      type: "macro",
      position: node.position,
      data: { graphNode: node },
    })),
    edges: graph.edges.map((edge) => {
      const source = nodesById.get(edge.from);
      const outputLabel = source
        ? OUTPUTS[source.type].find((output) => output.id === edge.output)?.label
        : undefined;
      return {
        id: edge.id,
        source: edge.from,
        sourceHandle: edge.output,
        target: edge.to,
        targetHandle: "in",
        type: "bezier",
        label:
          source && shouldLabelOutput(source.type, edge.output)
            ? outputLabel || edge.output
            : undefined,
        labelStyle: {
          fill: "var(--muted-foreground)",
          fontSize: 10,
          fontWeight: 600,
        },
        labelBgStyle: {
          fill: "var(--background)",
          fillOpacity: 0.9,
        },
        labelBgPadding: [5, 3] as [number, number],
        labelBgBorderRadius: 5,
      };
    }),
  };
}

export function flowToGraph(
  name: string,
  entry: string,
  nodes: MacroFlowNode[],
  edges: Edge[],
): NodeGraph {
  const graphEdges: GraphEdge[] = edges.map((edge) => ({
    id: edge.id,
    from: edge.source,
    output: edge.sourceHandle || "next",
    to: edge.target,
  }));
  return {
    version: 1,
    name,
    entry,
    nodes: nodes.map((node) => ({
      ...node.data.graphNode,
      position: node.position,
    })),
    edges: graphEdges,
  };
}
