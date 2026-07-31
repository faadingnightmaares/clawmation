import { memo, useMemo } from "react";
import {
  BaseEdge,
  EdgeLabelRenderer,
  type Edge,
  type EdgeProps,
} from "@xyflow/react";

import type { GraphPosition } from "@/api";
import { cn } from "@/lib/utils";

export interface ProCanvasEdgeData extends Record<string, unknown> {
  waypoints?: GraphPosition[];
  focused?: boolean;
  dimmed?: boolean;
  outcome?: string;
  onWaypointPointerDown?: (
    edgeId: string,
    index: number,
    event: React.PointerEvent<HTMLButtonElement>,
  ) => void;
  onWaypointDelete?: (edgeId: string, index: number) => void;
}

export type ProCanvasFlowEdge = Edge<ProCanvasEdgeData, "pro">;

function distance(a: GraphPosition, b: GraphPosition): number {
  return Math.hypot(b.x - a.x, b.y - a.y);
}

function appendPoint(points: GraphPosition[], point: GraphPosition) {
  const previous = points[points.length - 1];
  if (!previous || previous.x !== point.x || previous.y !== point.y) points.push(point);
}

export function orthogonalPoints(
  source: GraphPosition,
  target: GraphPosition,
  waypoints: GraphPosition[],
): GraphPosition[] {
  const exit = { x: source.x + 30, y: source.y };
  const entry = { x: target.x - 30, y: target.y };
  const anchors = [exit, ...waypoints, entry];
  const points: GraphPosition[] = [source];
  appendPoint(points, exit);

  if (waypoints.length === 0) {
    const middleX = exit.x + (entry.x - exit.x) / 2;
    appendPoint(points, { x: middleX, y: exit.y });
    appendPoint(points, { x: middleX, y: entry.y });
  } else {
    for (let index = 1; index < anchors.length; index += 1) {
      const previous = anchors[index - 1];
      const current = anchors[index];
      if (previous.x !== current.x && previous.y !== current.y) {
        appendPoint(points, { x: current.x, y: previous.y });
      }
      appendPoint(points, current);
    }
  }
  appendPoint(points, entry);
  appendPoint(points, target);
  return points;
}

export function roundedOrthogonalPath(points: GraphPosition[], radius = 10): string {
  if (points.length === 0) return "";
  if (points.length === 1) return `M ${points[0].x} ${points[0].y}`;
  let path = `M ${points[0].x} ${points[0].y}`;
  for (let index = 1; index < points.length - 1; index += 1) {
    const previous = points[index - 1];
    const current = points[index];
    const next = points[index + 1];
    const incoming = distance(previous, current);
    const outgoing = distance(current, next);
    if (incoming === 0 || outgoing === 0) continue;
    const cornerRadius = Math.min(radius, incoming / 2, outgoing / 2);
    const before = {
      x: current.x + ((previous.x - current.x) / incoming) * cornerRadius,
      y: current.y + ((previous.y - current.y) / incoming) * cornerRadius,
    };
    const after = {
      x: current.x + ((next.x - current.x) / outgoing) * cornerRadius,
      y: current.y + ((next.y - current.y) / outgoing) * cornerRadius,
    };
    path += ` L ${before.x} ${before.y} Q ${current.x} ${current.y} ${after.x} ${after.y}`;
  }
  const last = points[points.length - 1];
  return `${path} L ${last.x} ${last.y}`;
}

function ProCanvasEdgeComponent({
  id,
  sourceX,
  sourceY,
  targetX,
  targetY,
  data,
  markerEnd,
  selected,
}: EdgeProps<ProCanvasFlowEdge>) {
  const waypoints = data?.waypoints ?? [];
  const path = useMemo(
    () => roundedOrthogonalPath(
      orthogonalPoints(
        { x: sourceX, y: sourceY },
        { x: targetX, y: targetY },
        waypoints,
      ),
    ),
    [sourceX, sourceY, targetX, targetY, waypoints],
  );
  const failure = ["error", "missing", "false"].includes(data?.outcome ?? "");
  const success = ["found", "true"].includes(data?.outcome ?? "");

  return (
    <>
      <BaseEdge
        id={id}
        path={path}
        markerEnd={markerEnd}
        interactionWidth={data?.focused ? 28 : 20}
        className={cn(
          "pro-canvas-edge",
          data?.dimmed && "pro-canvas-edge--dimmed",
          (data?.focused || selected) && "pro-canvas-edge--focused",
          failure && "pro-canvas-edge--failure",
          success && "pro-canvas-edge--success",
        )}
      />
      {waypoints.length > 0 && (selected || data?.focused) && (
        <EdgeLabelRenderer>
          {waypoints.map((point, index) => (
            <button
              key={`${id}-${index}`}
              type="button"
              aria-label={`Reroute point ${index + 1}`}
              title="Drag to reroute. Double-click to remove."
              className="nodrag nopan pro-canvas-reroute"
              style={{ transform: `translate(-50%, -50%) translate(${point.x}px, ${point.y}px)` }}
              onPointerDown={(event) => data?.onWaypointPointerDown?.(id, index, event)}
              onDoubleClick={(event) => {
                event.preventDefault();
                event.stopPropagation();
                data?.onWaypointDelete?.(id, index);
              }}
            />
          ))}
        </EdgeLabelRenderer>
      )}
    </>
  );
}

export const ProCanvasEdge = memo(ProCanvasEdgeComponent);
