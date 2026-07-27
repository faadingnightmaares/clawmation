import type { Edge, Node } from "@xyflow/react";

import type { GraphEdge, GraphNode, GraphNodeType, NodeGraph, Step } from "@/api";

export interface MacroNodeData extends Record<string, unknown> {
  graphNode: GraphNode;
}

export type MacroFlowNode = Node<MacroNodeData, "macro">;

export const OUTPUTS: Record<GraphNodeType, { id: string; label: string }[]> = {
  start: [{ id: "next", label: "Next" }],
  action: [
    { id: "next", label: "Next" },
    { id: "error", label: "Error" },
  ],
  vision: [
    { id: "found", label: "Found" },
    { id: "missing", label: "Missing" },
  ],
  branch: [
    { id: "true", label: "True" },
    { id: "false", label: "False" },
  ],
  loop: [
    { id: "body", label: "Loop" },
    { id: "done", label: "Done" },
  ],
  sub_macro: [
    { id: "success", label: "Success" },
    { id: "error", label: "Error" },
  ],
  stop: [],
};

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
  detect_mode: "color",
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
      find_click: "Find and click",
      wait_for: "Wait for screen",
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
    branch: "branch",
    loop: "loop",
    sub_macro: "sub_macro",
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
    stop: "Finish",
  };
  const config: Record<string, unknown> =
    nodeType === "branch"
      ? { condition: "last_ok" }
      : nodeType === "loop"
        ? { count: 3 }
        : nodeType === "sub_macro"
          ? { macro_name: "" }
          : nodeType === "stop"
            ? { success: true }
            : {};
  return { id, type: nodeType, label: labels[nodeType], position, enabled: true, config };
}

export function graphToFlow(graph: NodeGraph): { nodes: MacroFlowNode[]; edges: Edge[] } {
  return {
    nodes: graph.nodes.map((node) => ({
      id: node.id,
      type: "macro",
      position: node.position,
      data: { graphNode: node },
    })),
    edges: graph.edges.map((edge) => ({
      id: edge.id,
      source: edge.from,
      sourceHandle: edge.output,
      target: edge.to,
      targetHandle: "in",
      type: "smoothstep",
    })),
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
