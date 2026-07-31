import type { GraphEdge, GraphNode, NodeGraph, Step } from "@/api";
import { createGraphNode } from "@/lib/nodeGraph";

export const LOOP_TEMPLATES = [
  {
    id: "learn-loops",
    name: "Learn Loops",
    description: "A guided, editable Loop tutorial",
  },
  {
    id: "basic-sequence",
    name: "Basic Sequence",
    description: "A clean Start, Wait, Stop workflow",
  },
] as const;

export type LoopTemplateId = (typeof LOOP_TEMPLATES)[number]["id"];

function templateNode(
  kind: string,
  id: string,
  position: { x: number; y: number },
  label?: string,
): GraphNode {
  const node = createGraphNode(kind, position);
  node.id = id;
  if (label) node.label = label;
  const step = node.config.step as Step | undefined;
  if (step) step.id = `${id}-step`;
  return node;
}

function edge(
  id: string,
  from: string,
  output: string,
  to: string,
): GraphEdge {
  return { id, from, output, to };
}

function learnLoops(name: string): NodeGraph {
  const start = templateNode("start", "start", { x: 0, y: 80 });
  const wait = templateNode("delay", "tutorial-wait", { x: 210, y: 80 });
  const branch = templateNode("branch", "tutorial-branch", { x: 420, y: 80 });
  const success = templateNode(
    "stop",
    "tutorial-success",
    { x: 670, y: 20 },
    "Finished",
  );
  const failure = templateNode(
    "stop",
    "tutorial-failure",
    { x: 670, y: 160 },
    "Stopped",
  );
  failure.config.success = false;
  const continueTip = templateNode(
    "note",
    "tutorial-continue-tip",
    { x: 65, y: 245 },
    "Right-click a node to continue",
  );
  continueTip.config.text =
    "Right-click any node with an output, then choose what should run next.";
  const editTip = templateNode(
    "note",
    "tutorial-edit-tip",
    { x: 390, y: 245 },
    "Select a node to edit it",
  );
  editTip.config.text =
    "Select a node to change its action. Drag nodes to organize the Loop.";

  return {
    version: 1,
    name,
    entry: start.id,
    nodes: [
      start,
      wait,
      branch,
      success,
      failure,
      continueTip,
      editTip,
    ],
    edges: [
      edge("tutorial-start-wait", start.id, "next", wait.id),
      edge("tutorial-wait-branch", wait.id, "next", branch.id),
      edge("tutorial-branch-success", branch.id, "true", success.id),
      edge("tutorial-branch-failure", branch.id, "false", failure.id),
    ],
  };
}

function basicSequence(name: string): NodeGraph {
  const start = templateNode("start", "start", { x: 0, y: 80 });
  const wait = templateNode("delay", "sequence-wait", { x: 230, y: 80 });
  const stop = templateNode(
    "stop",
    "sequence-stop",
    { x: 470, y: 80 },
    "Finished",
  );

  return {
    version: 1,
    name,
    entry: start.id,
    nodes: [start, wait, stop],
    edges: [
      edge("sequence-start-wait", start.id, "next", wait.id),
      edge("sequence-wait-stop", wait.id, "next", stop.id),
    ],
  };
}

export function createLoopTemplateGraph(
  templateId: LoopTemplateId,
  name: string,
): NodeGraph {
  return templateId === "learn-loops"
    ? learnLoops(name)
    : basicSequence(name);
}
