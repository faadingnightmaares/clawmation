import type { GraphEdge, GraphNode, GraphPosition } from "@/api";

export interface NodePaletteItem {
  kind: string;
  label: string;
  description: string;
  category: "Actions" | "Vision" | "Flow";
  keywords: string[];
}

export const NODE_PALETTE: NodePaletteItem[] = [
  { kind: "click", label: "Click", description: "Click a screen position", category: "Actions", keywords: ["mouse", "tap", "position"] },
  { kind: "key", label: "Key", description: "Press a keyboard key", category: "Actions", keywords: ["press", "keyboard", "hotkey"] },
  { kind: "type", label: "Type", description: "Enter text", category: "Actions", keywords: ["text", "write", "input"] },
  { kind: "scroll", label: "Scroll", description: "Move the mouse wheel", category: "Actions", keywords: ["wheel", "move"] },
  { kind: "delay", label: "Wait", description: "Pause before continuing", category: "Actions", keywords: ["delay", "sleep", "time"] },
  { kind: "find_click", label: "Find & click", description: "Find an image, then click it", category: "Vision", keywords: ["image", "vision", "detect", "button", "object"] },
  { kind: "wait_for", label: "Wait for image", description: "Continue when an image appears", category: "Vision", keywords: ["image", "vision", "detect", "guard", "object"] },
  { kind: "start", label: "Start", description: "Set the Loop entry point", category: "Flow", keywords: ["entry", "begin"] },
  { kind: "branch", label: "Branch", description: "Choose a path from a condition", category: "Flow", keywords: ["if", "condition", "otherwise"] },
  { kind: "loop", label: "Repeat", description: "Repeat a group of nodes", category: "Flow", keywords: ["loop", "times", "forever"] },
  { kind: "sub_macro", label: "Macro", description: "Run an imported macro", category: "Flow", keywords: ["recording", "actions"] },
  { kind: "chain", label: "Chain", description: "Run a saved macro sequence", category: "Flow", keywords: ["sequence", "macros"] },
  { kind: "note", label: "Add note", description: "Document this part of the Loop", category: "Flow", keywords: ["comment", "text"] },
  { kind: "stop", label: "Stop", description: "Finish the Loop", category: "Flow", keywords: ["finish", "end"] },
];

function orderedMatchScore(query: string, candidate: string): number | null {
  const normalizedQuery = query.trim().toLowerCase();
  const normalizedCandidate = candidate.toLowerCase();
  if (!normalizedQuery) return 0;
  if (normalizedCandidate === normalizedQuery) return 10_000;
  if (normalizedCandidate.startsWith(normalizedQuery)) return 8_000 - normalizedCandidate.length;
  const containedAt = normalizedCandidate.indexOf(normalizedQuery);
  if (containedAt >= 0) return 6_000 - containedAt * 20 - normalizedCandidate.length;

  let queryIndex = 0;
  let gapPenalty = 0;
  let lastMatch = -1;
  for (let index = 0; index < normalizedCandidate.length && queryIndex < normalizedQuery.length; index += 1) {
    if (normalizedCandidate[index] !== normalizedQuery[queryIndex]) continue;
    if (lastMatch >= 0) gapPenalty += index - lastMatch - 1;
    lastMatch = index;
    queryIndex += 1;
  }
  return queryIndex === normalizedQuery.length ? 3_000 - gapPenalty * 20 - normalizedCandidate.length : null;
}

export function filterNodePalette(
  query: string,
  options: { allowStart?: boolean; recent?: string[] } = {},
): NodePaletteItem[] {
  const recent = options.recent ?? [];
  return NODE_PALETTE
    .filter((item) => options.allowStart !== false || item.kind !== "start")
    .map((item, index) => {
      const candidates = [item.label, item.kind.replace(/_/g, " "), ...item.keywords];
      const score = Math.max(
        ...candidates.map((candidate) => orderedMatchScore(query, candidate) ?? Number.NEGATIVE_INFINITY),
      );
      const recentIndex = recent.indexOf(item.kind);
      return {
        item,
        index,
        score: score + (recentIndex >= 0 ? Math.max(20, 180 - recentIndex * 24) : 0),
      };
    })
    .filter(({ score }) => Number.isFinite(score))
    .sort((a, b) => b.score - a.score || a.index - b.index)
    .map(({ item }) => item);
}

export function primaryOutputForNode(node: GraphNode): string | null {
  if (node.type === "vision") return "found";
  if (node.type === "branch") return "true";
  if (node.type === "loop") return "done";
  if (node.type === "sub_macro" || node.type === "chain") return "success";
  if (node.type === "stop" || node.type === "note") return null;
  return "next";
}

function edgeId(): string {
  return `edge-${crypto.randomUUID().slice(0, 8)}`;
}

export function insertNodeOnEdge(
  nodes: GraphNode[],
  edges: GraphEdge[],
  targetEdgeId: string,
  node: GraphNode,
): { nodes: GraphNode[]; edges: GraphEdge[] } | null {
  const edge = edges.find((candidate) => candidate.id === targetEdgeId);
  const output = primaryOutputForNode(node);
  if (!edge || !output || node.type === "start") return null;

  const before: GraphEdge = { ...edge, to: node.id, waypoints: [] };
  const after: GraphEdge = {
    id: edgeId(),
    from: node.id,
    output,
    to: edge.to,
  };
  return {
    nodes: nodes.some((candidate) => candidate.id === node.id)
      ? nodes.map((candidate) => candidate.id === node.id ? node : candidate)
      : [...nodes, node],
    edges: edges.flatMap((candidate) => candidate.id === targetEdgeId ? [before, after] : [candidate]),
  };
}

function distanceToSegment(
  point: GraphPosition,
  start: GraphPosition,
  end: GraphPosition,
): number {
  const dx = end.x - start.x;
  const dy = end.y - start.y;
  if (dx === 0 && dy === 0) return Math.hypot(point.x - start.x, point.y - start.y);
  const progress = Math.max(
    0,
    Math.min(1, ((point.x - start.x) * dx + (point.y - start.y) * dy) / (dx * dx + dy * dy)),
  );
  return Math.hypot(
    point.x - (start.x + progress * dx),
    point.y - (start.y + progress * dy),
  );
}

export function findEdgeNearPoint(
  point: GraphPosition,
  nodeId: string,
  nodes: GraphNode[],
  edges: GraphEdge[],
  threshold = 36,
): GraphEdge | null {
  const byId = new Map(nodes.map((node) => [node.id, node]));
  let closest: { edge: GraphEdge; distance: number } | null = null;
  for (const edge of edges) {
    if (edge.from === nodeId || edge.to === nodeId) continue;
    const source = byId.get(edge.from);
    const target = byId.get(edge.to);
    if (!source || !target) continue;
    const sourcePoint = { x: source.position.x + 150, y: source.position.y + 36 };
    const targetPoint = { x: target.position.x, y: target.position.y + 36 };
    const exit = { x: sourcePoint.x + 30, y: sourcePoint.y };
    const entry = { x: targetPoint.x - 30, y: targetPoint.y };
    const points = [sourcePoint, exit];
    if ((edge.waypoints ?? []).length === 0) {
      const middleX = exit.x + (entry.x - exit.x) / 2;
      points.push(
        { x: middleX, y: exit.y },
        { x: middleX, y: entry.y },
      );
    } else {
      for (const waypoint of edge.waypoints ?? []) {
        const previous = points[points.length - 1];
        if (previous.x !== waypoint.x && previous.y !== waypoint.y) {
          points.push({ x: waypoint.x, y: previous.y });
        }
        points.push(waypoint);
      }
    }
    points.push(entry, targetPoint);
    const distance = Math.min(
      ...points.slice(1).map((end, index) => distanceToSegment(point, points[index], end)),
    );
    if (distance <= threshold && (!closest || distance < closest.distance)) {
      closest = { edge, distance };
    }
  }
  return closest?.edge ?? null;
}

export function deleteNodeAndReconnect(
  nodes: GraphNode[],
  edges: GraphEdge[],
  nodeId: string,
): { nodes: GraphNode[]; edges: GraphEdge[] } | null {
  const node = nodes.find((candidate) => candidate.id === nodeId);
  if (!node || node.type === "start") return null;
  const incoming = edges.filter((edge) => edge.to === nodeId);
  const primaryOutput = primaryOutputForNode(node);
  const outgoing = primaryOutput
    ? edges.filter((edge) => edge.from === nodeId && edge.output === primaryOutput)
    : [];
  if (incoming.length !== 1 || outgoing.length !== 1) return null;
  if (edges.some((edge) => edge.from === nodeId && edge.id !== outgoing[0].id)) return null;

  const replacement: GraphEdge = {
    ...incoming[0],
    to: outgoing[0].to,
    waypoints: [
      ...(incoming[0].waypoints ?? []),
      ...(outgoing[0].waypoints ?? []),
    ],
  };
  return {
    nodes: nodes.filter((candidate) => candidate.id !== nodeId),
    edges: edges
      .filter((edge) => edge.id !== incoming[0].id && edge.id !== outgoing[0].id)
      .concat(replacement),
  };
}

export function connectedEdgeIds(edges: GraphEdge[], startNodeId: string): Set<string> {
  const nodeIds = new Set([startNodeId]);
  const result = new Set<string>();
  let changed = true;
  while (changed) {
    changed = false;
    for (const edge of edges) {
      if (!nodeIds.has(edge.from) && !nodeIds.has(edge.to)) continue;
      if (!result.has(edge.id)) {
        result.add(edge.id);
        changed = true;
      }
      if (!nodeIds.has(edge.from)) {
        nodeIds.add(edge.from);
        changed = true;
      }
      if (!nodeIds.has(edge.to)) {
        nodeIds.add(edge.to);
        changed = true;
      }
    }
  }
  return result;
}

export function wrapSelectionInRepeat(
  nodes: GraphNode[],
  edges: GraphEdge[],
  repeatId: string,
  selectedIds: string[],
): { nodes: GraphNode[]; edges: GraphEdge[] } | null {
  const selected = new Set(selectedIds.filter((id) => id !== repeatId));
  const repeat = nodes.find((node) => node.id === repeatId && node.type === "loop");
  if (!repeat || selected.size === 0) return null;

  const incoming = edges.filter((edge) => !selected.has(edge.from) && selected.has(edge.to));
  const outgoing = edges.filter((edge) => selected.has(edge.from) && !selected.has(edge.to));
  if (incoming.length !== 1 || outgoing.length !== 1) return null;
  const internal = edges.filter((edge) => selected.has(edge.from) && selected.has(edge.to));
  const internalIncoming = new Set(internal.map((edge) => edge.to));
  const internalOutgoing = new Set(internal.map((edge) => edge.from));
  const first = [...selected].filter((id) => !internalIncoming.has(id));
  const last = [...selected].filter((id) => !internalOutgoing.has(id));
  if (first.length !== 1 || last.length !== 1 || internal.length !== selected.size - 1) return null;

  const lastNode = nodes.find((node) => node.id === last[0]);
  const returnOutput = lastNode ? primaryOutputForNode(lastNode) : null;
  if (!returnOutput) return null;

  const replacedIncoming: GraphEdge = { ...incoming[0], to: repeatId };
  const doEdge: GraphEdge = {
    id: edgeId(),
    from: repeatId,
    output: "body",
    to: first[0],
  };
  const returnEdge: GraphEdge = {
    id: edgeId(),
    from: last[0],
    output: returnOutput,
    to: repeatId,
  };
  const thenEdge: GraphEdge = {
    id: edgeId(),
    from: repeatId,
    output: "done",
    to: outgoing[0].to,
  };
  const removed = new Set([incoming[0].id, outgoing[0].id]);
  return {
    nodes,
    edges: [
      ...edges.filter((edge) => !removed.has(edge.id)),
      replacedIncoming,
      doEdge,
      returnEdge,
      thenEdge,
    ],
  };
}

export function repeatReturnEdge(
  nodes: GraphNode[],
  edges: GraphEdge[],
  repeatId: string,
): GraphEdge | null {
  const body = edges.find((edge) => edge.from === repeatId && edge.output === "body");
  if (!body) return null;
  const seen = new Set([repeatId]);
  let current = body.to;
  while (!seen.has(current)) {
    seen.add(current);
    const node = nodes.find((candidate) => candidate.id === current);
    if (!node) return null;
    const output = primaryOutputForNode(node);
    if (!output) return null;
    const next = edges.find((edge) => edge.from === current && edge.output === output);
    if (!next) {
      return {
        id: `edge-repeat-return-${repeatId}-${current}-${output}`,
        from: current,
        output,
        to: repeatId,
      };
    }
    if (next.to === repeatId) return next;
    current = next.to;
  }
  return null;
}

export function ensureRepeatReturnEdges(
  nodes: GraphNode[],
  edges: GraphEdge[],
): GraphEdge[] {
  let repaired = edges;
  for (const repeat of nodes.filter((node) => node.type === "loop")) {
    const returnEdge = repeatReturnEdge(nodes, repaired, repeat.id);
    if (!returnEdge || repaired.some((edge) => edge.id === returnEdge.id)) continue;
    repaired = [...repaired, returnEdge];
  }
  return repaired;
}

export function repeatReturnEdgeIds(nodes: GraphNode[], edges: GraphEdge[]): Set<string> {
  const repeatIds = new Set(
    nodes.filter((node) => node.type === "loop").map((node) => node.id),
  );
  const result = new Set<string>();
  for (const repeatId of repeatIds) {
    const body = edges.find((edge) => edge.from === repeatId && edge.output === "body");
    if (!body) continue;
    const visited = new Set<string>();
    const queue = [body.to];
    while (queue.length > 0) {
      const current = queue.shift()!;
      if (!visited.add(current)) continue;
      for (const edge of edges.filter((candidate) => candidate.from === current)) {
        if (edge.to === repeatId) result.add(edge.id);
        else if (!repeatIds.has(edge.to)) queue.push(edge.to);
      }
    }
  }
  return result;
}

export function nearestFreePosition(
  desired: GraphPosition,
  nodes: GraphNode[],
): GraphPosition {
  const overlaps = (position: GraphPosition) =>
    nodes.some((node) => Math.abs(node.position.x - position.x) < 170 && Math.abs(node.position.y - position.y) < 110);
  if (!overlaps(desired)) return desired;
  for (let ring = 1; ring <= 8; ring += 1) {
    for (const [dx, dy] of [[ring * 190, 0], [0, ring * 130], [ring * 190, ring * 130], [-ring * 190, ring * 130]]) {
      const candidate = { x: desired.x + dx, y: desired.y + dy };
      if (!overlaps(candidate)) return candidate;
    }
  }
  return { x: desired.x + 190, y: desired.y + 130 };
}
