import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import {
  Background,
  Handle,
  MiniMap,
  MarkerType,
  Position,
  ReactFlow,
  addEdge,
  useEdgesState,
  useNodesState,
  type Connection,
  type Edge,
  type EdgeChange,
  type FinalConnectionState,
  type NodeChange,
  type NodeProps,
  type OnConnectStartParams,
  type ReactFlowInstance,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import {
  ArrowClockwise,
  ArrowRight,
  BookOpenText,
  CaretRight,
  Command,
  Copy,
  CornersOut,
  Crosshair,
  CursorClick,
  DownloadSimple,
  DotsThree,
  Eye,
  FloppyDisk,
  GitBranch,
  Keyboard,
  LinkSimple,
  Minus,
  MouseScroll,
  NotePencil,
  PencilSimple,
  Play,
  PlayCircle,
  Plus,
  Repeat,
  SpinnerGap,
  Stop,
  StopCircle,
  TextT,
  Timer,
  Trash,
  UploadSimple,
  WarningCircle,
} from "@phosphor-icons/react";

import {
  addChain,
  addTemplateImage,
  captureTemplate,
  exportLoop,
  guardPickRegion,
  guardPickColor,
  nodeGraphLoad,
  nodeGraphRun,
  nodeGraphSave,
  nodeGraphValidate,
  macroToSteps,
  pickScreenPoint,
  saveTemplateUpload,
  stepsTest,
  stopPlayback,
  updateChain,
  type CaptureTemplateResult,
  type Chain,
  type GraphEdge,
  type GraphNode,
  type MacroListItem,
  type NodeLoopItem,
  type Status,
  type Step,
} from "@/api";
import {
  OUTPUTS,
  createGraphNode,
  effectiveFailureMode,
  embedMacroInNode,
  flowToGraph,
  graphToFlow,
  validateGraphClient,
  visibleNodeOutputs,
  type MacroFlowNode,
} from "@/lib/nodeGraph";
import {
  connectedEdgeIds,
  deleteNodeAndReconnect,
  ensureRepeatReturnEdges,
  filterNodePalette,
  findEdgeNearPoint,
  insertNodeOnEdge,
  nearestFreePosition,
  primaryOutputForNode,
  repeatReturnEdgeIds,
  wrapSelectionInRepeat,
} from "@/lib/nodeGraphAuthoring";
import { notify } from "@/lib/toast";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { ImageCandidateGallery } from "@/components/vision/ImageCandidateGallery";
import {
  appendImageCandidate,
  imageCandidates,
  removeImageCandidate,
  splitImageCandidates,
} from "@/lib/visionImages";
import {
  LOOP_TEMPLATES,
  type LoopTemplateId,
} from "@/lib/nodeGraphTemplates";
import {
  ChainComposer,
  type ChainDraft,
} from "./ChainComposer";
import { useNodeGraphHistory } from "./useNodeGraphHistory";
import {
  ProCanvasEdge,
  type ProCanvasEdgeData,
} from "./ProCanvasEdge";

const NODE_ICONS = {
  start: Play,
  action: CursorClick,
  vision: Eye,
  branch: GitBranch,
  loop: Repeat,
  sub_macro: Command,
  chain: LinkSimple,
  note: NotePencil,
  stop: Stop,
};

const NODE_TONES = {
  start: "node-card--start text-success",
  action: "node-card--action text-primary",
  vision: "node-card--vision text-sky-400",
  branch: "node-card--branch text-violet-400",
  loop: "node-card--loop text-primary",
  sub_macro: "node-card--macro text-primary",
  chain: "node-card--chain text-amber-600",
  note: "node-card--note text-muted-foreground",
  stop: "node-card--stop text-destructive",
} satisfies Record<GraphNode["type"], string>;

const NODE_WIDTHS = {
  start: "w-[122px]",
  action: "w-[130px]",
  vision: "w-[150px]",
  branch: "w-[148px]",
  loop: "w-[144px]",
  sub_macro: "w-[144px]",
  chain: "w-[144px]",
  note: "w-[176px]",
  stop: "w-[120px]",
} satisfies Record<GraphNode["type"], string>;

function nodeSummary(node: GraphNode): string {
  if (node.type === "action" || node.type === "vision") {
    const step = node.config.step as Step | undefined;
    if (!step) return "Invalid action";
    if (step.type === "click") return `${step.x}, ${step.y}`;
    if (step.type === "key") return step.key || "Choose a key";
    if (step.type === "type") return step.text || "Enter text";
    if (step.type === "scroll") return `${step.scroll_amount > 0 ? "+" : ""}${step.scroll_amount}`;
    if (step.type === "delay") return `${step.delay} second${step.delay === 1 ? "" : "s"}`;
    if (step.type === "wait_for") {
      return imageCandidates(step.template, step.templates).length > 0
        ? `Wait up to ${step.timeout}s`
        : "Choose an image";
    }
    if (step.type === "find_click") {
      return imageCandidates(step.template, step.templates).length > 0
        ? "Click when image appears"
        : "Choose an image";
    }
    return step.type;
  }
  if (node.type === "branch") {
    const condition = String(node.config.condition || "last_ok");
    if (condition === "last_ok" || condition === "last_failed") return "Status check";
    return condition.replace(/_/g, " ");
  }
  if (node.type === "loop") {
    const count = Number(node.config.count ?? 1);
    return count === 0 ? "Until stopped" : `${count} times`;
  }
  if (node.type === "sub_macro") {
    const count = Array.isArray(node.config.embedded_steps)
      ? node.config.embedded_steps.length
      : 0;
    const name = String(node.config.macro_name || "Choose a macro");
    return count > 0 ? `${name} · ${count} action${count === 1 ? "" : "s"}` : name;
  }
  if (node.type === "chain") return String(node.config.chain_name || "Choose a chain");
  if (node.type === "note") return String(node.config.text || "Add context");
  if (node.type === "stop") return node.config.success === false ? "Failure" : "Success";
  return "Entry point";
}

function MacroNodeCard({ data, selected }: NodeProps<MacroFlowNode>) {
  const node = data.graphNode;
  const step = node.config.step as Step | undefined;
  const Icon =
    node.type === "action" && step
      ? step.type === "delay"
        ? Timer
        : step.type === "key"
          ? Keyboard
          : step.type === "type"
            ? TextT
            : step.type === "scroll"
              ? MouseScroll
              : CursorClick
      : NODE_ICONS[node.type];
  const outputs = data.outputs ?? OUTPUTS[node.type];
  const showsOutcomeLabels = outputs.length > 1;
  const compact = node.type === "start" || node.type === "stop";
  const title =
    node.type === "sub_macro" &&
    (!node.label || node.label === node.config.macro_name || node.label === "Run macro")
      ? "Macro"
      : node.type === "chain" && (!node.label || node.label === "Run chain")
        ? "Chain"
        : node.type === "stop" && node.label === "Finish"
          ? "Stop"
          : node.label;
  const templateThumb =
    node.type === "vision" && typeof node.config.template_thumb === "string"
      ? node.config.template_thumb
      : "";
  const templateCount =
    node.type === "vision" && step
      ? imageCandidates(step.template, step.templates).length
      : 0;
  return (
    <div
      className={cn(
        "node-card group/node relative overflow-visible rounded-[15px] border shadow-[0_7px_18px_rgba(0,0,0,0.08)] transition-[border-color,box-shadow,opacity]",
        NODE_WIDTHS[node.type],
        NODE_TONES[node.type],
        (selected || data.focused) && "ring-1 ring-primary/70 ring-offset-1 ring-offset-background",
        data.invalid && "border-destructive/70",
        !node.enabled && "opacity-50",
      )}
    >
      {node.type !== "start" && (
        <Handle
          type="target"
          position={Position.Left}
          id="in"
          className="!left-[-6px] !size-[11px] !border-2 !border-card !bg-card-foreground/55"
        />
      )}
      <div
        className={cn(
          "flex items-center gap-3 px-3",
          compact ? "min-h-[60px] py-2.5" : "min-h-[72px] py-3",
        )}
      >
        <span
          className="node-card__icon grid size-9 shrink-0 place-items-center rounded-[10px] border border-current/20 bg-background/75"
        >
          <Icon
            className="size-[18px]"
            weight={
              node.type === "start" || node.type === "stop"
                ? "fill"
                : node.type === "branch"
                  ? "duotone"
                  : "regular"
            }
          />
        </span>
        <div className="min-w-0 flex-1">
          <p className="truncate text-[13px] font-semibold text-foreground">{title}</p>
          {!compact && (
            <p className="mt-0.5 truncate text-[10px] text-muted-foreground">
              {nodeSummary(node)}
            </p>
          )}
        </div>
        {templateThumb && (
          <span className="relative shrink-0">
            <img
              src={templateThumb}
              alt={`${node.label} template`}
              className="size-10 rounded-md border border-border bg-muted/30 object-contain"
            />
            {templateCount > 1 && (
              <span
                className="absolute -right-1.5 -top-1.5 grid min-w-4 place-items-center rounded-full border border-card bg-primary px-1 text-[8px] font-bold leading-4 text-primary-foreground"
                aria-label={`${templateCount} image alternatives`}
              >
                {templateCount}
              </span>
            )}
          </span>
        )}
      </div>
      {outputs.length === 1 && (
        <Handle
          type="source"
          position={Position.Right}
          id={outputs[0].id}
          aria-label={outputs[0].label}
          className="!right-[-6px] !size-[11px] !border-2 !border-card !bg-card-foreground/55"
        />
      )}
      {outputs.length > 1 &&
        outputs.map((output, index) => {
          const top = `${((index + 1) / (outputs.length + 1)) * 100}%`;
          return (
            <div key={output.id}>
              {showsOutcomeLabels && (
                <span
                  aria-hidden="true"
                  className={cn(
                    "pointer-events-none absolute left-[calc(100%+21px)] z-20 w-max -translate-y-1/2 bg-background px-1.5 py-0.5 text-[9px] font-semibold leading-none before:absolute before:right-full before:top-1/2 before:h-px before:w-2 before:-translate-y-1/2 before:bg-current/45",
                    ["error", "missing", "false"].includes(output.id)
                      ? "text-destructive"
                      : ["true", "found"].includes(output.id)
                        ? "text-success"
                        : "text-primary",
                  )}
                  style={{ top }}
                >
                  {output.label}
                </span>
              )}
              <Handle
                type="source"
                position={Position.Right}
                id={output.id}
                aria-label={output.label}
                style={{ top, right: "-6px" }}
                className={cn(
                  "!size-[11px] !border-2 !border-card !bg-card-foreground/55",
                  showsOutcomeLabels &&
                    (["error", "missing", "false"].includes(output.id)
                      ? "!bg-destructive"
                      : ["true", "found"].includes(output.id)
                        ? "!bg-success"
                        : "!bg-primary"),
                )}
              />
            </div>
          );
        })}
    </div>
  );
}

const nodeTypes = { macro: MacroNodeCard };
const edgeTypes = { pro: ProCanvasEdge };

function miniMapNodeColor(node: MacroFlowNode): string {
  switch (node.data.graphNode.type) {
    case "start":
      return "var(--success)";
    case "stop":
      return "var(--destructive)";
    case "branch":
      return "rgb(168 85 247)";
    case "vision":
      return "rgb(56 189 248)";
    case "chain":
      return "rgb(217 119 6)";
    case "note":
      return "var(--muted-foreground)";
    default:
      return "var(--primary)";
  }
}

const PALETTE = [
  { kind: "click", label: "Click", Icon: CursorClick },
  { kind: "key", label: "Key", Icon: Keyboard },
  { kind: "type", label: "Type", Icon: TextT },
  { kind: "scroll", label: "Scroll", Icon: MouseScroll },
  { kind: "delay", label: "Wait", Icon: Timer },
  { kind: "find_click", label: "Find & click", Icon: Crosshair },
  { kind: "wait_for", label: "Wait for image", Icon: Eye },
  { kind: "start", label: "Start", Icon: PlayCircle },
  { kind: "branch", label: "Branch", Icon: GitBranch },
  { kind: "loop", label: "Repeat", Icon: Repeat },
  { kind: "sub_macro", label: "Macro", Icon: Play },
  { kind: "chain", label: "Chain", Icon: LinkSimple },
  { kind: "note", label: "Add note", Icon: NotePencil },
  { kind: "stop", label: "Stop", Icon: StopCircle },
];

const ACTION_PALETTE = [
  PALETTE[0],
  PALETTE[1],
  PALETTE[2],
  PALETTE[3],
  PALETTE[4],
  PALETTE[5],
  PALETTE[6],
];
const FLOW_PALETTE = [
  PALETTE[7],
  PALETTE[8],
  PALETTE[9],
  PALETTE[10],
  PALETTE[11],
  PALETTE[12],
  PALETTE[13],
];
const CONNECTED_FLOW_PALETTE = FLOW_PALETTE.filter(
  ({ kind }) => kind !== "start",
);

function NumberField({
  label,
  value,
  onChange,
  min,
  max,
  step,
}: {
  label: string;
  value: number;
  onChange: (value: number) => void;
  min?: number;
  max?: number;
  step?: number;
}) {
  return (
    <label className="grid gap-1.5 text-xs text-muted-foreground">
      {label}
      <Input
        className="h-9"
        type="number"
        min={min}
        max={max}
        step={step}
        value={value}
        onChange={(event) => onChange(Number(event.target.value))}
      />
    </label>
  );
}

interface NodeGraphEditorProps {
  loopName: string;
  loops: NodeLoopItem[];
  macros: MacroListItem[];
  chains: Chain[];
  status: Status | null;
  active?: boolean;
  workspaceBusy?: boolean;
  onSelectLoop: (name: string) => void;
  onCreateLoop: (templateId?: LoopTemplateId) => void | Promise<void>;
  onImportLoop?: () => void | Promise<void>;
  onRenameLoop: (oldName: string, newName: string) => Promise<boolean>;
  onDeleteLoop: (name: string) => void;
  onChanged?: () => void | Promise<void>;
}

interface EditorSnapshot {
  nodes: MacroFlowNode[];
  edges: Edge[];
  entry: string;
  selectedId: string | null;
}

export function NodeGraphEditor({
  loopName,
  loops,
  macros,
  chains,
  status,
  active = true,
  workspaceBusy = false,
  onSelectLoop,
  onCreateLoop,
  onImportLoop,
  onRenameLoop,
  onDeleteLoop,
  onChanged,
}: NodeGraphEditorProps) {
  const [nodes, setNodes, onNodesChange] = useNodesState<MacroFlowNode>([]);
  const [edges, setEdges, onEdgesChange] = useEdgesState<Edge>([]);
  const [entry, setEntry] = useState("start");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [busy, setBusy] = useState<"save" | "run" | "validate" | null>(null);
  const [issues, setIssues] = useState<{ errors: string[]; warnings: string[] }>({
    errors: [],
    warnings: [],
  });
  const [dirty, setDirty] = useState(false);
  const [testNodeId, setTestNodeId] = useState<string | null>(null);
  const [imageBusy, setImageBusy] = useState(false);
  const [imageDragOver, setImageDragOver] = useState(false);
  const [macroImportBusy, setMacroImportBusy] = useState<string | null>(null);
  const [chainBusy, setChainBusy] = useState(false);
  const [transferBusy, setTransferBusy] = useState<"import" | "export" | null>(
    null,
  );
  const [renamingLoop, setRenamingLoop] = useState<string | null>(null);
  const [loopNameDraft, setLoopNameDraft] = useState(loopName);
  const [viewportZoom, setViewportZoom] = useState(1);
  const [hoveredNodeId, setHoveredNodeId] = useState<string | null>(null);
  const [hoveredEdgeId, setHoveredEdgeId] = useState<string | null>(null);
  const [selectedEdgeId, setSelectedEdgeId] = useState<string | null>(null);
  const [pointPickBusy, setPointPickBusy] = useState(false);
  const [pickedMonitor, setPickedMonitor] = useState("");
  const [repeatWrapCandidates, setRepeatWrapCandidates] = useState<string[]>([]);
  const [recentNodeKinds, setRecentNodeKinds] = useState<string[]>(() => {
    try {
      return JSON.parse(localStorage.getItem("clawmation:recent-node-kinds") || "[]");
    } catch {
      return [];
    }
  });
  const [autocomplete, setAutocomplete] = useState<{
    left: number;
    top: number;
    position: { x: number; y: number };
    source: string;
    output: string;
    inserting: boolean;
    query: string;
    activeIndex: number;
  } | null>(null);
  const [contextMenu, setContextMenu] = useState<{
    left: number;
    top: number;
    position: { x: number; y: number };
    source?: string;
    output?: string;
  } | null>(null);
  const [templateMenu, setTemplateMenu] = useState<{
    left: number;
    top: number;
  } | null>(null);
  const [loopContextMenu, setLoopContextMenu] = useState<{
    left: number;
    top: number;
    name: string;
  } | null>(null);
  const loadedRef = useRef(false);
  const savedSignatureRef = useRef("");
  const draftStorageWarnedRef = useRef(false);
  const dragHistoryActiveRef = useRef(false);
  const skipRenameBlurRef = useRef(false);
  const flowRef = useRef<ReactFlowInstance<MacroFlowNode, Edge> | null>(null);
  const nodeCanvasRef = useRef<HTMLDivElement | null>(null);
  const autocompleteInputRef = useRef<HTMLInputElement | null>(null);
  const connectingRef = useRef<{ nodeId: string; handleId: string } | null>(null);
  const draftKey = `clawmation:node-draft:${loopName}`;

  const selected = useMemo(
    () => nodes.find((node) => node.id === selectedId) ?? null,
    [nodes, selectedId],
  );
  const historySnapshot = useMemo<EditorSnapshot>(
    () => ({ nodes, edges, entry, selectedId }),
    [edges, entry, nodes, selectedId],
  );
  const restoreHistory = useCallback(
    (snapshot: EditorSnapshot) => {
      setNodes(snapshot.nodes);
      setEdges(snapshot.edges);
      setEntry(snapshot.entry);
      setSelectedId(snapshot.selectedId);
      setContextMenu(null);
    },
    [setEdges, setNodes],
  );
  const {
    checkpoint,
    undo,
    redo,
    reset: resetHistory,
  } = useNodeGraphHistory(historySnapshot, restoreHistory);

  const buildGraph = useCallback(
    () => {
      const graph = flowToGraph(loopName, entry, nodes, edges);
      return {
        ...graph,
        edges: ensureRepeatReturnEdges(graph.nodes, graph.edges),
      };
    },
    [loopName, entry, nodes, edges],
  );
  const graphEdges = useMemo<GraphEdge[]>(
    () =>
      edges.map((edge) => ({
        id: edge.id,
        from: edge.source,
        output: edge.sourceHandle || "next",
        to: edge.target,
        ...(
          Array.isArray(edge.data?.waypoints) && edge.data.waypoints.length > 0
            ? { waypoints: edge.data.waypoints as GraphEdge["waypoints"] }
            : {}
        ),
      })),
    [edges],
  );

  const openAutocomplete = useCallback(
    (
      source: string,
      output: string,
      clientX: number,
      clientY: number,
      position?: { x: number; y: number },
    ) => {
      const canvas = nodeCanvasRef.current;
      const flow = flowRef.current;
      if (!canvas) return;
      const bounds = canvas.getBoundingClientRect();
      const menuWidth = 320;
      const menuHeight = 390;
      setContextMenu(null);
      setAutocomplete({
        left: Math.max(12, Math.min(clientX - bounds.left, bounds.width - menuWidth - 12)),
        top: Math.max(12, Math.min(clientY - bounds.top, bounds.height - menuHeight - 12)),
        position:
          position ??
          flow?.screenToFlowPosition({ x: clientX, y: clientY }) ?? {
            x: clientX - bounds.left,
            y: clientY - bounds.top,
          },
        source,
        output,
        inserting: edges.some(
          (edge) =>
            edge.source === source && (edge.sourceHandle || "next") === output,
        ),
        query: "",
        activeIndex: 0,
      });
    },
    [edges],
  );

  useEffect(() => {
    if (!autocomplete) return;
    const frame = requestAnimationFrame(() => autocompleteInputRef.current?.focus());
    return () => cancelAnimationFrame(frame);
  }, [autocomplete]);

  useEffect(() => {
    let active = true;
    let readyTimer: ReturnType<typeof setTimeout> | undefined;
    loadedRef.current = false;
    resetHistory();
    setLoading(true);
    setIssues({ errors: [], warnings: [] });
    void nodeGraphLoad(loopName)
      .then((result) => {
        if (!active) return;
        if (!result.ok || !result.graph) {
          notify("error", result.error || "Couldn’t load this Loop.");
          return;
        }
        const repairedEdges = ensureRepeatReturnEdges(
          result.graph.nodes,
          result.graph.edges,
        );
        const savedSignature = JSON.stringify(result.graph);
        savedSignatureRef.current = savedSignature;
        let graph = { ...result.graph, edges: repairedEdges };
        try {
          const draft = JSON.parse(localStorage.getItem(draftKey) || "null") as {
            graph?: typeof result.graph;
          } | null;
          if (draft?.graph?.version === result.graph.version && draft.graph.name === loopName) {
            graph = {
              ...draft.graph,
              edges: ensureRepeatReturnEdges(draft.graph.nodes, draft.graph.edges),
            };
          }
        } catch {
          localStorage.removeItem(draftKey);
        }
        const flow = graphToFlow(graph);
        setNodes(flow.nodes);
        setEdges(flow.edges);
        setEntry(graph.entry);
        setSelectedId(null);
        setDirty(JSON.stringify(graph) !== savedSignature);
        readyTimer = setTimeout(() => {
          loadedRef.current = true;
        }, 0);
      })
      .catch((error) => {
        if (active) notify("error", String(error));
      })
      .finally(() => {
        if (active) setLoading(false);
      });
    return () => {
      active = false;
      if (readyTimer) clearTimeout(readyTimer);
    };
  }, [draftKey, loopName, resetHistory, setEdges, setNodes]);

  useEffect(() => {
    if (!loadedRef.current) return;
    const graph = buildGraph();
    const signature = JSON.stringify(graph);
    const changed = signature !== savedSignatureRef.current;
    setDirty(changed);
    const timer = setTimeout(() => {
      if (changed) {
        try {
          localStorage.setItem(draftKey, JSON.stringify({ graph, updated_at: Date.now() }));
          draftStorageWarnedRef.current = false;
        } catch {
          localStorage.removeItem(draftKey);
          if (!draftStorageWarnedRef.current) {
            draftStorageWarnedRef.current = true;
            notify("warning", "This graph is too large for draft recovery. Save it to keep your changes.");
          }
        }
      } else {
        localStorage.removeItem(draftKey);
        draftStorageWarnedRef.current = false;
      }
    }, 350);
    return () => clearTimeout(timer);
  }, [buildGraph, draftKey]);

  const clientIssues = useMemo(
    () => validateGraphClient(buildGraph(), macros.map((macro) => macro.name), chains),
    [buildGraph, chains, macros],
  );
  const renderNodes = useMemo<MacroFlowNode[]>(
    () =>
      nodes.map((node) => ({
        ...node,
        data: {
          ...node.data,
          invalid: clientIssues.nodeIds.has(node.id),
          focused: hoveredNodeId === node.id,
          outputs: visibleNodeOutputs(
            node.data.graphNode,
            graphEdges,
          ),
        },
      })),
    [clientIssues.nodeIds, graphEdges, hoveredNodeId, nodes],
  );

  const focusedEdgeIds = useMemo(() => {
    if (hoveredEdgeId) return new Set([hoveredEdgeId]);
    const focusNode = hoveredNodeId ?? selectedId;
    if (!focusNode) return null;
    return connectedEdgeIds(
      graphEdges,
      focusNode,
    );
  }, [graphEdges, hoveredEdgeId, hoveredNodeId, selectedId]);
  const loopRailEdgeIds = useMemo(
    () =>
      repeatReturnEdgeIds(
        nodes.map((node) => ({ ...node.data.graphNode, position: node.position })),
        graphEdges,
      ),
    [graphEdges, nodes],
  );

  const beginWaypointDrag = useCallback(
    (
      edgeId: string,
      index: number,
      event: React.PointerEvent<HTMLButtonElement>,
    ) => {
      event.preventDefault();
      event.stopPropagation();
      checkpoint();
      const move = (pointer: PointerEvent) => {
        const point = flowRef.current?.screenToFlowPosition({
          x: pointer.clientX,
          y: pointer.clientY,
        });
        if (!point) return;
        setEdges((current) =>
          current.map((edge) => {
            if (edge.id !== edgeId) return edge;
            const waypoints = [...((edge.data?.waypoints as { x: number; y: number }[] | undefined) ?? [])];
            waypoints[index] = point;
            return { ...edge, data: { ...edge.data, waypoints } };
          }),
        );
      };
      const finish = () => {
        window.removeEventListener("pointermove", move);
        window.removeEventListener("pointerup", finish);
      };
      window.addEventListener("pointermove", move);
      window.addEventListener("pointerup", finish, { once: true });
    },
    [checkpoint, setEdges],
  );

  const deleteWaypoint = useCallback(
    (edgeId: string, index: number) => {
      checkpoint();
      setEdges((current) =>
        current.map((edge) => {
          if (edge.id !== edgeId) return edge;
          const waypoints = ((edge.data?.waypoints as { x: number; y: number }[] | undefined) ?? [])
            .filter((_, waypointIndex) => waypointIndex !== index);
          return { ...edge, data: { ...edge.data, waypoints } };
        }),
      );
    },
    [checkpoint, setEdges],
  );

  const renderEdges = useMemo<Edge[]>(
    () =>
      edges.filter((edge) => !loopRailEdgeIds.has(edge.id)).map((edge) => {
        const focused = focusedEdgeIds?.has(edge.id) ?? false;
        return {
          ...edge,
          type: "pro",
          data: {
            ...edge.data,
            focused,
            dimmed: focusedEdgeIds !== null && !focused,
            outcome: edge.sourceHandle || "next",
            onWaypointPointerDown: beginWaypointDrag,
            onWaypointDelete: deleteWaypoint,
          } satisfies ProCanvasEdgeData,
        };
      }),
    [beginWaypointDrag, deleteWaypoint, edges, focusedEdgeIds, loopRailEdgeIds],
  );

  const handleNodesChange = useCallback(
    (changes: NodeChange<MacroFlowNode>[]) => {
      if (changes.some((change) => change.type === "remove")) checkpoint();
      onNodesChange(changes);
    },
    [checkpoint, onNodesChange],
  );

  const handleEdgesChange = useCallback(
    (changes: EdgeChange<Edge>[]) => {
      if (changes.some((change) => change.type === "remove")) checkpoint();
      onEdgesChange(changes);
    },
    [checkpoint, onEdgesChange],
  );

  const onConnect = useCallback(
    (connection: Connection) => {
      if (!connection.source || !connection.target || connection.source === connection.target) return;
      checkpoint();
      const output = connection.sourceHandle || "next";
      setEdges((current) => {
        const connected = addEdge(
          {
            ...connection,
            id: `edge-${crypto.randomUUID().slice(0, 8)}`,
            type: "pro",
            data: { waypoints: [] },
          },
          current.filter(
            (edge) => !(edge.source === connection.source && edge.sourceHandle === output),
          ),
        );
        const graph = flowToGraph(loopName, entry, nodes, connected);
        const repaired = ensureRepeatReturnEdges(graph.nodes, graph.edges);
        return graphToFlow({ ...graph, edges: repaired }).edges;
      });
    },
    [checkpoint, edges, entry, loopName, nodes, setEdges],
  );

  const autocompleteResults = useMemo(
    () =>
      filterNodePalette(autocomplete?.query ?? "", {
        allowStart: false,
        recent: recentNodeKinds,
      }).filter(
        (item) =>
          !autocomplete?.inserting ||
          !["note", "stop"].includes(item.kind),
      ),
    [autocomplete?.query, recentNodeKinds],
  );

  const openContextMenu = useCallback(
    (
      event: MouseEvent | React.MouseEvent<Element>,
      connection?: {
        source: string;
        output: string;
        position: { x: number; y: number };
      },
    ) => {
      event.preventDefault();
      const flow = flowRef.current;
      const canvas = (event.currentTarget as HTMLElement).closest(
        ".node-canvas",
      ) as HTMLElement | null;
      if (!canvas) return;
      const bounds = canvas.getBoundingClientRect();
      const menuWidth = 192;
      const menuHeight = connection ? 410 : 480;
      const pointerLeft = event.clientX - bounds.left;
      setAutocomplete(null);
      setTemplateMenu(null);
      setContextMenu({
        left: Math.max(
          8,
          Math.min(pointerLeft, Math.max(8, bounds.width - menuWidth - 8)),
        ),
        top: Math.max(
          8,
          Math.min(
            event.clientY - bounds.top,
            bounds.height - menuHeight - 8,
          ),
        ),
        position:
          connection?.position ??
          (flow
            ? flow.screenToFlowPosition({ x: event.clientX, y: event.clientY })
            : {
                x: event.clientX - bounds.left,
                y: event.clientY - bounds.top,
              }),
        source: connection?.source,
        output: connection?.output,
      });
    },
    [],
  );

  useEffect(() => {
    if (
      !active ||
      (!contextMenu && !loopContextMenu && !autocomplete && !templateMenu)
    ) return;
    const close = () => {
      setContextMenu(null);
      setLoopContextMenu(null);
      setAutocomplete(null);
      setTemplateMenu(null);
    };
    const onEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") close();
    };
    window.addEventListener("pointerdown", close);
    window.addEventListener("keydown", onEscape);
    return () => {
      window.removeEventListener("pointerdown", close);
      window.removeEventListener("keydown", onEscape);
    };
  }, [active, autocomplete, contextMenu, loopContextMenu, templateMenu]);

  const addNode = (kind: string, position?: { x: number; y: number }) => {
    const desiredPosition =
      position ?? {
        x: 280 + (nodes.length % 4) * 250,
        y: 100 + Math.floor(nodes.length / 4) * 180,
      };
    const targetPosition = nearestFreePosition(
      desiredPosition,
      nodes.map((node) => ({ ...node.data.graphNode, position: node.position })),
    );
    const selectedBeforeAdd = nodes
      .filter((node) => node.selected && node.data.graphNode.type !== "start")
      .map((node) => node.id);
    const existingStart = kind === "start"
      ? nodes.find((node) => node.data.graphNode.type === "start")
      : undefined;
    checkpoint();
    if (existingStart) {
      setNodes((current) =>
        current.map((node) =>
          node.id === existingStart.id
            ? {
                ...node,
                position: targetPosition,
                data: {
                  graphNode: {
                    ...node.data.graphNode,
                    position: targetPosition,
                  },
                },
              }
            : node,
        ),
      );
      setEntry(existingStart.id);
      setSelectedId(existingStart.id);
      setContextMenu(null);
      notify("info", "Start moved to the selected position.");
      return;
    }
    const graphNode = createGraphNode(
      kind,
      targetPosition,
    );
    setNodes((current) => [
      ...current,
      {
        id: graphNode.id,
        type: "macro",
        position: graphNode.position,
        data: { graphNode },
      },
    ]);
    if (graphNode.type === "start") setEntry(graphNode.id);
    if (graphNode.type === "loop") setRepeatWrapCandidates(selectedBeforeAdd);
    else setRepeatWrapCandidates([]);
    setSelectedId(graphNode.id);
    setContextMenu(null);
  };

  const createConnectedNode = (
    kind: string,
    target: {
      source: string;
      output: string;
      position: { x: number; y: number };
    } | null = autocomplete,
  ) => {
    if (!target) return;
    const sourceNode = nodes.find((node) => node.id === target.source);
    if (!sourceNode) {
      setAutocomplete(null);
      return;
    }
    const targetPosition = nearestFreePosition(
      target.position,
      nodes.map((node) => ({ ...node.data.graphNode, position: node.position })),
    );
    const graphNode = createGraphNode(kind, targetPosition);
    if (graphNode.type === "start") return;
    const primaryOutput = primaryOutputForNode(graphNode);
    const selectedBeforeAdd = nodes
      .filter((node) => node.selected && node.id !== sourceNode.id)
      .map((node) => node.id);
    checkpoint();
    setNodes((current) => [
      ...current,
      {
        id: graphNode.id,
        type: "macro",
        position: graphNode.position,
        data: { graphNode },
      },
    ]);
    setEdges((current) => {
      const existing = current.find(
        (edge) =>
          edge.source === target.source &&
          (edge.sourceHandle || "next") === target.output,
      );
      const connected: Edge = {
        id: existing?.id ?? `edge-${crypto.randomUUID().slice(0, 8)}`,
        source: target.source,
        sourceHandle: target.output,
        target: graphNode.id,
        targetHandle: "in",
        type: "pro",
        data: { waypoints: [] },
      };
      const next = current.filter((edge) => edge.id !== existing?.id);
      if (existing && primaryOutput) {
        next.push({
          id: `edge-${crypto.randomUUID().slice(0, 8)}`,
          source: graphNode.id,
          sourceHandle: primaryOutput,
          target: existing.target,
          targetHandle: "in",
          type: "pro",
          data: { waypoints: existing.data?.waypoints ?? [] },
        });
      } else if (
        !existing &&
        sourceNode.data.graphNode.type === "loop" &&
        target.output === "body" &&
        primaryOutput
      ) {
        next.push({
          id: `edge-${crypto.randomUUID().slice(0, 8)}`,
          source: graphNode.id,
          sourceHandle: primaryOutput,
          target: sourceNode.id,
          targetHandle: "in",
          type: "pro",
          data: { waypoints: [] },
        });
      }
      return [...next, connected];
    });
    const recent = [kind, ...recentNodeKinds.filter((item) => item !== kind)].slice(0, 6);
    setRecentNodeKinds(recent);
    localStorage.setItem("clawmation:recent-node-kinds", JSON.stringify(recent));
    setRepeatWrapCandidates(graphNode.type === "loop" ? selectedBeforeAdd : []);
    setSelectedId(graphNode.id);
    setSelectedEdgeId(null);
    setAutocomplete(null);
    setContextMenu(null);
    setTemplateMenu(null);
  };

  const duplicateSelected = () => {
    if (!selected || selected.data.graphNode.type === "start") return;
    checkpoint();
    const id = `node-${crypto.randomUUID().slice(0, 8)}`;
    const graphNode = structuredClone(selected.data.graphNode);
    graphNode.id = id;
    graphNode.label = `${graphNode.label} copy`;
    graphNode.position = {
      x: selected.position.x + 36,
      y: selected.position.y + 36,
    };
    const step = graphNode.config.step as Step | undefined;
    if (step) step.id = crypto.randomUUID().slice(0, 8);
    setNodes((current) => [
      ...current,
      {
        id,
        type: "macro",
        position: graphNode.position,
        data: { graphNode },
      },
    ]);
    setSelectedId(id);
  };

  const arrangeGraph = () => {
    if (nodes.length === 0) return;
    checkpoint();
    const outgoing = new Map<string, string[]>();
    for (const edge of edges) {
      outgoing.set(edge.source, [...(outgoing.get(edge.source) ?? []), edge.target]);
    }
    const levels = new Map<string, number>([[entry, 0]]);
    const queue = [entry];
    while (queue.length > 0) {
      const source = queue.shift()!;
      const level = levels.get(source) ?? 0;
      for (const target of outgoing.get(source) ?? []) {
        if (!levels.has(target)) {
          levels.set(target, level + 1);
          queue.push(target);
        }
      }
    }
    const fallbackLevel = Math.max(0, ...levels.values()) + 1;
    const rows = new Map<number, string[]>();
    for (const node of nodes) {
      const level = levels.get(node.id) ?? fallbackLevel;
      rows.set(level, [...(rows.get(level) ?? []), node.id]);
    }
    setNodes((current) =>
      current.map((node) => {
        const level = levels.get(node.id) ?? fallbackLevel;
        const row = rows.get(level)?.indexOf(node.id) ?? 0;
        const rowCount = rows.get(level)?.length ?? 1;
        return {
          ...node,
          position: {
            x: 100 + level * 205,
            y: 220 + (row - (rowCount - 1) / 2) * 160,
          },
        };
      }),
    );
    requestAnimationFrame(() => {
      void flowRef.current?.fitView({ padding: 0.22, duration: 220 });
    });
  };

  const updateSelected = (patch: Partial<GraphNode>) => {
    if (!selectedId) return;
    checkpoint();
    setNodes((current) =>
      current.map((node) =>
        node.id === selectedId
          ? {
              ...node,
              data: {
                graphNode: { ...node.data.graphNode, ...patch },
              },
            }
          : node,
      ),
    );
  };

  const updateConfig = (patch: Record<string, unknown>) => {
    if (!selected) return;
    updateSelected({ config: { ...selected.data.graphNode.config, ...patch } });
  };

  const updateStep = (patch: Partial<Step>) => {
    if (!selected) return;
    const step = selected.data.graphNode.config.step as Step;
    updateConfig({ step: { ...step, ...patch } });
  };

  const importMacroSnapshot = async (name: string) => {
    if (!selectedId || !name) return;
    const targetId = selectedId;
    const source = macros.find((macro) => macro.name === name);
    if (!source) {
      notify("error", "That macro no longer exists.");
      return;
    }
    setMacroImportBusy(targetId);
    try {
      const result = await macroToSteps(name);
      if (!result.ok || !result.steps) {
        notify("error", result.error || `Couldn’t import “${name}”.`);
        return;
      }
      if (result.steps.length === 0) {
        notify("error", `“${name}” has no actions to import.`);
        return;
      }
      checkpoint();
      setNodes((current) =>
        current.map((node) =>
          node.id === targetId
            ? {
                ...node,
                data: {
                  graphNode: embedMacroInNode(node.data.graphNode, source, result.steps!),
                },
              }
            : node,
        ),
      );
      notify("success", `Imported “${name}” into this node.`);
    } catch (error) {
      notify("error", `Couldn’t import the macro: ${String(error)}`);
    } finally {
      setMacroImportBusy((current) => (current === targetId ? null : current));
    }
  };

  const removeSelected = () => {
    if (!selected) return;
    checkpoint();
    const nextStart = nodes.find(
      (node) => node.id !== selected.id && node.data.graphNode.type === "start",
    );
    setNodes((current) => current.filter((node) => node.id !== selected.id));
    setEdges((current) =>
      current.filter((edge) => edge.source !== selected.id && edge.target !== selected.id),
    );
    if (selected.id === entry) setEntry(nextStart?.id ?? "");
    setSelectedId(null);
  };

  const removeSelectedAndReconnect = () => {
    if (!selected) return;
    const graphNodes = nodes.map((node) => ({
      ...node.data.graphNode,
      position: node.position,
    }));
    const graphEdges = flowToGraph(loopName, entry, nodes, edges).edges;
    const result = deleteNodeAndReconnect(graphNodes, graphEdges, selected.id);
    if (!result) return;
    checkpoint();
    const flow = graphToFlow({
      version: 1,
      name: loopName,
      entry,
      nodes: result.nodes,
      edges: result.edges,
    });
    setNodes(flow.nodes);
    setEdges(flow.edges);
    setSelectedId(null);
  };

  const removeSelectedEdge = () => {
    if (!selectedEdgeId) return;
    checkpoint();
    setEdges((current) => current.filter((edge) => edge.id !== selectedEdgeId));
    setSelectedEdgeId(null);
  };

  const setFailureMode = (failureMode: "stop" | "continue" | "recovery") => {
    if (!selectedGraphNode) return;
    checkpoint();
    setNodes((current) =>
      current.map((node) =>
        node.id === selectedGraphNode.id
          ? {
              ...node,
              data: {
                ...node.data,
                graphNode: {
                  ...node.data.graphNode,
                  config: {
                    ...node.data.graphNode.config,
                    failure_mode: failureMode,
                  },
                },
              },
            }
          : node,
      ),
    );
    if (failureMode !== "recovery") {
      setEdges((current) =>
        current.filter(
          (edge) =>
            !(
              edge.source === selectedGraphNode.id &&
              (edge.sourceHandle || "next") === "error"
            ),
        ),
      );
    }
  };

  const wrapRepeatSelection = () => {
    if (selectedGraphNode?.type !== "loop" || repeatWrapCandidates.length === 0) return;
    const graph = flowToGraph(loopName, entry, nodes, edges);
    const result = wrapSelectionInRepeat(
      graph.nodes,
      graph.edges,
      selectedGraphNode.id,
      repeatWrapCandidates,
    );
    if (!result) {
      notify("warning", "Select one continuous path with one entrance and one exit.");
      return;
    }
    checkpoint();
    const flow = graphToFlow({ ...graph, nodes: result.nodes, edges: result.edges });
    setNodes(flow.nodes);
    setEdges(flow.edges);
    setRepeatWrapCandidates([]);
  };

  const pickClickPoint = async () => {
    if (!selectedStep || selectedStep.type !== "click" || pointPickBusy) return;
    setPointPickBusy(true);
    try {
      const result = await pickScreenPoint();
      if (result.ok && typeof result.x === "number" && typeof result.y === "number") {
        updateStep({ x: result.x, y: result.y });
        setPickedMonitor(result.monitor || "Desktop");
      } else if (result.error && result.error !== "cancelled") {
        notify("error", result.error);
      }
    } catch (error) {
      notify("error", `Couldn’t pick a screen point: ${String(error)}`);
    } finally {
      setPointPickBusy(false);
    }
  };

  const insertDraggedNode = (dragged: MacroFlowNode) => {
    if (dragged.data.graphNode.type === "start") return;
    const graph = flowToGraph(loopName, entry, nodes, edges);
    const draggedGraphNode = graph.nodes.find((node) => node.id === dragged.id);
    if (!draggedGraphNode) return;
    const targetEdge = findEdgeNearPoint(
      {
        x: dragged.position.x + 75,
        y: dragged.position.y + 36,
      },
      dragged.id,
      graph.nodes,
      graph.edges,
    );
    if (!targetEdge) return;
    const result = insertNodeOnEdge(graph.nodes, graph.edges, targetEdge.id, draggedGraphNode);
    if (!result) return;
    const flow = graphToFlow({ ...graph, nodes: result.nodes, edges: result.edges });
    setNodes(flow.nodes);
    setEdges(flow.edges);
    setSelectedId(dragged.id);
  };

  const pickColour = async () => {
    if (!selected) return;
    try {
      const result = await guardPickColor();
      if (result.ok && result.hsv_low && result.hsv_high) {
        updateStep({ detect_mode: "color", hsv_low: result.hsv_low, hsv_high: result.hsv_high });
      } else if (result.error && result.error !== "cancelled") {
        notify("error", result.error);
      }
    } catch {
      notify("error", "Couldn’t pick a colour.");
    }
  };

  const pickRegion = async () => {
    if (!selectedStep) return;
    try {
      const result = await guardPickRegion();
      if (result.ok && result.region) updateStep({ region: result.region });
      else if (result.error && result.error !== "cancelled") notify("error", result.error);
    } catch {
      notify("error", "Couldn’t pick a region.");
    }
  };

  const applyTemplate = (result: CaptureTemplateResult, successMessage: string) => {
    if (result.error === "cancelled") return;
    if (result.ok && result.path && selectedId && selectedStep) {
      const current = imageCandidates(selectedStep.template, selectedStep.templates);
      const appended = appendImageCandidate(current, result.path);
      if (appended.full) {
        notify("warning", "A vision node can use up to 8 images.");
        return;
      }
      if (!appended.added) {
        notify("info", "That image is already included.");
        return;
      }
      const split = splitImageCandidates(appended.candidates);
      const storedThumbs = Array.isArray(selectedGraphNode?.config.template_thumbs)
        ? (selectedGraphNode.config.template_thumbs as string[])
        : current.map((_, index) =>
            index === 0 && typeof selectedGraphNode?.config.template_thumb === "string"
              ? selectedGraphNode.config.template_thumb
              : "",
          );
      const nextThumbs = [
        ...current.map((_, index) => storedThumbs[index] || ""),
        result.thumb ? `data:image/png;base64,${result.thumb}` : "",
      ];
      updateConfig({
        step: {
          ...selectedStep,
          detect_mode: "template",
          template: split.primary,
          templates: split.alternatives,
        },
        template_thumb: nextThumbs[0] || "",
        template_thumbs: nextThumbs,
      });
      notify("success", successMessage);
    } else if (result.error) {
      notify("error", result.error);
    }
  };

  const chooseImage = async () => {
    if (!selectedStep) return;
    setImageBusy(true);
    try {
      applyTemplate(await addTemplateImage(), "Image added.");
    } catch (error) {
      notify("error", `Couldn’t choose the image: ${String(error)}`);
    } finally {
      setImageBusy(false);
    }
  };

  const magicSelectImage = async () => {
    if (!selectedStep) return;
    setImageBusy(true);
    try {
      applyTemplate(await captureTemplate(), "Screen selection saved.");
    } catch (error) {
      notify("error", `Couldn’t capture the selection: ${String(error)}`);
    } finally {
      setImageBusy(false);
    }
  };

  const uploadDroppedImage = async (file: File) => {
    if (!file.type.startsWith("image/")) {
      notify("error", "Drop a PNG, JPG, BMP, or WebP image.");
      return;
    }
    if (file.size > 20 * 1024 * 1024) {
      notify("error", "The image must be 20 MB or smaller.");
      return;
    }
    setImageBusy(true);
    try {
      const dataUrl = await new Promise<string>((resolve, reject) => {
        const reader = new FileReader();
        reader.onload = () => resolve(String(reader.result));
        reader.onerror = () => reject(reader.error || new Error("Could not read image"));
        reader.readAsDataURL(file);
      });
      const encoded = dataUrl.slice(dataUrl.indexOf(",") + 1);
      applyTemplate(await saveTemplateUpload(encoded), "Dropped image added.");
    } catch (error) {
      notify("error", `Couldn’t import the image: ${String(error)}`);
    } finally {
      setImageBusy(false);
      setImageDragOver(false);
    }
  };

  const removeTemplate = (index: number) => {
    if (!selectedStep) return;
    const current = imageCandidates(selectedStep.template, selectedStep.templates);
    const remaining = removeImageCandidate(current, index);
    const split = splitImageCandidates(remaining);
    const storedThumbs = Array.isArray(selectedGraphNode?.config.template_thumbs)
      ? (selectedGraphNode.config.template_thumbs as string[])
      : current.map((_, candidateIndex) =>
          candidateIndex === 0 &&
          typeof selectedGraphNode?.config.template_thumb === "string"
            ? selectedGraphNode.config.template_thumb
            : "",
        );
    const remainingThumbs = storedThumbs.filter(
      (_, candidateIndex) => candidateIndex !== index,
    );
    updateConfig({
      step: {
        ...selectedStep,
        template: split.primary,
        templates: split.alternatives,
      },
      template_thumb: remainingThumbs[0] || "",
      template_thumbs: remainingThumbs,
    });
  };

  const testStep = async () => {
    if (!selectedStep || !selectedId) return;
    setTestNodeId(selectedId);
    try {
      const result = await stepsTest(selectedStep);
      notify(
        result.ok ? "success" : "error",
        result.message || (result.ok ? "Node passed." : "Node failed."),
      );
    } catch (error) {
      notify("error", String(error));
    } finally {
      setTestNodeId(null);
    }
  };

  const validate = async (): Promise<boolean> => {
    setBusy("validate");
    try {
      if (clientIssues.errors.length > 0) {
        setIssues({ errors: clientIssues.errors, warnings: clientIssues.warnings });
        notify("error", clientIssues.errors[0]);
        return false;
      }
      const report = await nodeGraphValidate(buildGraph());
      setIssues({
        errors: report.errors,
        warnings: [...new Set([...clientIssues.warnings, ...report.warnings])],
      });
      if (!report.ok) notify("error", report.errors[0] || "The graph is incomplete.");
      return report.ok;
    } catch (error) {
      notify("error", String(error));
      return false;
    } finally {
      setBusy(null);
    }
  };

  const save = async (refreshWorkspace = true): Promise<boolean> => {
    if (!(await validate())) return false;
    setBusy("save");
    try {
      const result = await nodeGraphSave(loopName, buildGraph());
      if (result.ok) {
        savedSignatureRef.current = JSON.stringify(buildGraph());
        localStorage.removeItem(draftKey);
        setDirty(false);
        notify("success", "Loop saved.");
        if (refreshWorkspace) onChanged?.();
        return true;
      } else {
        notify("error", result.error || "Couldn’t save the Loop.");
        return false;
      }
    } catch (error) {
      notify("error", String(error));
      return false;
    } finally {
      setBusy(null);
    }
  };

  const beginLoopRename = (name = loopName) => {
    if (workspaceBusy) return;
    skipRenameBlurRef.current = false;
    setLoopContextMenu(null);
    setLoopNameDraft(name);
    setRenamingLoop(name);
  };

  const cancelLoopRename = () => {
    skipRenameBlurRef.current = true;
    setLoopNameDraft(loopName);
    setRenamingLoop(null);
  };

  const commitLoopRename = async (oldName: string) => {
    const nextName = loopNameDraft.trim();
    if (!nextName || nextName === oldName) {
      setLoopNameDraft(loopName);
      setRenamingLoop(null);
      return;
    }
    if (oldName === loopName && dirty && !(await save(false))) return;
    if (await onRenameLoop(oldName, nextName)) {
      localStorage.removeItem(`clawmation:node-draft:${oldName}`);
      setRenamingLoop(null);
    }
  };

  const run = async () => {
    if (!(await validate())) return;
    setBusy("run");
    try {
      const result = await nodeGraphRun(buildGraph());
      if (result.ok) notify("success", "Loop is running.");
      else notify("error", result.error || "Couldn’t run the Loop.");
    } finally {
      setBusy(null);
    }
  };

  const exportCurrentLoop = async () => {
    if (transferBusy) return;
    setLoopContextMenu(null);
    setTransferBusy("export");
    try {
      if (dirty && !(await save(false))) return;
      const result = await exportLoop(loopName);
      if (result.ok) notify("success", "Loop exported with all of its images.");
      else if (result.error !== "cancelled") {
        notify("error", result.error || "Couldn’t export the Loop.");
      }
    } catch (error) {
      notify("error", String(error));
    } finally {
      setTransferBusy(null);
    }
  };

  const importPortableLoop = async () => {
    if (transferBusy) return;
    setLoopContextMenu(null);
    setTransferBusy("import");
    try {
      await onImportLoop?.();
    } finally {
      setTransferBusy(null);
    }
  };

  const stop = async () => {
    try {
      const result = await stopPlayback();
      if (!result.ok) notify("error", result.error || "Couldn’t stop the graph.");
    } catch (error) {
      notify("error", String(error));
    }
  };

  useEffect(() => {
    if (!active) return;
    const onKey = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null;
      const editable =
        ["INPUT", "TEXTAREA", "SELECT"].includes(target?.tagName || "") ||
        Boolean(target?.isContentEditable);
      const primary = event.ctrlKey || event.metaKey;
      const key = event.key.toLowerCase();

      if (primary && key === "s") {
        event.preventDefault();
        void save();
        return;
      }
      if (editable) return;
      if (primary && key === "z") {
        event.preventDefault();
        if (event.shiftKey) redo();
        else undo();
      } else if (primary && key === "y") {
        event.preventDefault();
        redo();
      } else if (primary && key === "d") {
        event.preventDefault();
        duplicateSelected();
      } else if (event.key === "F2") {
        event.preventDefault();
        beginLoopRename();
      } else if (selectedEdgeId && (event.key === "Delete" || event.key === "Backspace")) {
        event.preventDefault();
        removeSelectedEdge();
      } else if (selected && (event.key === "Delete" || event.key === "Backspace")) {
        event.preventDefault();
        removeSelected();
      } else if (event.key === "Escape") {
        setSelectedId(null);
        setSelectedEdgeId(null);
        setContextMenu(null);
        setAutocomplete(null);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  const selectedGraphNode = selected?.data.graphNode;
  const selectedStep = selectedGraphNode?.config.step as Step | undefined;
  const selectedEmbeddedSteps = Array.isArray(selectedGraphNode?.config.embedded_steps)
    ? (selectedGraphNode.config.embedded_steps as Step[])
    : [];
  const selectedTemplateThumb =
    typeof selectedGraphNode?.config.template_thumb === "string"
      ? selectedGraphNode.config.template_thumb
      : "";
  const selectedImageCandidates = selectedStep
    ? imageCandidates(selectedStep.template, selectedStep.templates)
    : [];
  const selectedTemplateThumbs = Array.isArray(
    selectedGraphNode?.config.template_thumbs,
  )
    ? (selectedGraphNode.config.template_thumbs as string[])
    : selectedImageCandidates.map((_, index) =>
        index === 0 ? selectedTemplateThumb : "",
      );
  const selectedChain =
    selectedGraphNode?.type === "chain"
      ? chains.find(
          (chain) =>
            String(chain.id || "") ===
            String(selectedGraphNode.config.chain_id || ""),
        ) ?? null
      : null;
  const visibleErrors = issues.errors.length > 0 ? issues.errors : clientIssues.errors;
  const visibleWarnings = issues.warnings.length > 0 ? issues.warnings : clientIssues.warnings;
  const currentGraphEdges = graphEdges;
  const canReconnectSelected = Boolean(
    selected &&
      deleteNodeAndReconnect(
        nodes.map((node) => ({ ...node.data.graphNode, position: node.position })),
        currentGraphEdges,
        selected.id,
      ),
  );
  const createChainForSelectedNode = async () => {
    if (selectedGraphNode?.type !== "chain" || chainBusy) return;
    setChainBusy(true);
    try {
      const result = await addChain(
        `Chain ${chains.length + 1}`,
        [],
        1,
        1,
      );
      const chain = result.chain;
      const chainId = String(chain?.id || "");
      if (!result.ok || !chain || !chainId) {
        notify("error", result.error || "Couldn’t create the chain.");
        return;
      }
      updateConfig({
        chain_id: chainId,
        chain_name: String(chain.name || "Untitled chain"),
      });
      await onChanged?.();
    } catch (error) {
      notify("error", String(error));
    } finally {
      setChainBusy(false);
    }
  };

  const saveSelectedChain = async (
    chainId: string,
    draft: ChainDraft,
  ): Promise<boolean> => {
    if (chainBusy) return false;
    setChainBusy(true);
    try {
      const result = await updateChain(
        chainId,
        draft.name,
        draft.macroNames,
        draft.delayBetween,
        draft.repeat,
      );
      if (!result.ok) {
        notify("error", result.error || "Couldn’t save the chain.");
        return false;
      }
      updateConfig({ chain_name: draft.name });
      await onChanged?.();
      return true;
    } catch (error) {
      notify("error", String(error));
      return false;
    } finally {
      setChainBusy(false);
    }
  };

  return (
    <div className="flex h-full min-h-0 min-w-0 flex-col bg-background">
        <div className="flex min-h-[60px] shrink-0 items-center gap-2 border-b border-border bg-card/75 px-3 py-2">
          <div className="flex min-w-0 flex-1 items-center gap-2">
            <div
              className="flex min-w-36 max-w-56 flex-1 items-center"
              data-loop-picker=""
            >
              {renamingLoop === loopName ? (
                <Input
                  autoFocus
                  aria-label={`Rename ${loopName}`}
                  className="h-10 min-w-0 flex-1 bg-background text-sm font-semibold"
                  value={loopNameDraft}
                  maxLength={80}
                  disabled={workspaceBusy}
                  onFocus={(event) => event.currentTarget.select()}
                  onChange={(event) => setLoopNameDraft(event.target.value)}
                  onBlur={() => {
                    if (skipRenameBlurRef.current) {
                      skipRenameBlurRef.current = false;
                      return;
                    }
                    void commitLoopRename(loopName);
                  }}
                  onKeyDown={(event) => {
                    if (event.key === "Enter") {
                      event.preventDefault();
                      event.currentTarget.blur();
                    } else if (event.key === "Escape") {
                      event.preventDefault();
                      cancelLoopRename();
                      event.currentTarget.blur();
                    }
                  }}
                />
              ) : (
                <Select
                  value={loopName}
                  onValueChange={onSelectLoop}
                  disabled={workspaceBusy || loops.length === 0}
                >
                  <SelectTrigger
                    aria-label="Current Loop"
                    className="h-10 min-w-0 flex-1 bg-background text-sm font-semibold"
                    title="Double-click or right-click to rename"
                    onDoubleClick={(event) => {
                      event.preventDefault();
                      beginLoopRename(loopName);
                    }}
                    onContextMenu={(event) => {
                      event.preventDefault();
                      event.stopPropagation();
                      setContextMenu(null);
                      setLoopContextMenu({
                        left: Math.max(8, Math.min(event.clientX, window.innerWidth - 168)),
                        top: Math.max(8, Math.min(event.clientY, window.innerHeight - 128)),
                        name: loopName,
                      });
                    }}
                  >
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {loops.map((loop) => (
                      <SelectItem key={loop.name} value={loop.name}>
                        {loop.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              )}
            </div>
            <Button
              variant="outline"
              size="sm"
              className="h-10 shrink-0 whitespace-nowrap border-primary/35 px-3 text-primary hover:bg-primary/10 hover:text-primary"
              aria-label="New Loop"
              onClick={() => void onCreateLoop()}
              disabled={workspaceBusy}
            >
              <Plus className="size-4" weight="bold" />
              New Loop
            </Button>
            <div
              className="flex shrink-0 items-center gap-0.5 border-l border-border pl-1.5"
              aria-label="Loop file commands"
              role="group"
            >
              <Button
                type="button"
                variant="ghost"
                size="sm"
                className="h-10 shrink-0 whitespace-nowrap px-2.5 text-muted-foreground hover:text-foreground"
                title="Import a .clawbundle"
                onClick={() => void importPortableLoop()}
                disabled={workspaceBusy || transferBusy !== null || !onImportLoop}
              >
                {transferBusy === "import" ? (
                  <SpinnerGap className="size-4 animate-spin" />
                ) : (
                  <DownloadSimple className="size-4" />
                )}
                Import
              </Button>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                className="h-10 shrink-0 whitespace-nowrap px-2.5 text-muted-foreground hover:text-foreground"
                title="Export this Loop as a .clawbundle"
                onClick={() => void exportCurrentLoop()}
                disabled={
                  workspaceBusy ||
                  transferBusy !== null ||
                  loops.length === 0 ||
                  renamingLoop !== null
                }
              >
                {transferBusy === "export" ? (
                  <SpinnerGap className="size-4 animate-spin" />
                ) : (
                  <UploadSimple className="size-4" />
                )}
                Export
              </Button>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                className="h-10 shrink-0 whitespace-nowrap px-2.5 text-muted-foreground hover:text-foreground"
                aria-label="More"
                title="More Loop actions"
                disabled={
                  workspaceBusy ||
                  loops.length === 0 ||
                  renamingLoop !== null
                }
                onPointerDown={(event) => event.stopPropagation()}
                onClick={(event) => {
                  const bounds = event.currentTarget.getBoundingClientRect();
                  setContextMenu(null);
                  setLoopContextMenu((current) =>
                    current
                      ? null
                      : {
                          left: Math.max(
                            8,
                            Math.min(bounds.right - 160, window.innerWidth - 168),
                          ),
                          top: Math.max(
                            8,
                            Math.min(bounds.bottom + 6, window.innerHeight - 128),
                          ),
                          name: loopName,
                        },
                  );
                }}
              >
                <DotsThree className="size-4" weight="bold" />
                More
              </Button>
            </div>
            <div className="hidden items-center gap-2.5 border-l border-border pl-3 text-xs tabular-nums text-muted-foreground xl:flex">
              <ArrowRight className="size-3.5" />
              <span>{nodes.length} nodes</span>
              <span className="size-1 rounded-full bg-border" />
              <span>{edges.length} connections</span>
              {dirty && (
                <>
                  <span className="size-1 rounded-full bg-border" />
                  <span className="font-medium text-primary">Unsaved</span>
                </>
              )}
            </div>
          </div>

          <div className="flex shrink-0 items-center gap-2 border-l border-border pl-2">
            {status?.mode === "playing" ? (
              <Button variant="outline" size="sm" className="h-10 px-4 text-destructive hover:text-destructive" onClick={() => void stop()}>
                <StopCircle className="size-4" weight="fill" />
                Stop
              </Button>
            ) : (
              <Button variant="outline" size="sm" className="h-10 px-4" onClick={() => void run()} disabled={busy !== null || nodes.length === 0}>
                {busy === "run" ? <SpinnerGap className="size-4 animate-spin" /> : <Play className="size-4" weight="fill" />}
                Run
              </Button>
            )}
            <Button
              size="sm"
              className="h-10 px-4 shadow-[0_6px_16px_color-mix(in_srgb,var(--primary)_18%,transparent)]"
              onClick={() => void save()}
              disabled={busy !== null || !dirty}
            >
              {busy === "save" ? <SpinnerGap className="size-4 animate-spin" /> : <FloppyDisk className="size-4" />}
              Save
            </Button>
          </div>
        </div>

        <div className="flex min-h-0 min-w-0 flex-1">
          <div
            ref={nodeCanvasRef}
            className="node-canvas relative min-w-0 flex-1 bg-background"
          >
            {loading ? (
              <div className="absolute inset-0 z-10 grid place-items-center bg-background/80 text-sm text-muted-foreground">
                <span className="flex items-center gap-2">
                  <SpinnerGap className="size-4 animate-spin" /> Loading Loop…
                </span>
              </div>
            ) : (
              <ReactFlow
                nodes={renderNodes}
                edges={renderEdges}
                nodeTypes={nodeTypes}
                edgeTypes={edgeTypes}
                onInit={(instance) => {
                  flowRef.current = instance;
                  setViewportZoom(instance.getZoom());
                }}
                onMove={(_, viewport) => setViewportZoom(viewport.zoom)}
                onNodesChange={handleNodesChange}
                onEdgesChange={handleEdgesChange}
                onConnect={onConnect}
                onConnectStart={(
                  _event: MouseEvent | TouchEvent,
                  params: OnConnectStartParams,
                ) => {
                  connectingRef.current =
                    params.nodeId && params.handleId && params.handleType === "source"
                      ? { nodeId: params.nodeId, handleId: params.handleId }
                      : null;
                }}
                onConnectEnd={(
                  event: MouseEvent | TouchEvent,
                  connectionState: FinalConnectionState,
                ) => {
                  const started = connectingRef.current;
                  connectingRef.current = null;
                  if (!started || connectionState.isValid || connectionState.toHandle) return;
                  const pointer =
                    "clientX" in event
                      ? { x: event.clientX, y: event.clientY }
                      : {
                          x: event.changedTouches[0]?.clientX ?? 0,
                          y: event.changedTouches[0]?.clientY ?? 0,
                        };
                  openAutocomplete(
                    started.nodeId,
                    started.handleId,
                    pointer.x,
                    pointer.y,
                  );
                }}
                isValidConnection={(connection) =>
                  connection.source !== connection.target &&
                  nodes.find((node) => node.id === connection.target)?.data.graphNode.type !== "start"
                }
                onNodeClick={(_, node) => {
                  setSelectedId(node.id);
                  setSelectedEdgeId(null);
                }}
                onNodeContextMenu={(event, node) => {
                  event.preventDefault();
                  event.stopPropagation();
                  const output = primaryOutputForNode(node.data.graphNode);
                  if (!output) return;
                  setSelectedId(node.id);
                  setSelectedEdgeId(null);
                  setLoopContextMenu(null);
                  openContextMenu(event, {
                    source: node.id,
                    output,
                    position: {
                      x: node.position.x + 230,
                      y: node.position.y,
                    },
                  });
                }}
                onNodeMouseEnter={(_, node) => setHoveredNodeId(node.id)}
                onNodeMouseLeave={() => setHoveredNodeId(null)}
                onEdgeClick={(_, edge) => {
                  setSelectedId(null);
                  setSelectedEdgeId(edge.id);
                }}
                onEdgeMouseEnter={(_, edge) => setHoveredEdgeId(edge.id)}
                onEdgeMouseLeave={() => setHoveredEdgeId(null)}
                onEdgeDoubleClick={(event, edge) => {
                  event.preventDefault();
                  const point = flowRef.current?.screenToFlowPosition({
                    x: event.clientX,
                    y: event.clientY,
                  });
                  if (!point) return;
                  checkpoint();
                  setEdges((current) =>
                    current.map((candidate) =>
                      candidate.id === edge.id
                        ? {
                            ...candidate,
                            data: {
                              ...candidate.data,
                              waypoints: [
                                ...((candidate.data?.waypoints as { x: number; y: number }[] | undefined) ?? []),
                                point,
                              ],
                            },
                          }
                        : candidate,
                    ),
                  );
                  setSelectedEdgeId(edge.id);
                }}
                onNodeDragStart={() => {
                  if (!dragHistoryActiveRef.current) {
                    dragHistoryActiveRef.current = true;
                    checkpoint();
                  }
                }}
                onNodeDragStop={(_, node) => {
                  dragHistoryActiveRef.current = false;
                  insertDraggedNode(node);
                }}
                onPaneClick={() => {
                  setSelectedId(null);
                  setSelectedEdgeId(null);
                  setContextMenu(null);
                  setAutocomplete(null);
                  setTemplateMenu(null);
                }}
                onPaneContextMenu={openContextMenu}
                defaultViewport={{ x: 160, y: 0, zoom: 1 }}
                minZoom={0.2}
                maxZoom={1.8}
                deleteKeyCode={null}
                defaultEdgeOptions={{
                  type: "pro",
                  style: { strokeWidth: 1.7 },
                  markerEnd: {
                    type: MarkerType.ArrowClosed,
                    width: 14,
                    height: 14,
                    color: "color-mix(in srgb, var(--foreground) 48%, transparent)",
                  },
                }}
                proOptions={{ hideAttribution: true }}
              >
                <Background gap={24} size={1} color="color-mix(in srgb, var(--border) 48%, transparent)" />
                <MiniMap
                  pannable
                  zoomable
                  ariaLabel="Graph overview"
                  nodeColor={miniMapNodeColor}
                  nodeStrokeColor="color-mix(in srgb, var(--foreground) 30%, transparent)"
                  nodeStrokeWidth={1.5}
                  nodeBorderRadius={8}
                  bgColor="color-mix(in srgb, var(--card) 94%, var(--primary) 6%)"
                  maskColor="color-mix(in srgb, var(--background) 68%, transparent)"
                  maskStrokeColor="color-mix(in srgb, var(--primary) 45%, transparent)"
                  maskStrokeWidth={1}
                  offsetScale={2}
                  className={cn(
                    "node-minimap",
                    selectedGraphNode && "node-minimap--inspector-open",
                  )}
                  style={{ width: 172, height: 108 }}
                />
              </ReactFlow>
            )}
            {!loading && (
              <div className="absolute bottom-4 left-7 z-10 grid gap-3">
                <div className="overflow-hidden rounded-lg border border-border bg-card shadow-sm">
                  <button
                    type="button"
                    aria-label="Zoom in"
                    className="grid size-10 place-items-center border-b border-border text-foreground hover:bg-muted"
                    onClick={() => void flowRef.current?.zoomIn({ duration: 140 })}
                  >
                    <Plus className="size-[17px]" />
                  </button>
                  <div
                    className="grid h-9 w-10 place-items-center border-b border-border text-[10px] font-medium tabular-nums text-foreground"
                    aria-label={`Zoom ${Math.round(viewportZoom * 100)}%`}
                  >
                    {Math.round(viewportZoom * 100)}%
                  </div>
                  <button
                    type="button"
                    aria-label="Zoom out"
                    className="grid size-10 place-items-center text-foreground hover:bg-muted"
                    onClick={() => void flowRef.current?.zoomOut({ duration: 140 })}
                  >
                    <Minus className="size-[17px]" />
                  </button>
                </div>
                <button
                  type="button"
                  aria-label="Fit graph to view"
                  title="Fit graph to view"
                  className="grid size-10 place-items-center rounded-lg border border-border bg-card text-foreground shadow-sm hover:bg-muted"
                  onClick={() =>
                    void flowRef.current?.fitView({ padding: 0.2, duration: 180 })
                  }
                >
                  <CornersOut className="size-[17px]" />
                </button>
                <button
                  type="button"
                  aria-label="Arrange graph"
                  title="Arrange graph"
                  className="grid size-10 place-items-center rounded-lg border border-border bg-card text-foreground shadow-sm hover:bg-muted"
                  onClick={arrangeGraph}
                  disabled={nodes.length === 0}
                >
                  <GitBranch className="size-[17px]" />
                </button>
              </div>
            )}
            {contextMenu && (
              <div
                role="menu"
                aria-label="Add node"
                className="workspace-scrollbar absolute z-20 max-h-[calc(100%-16px)] w-48 overflow-y-auto rounded-xl border border-border bg-popover p-2 shadow-[0_18px_42px_rgba(0,0,0,0.18)]"
                style={{ left: contextMenu.left, top: contextMenu.top }}
                onPointerDown={(event) => event.stopPropagation()}
                onContextMenu={(event) => event.preventDefault()}
              >
                <p className="px-2 pb-1 pt-1.5 text-[10px] font-semibold text-muted-foreground">
                  Actions
                </p>
                {ACTION_PALETTE.map(({ kind, label, Icon }) => (
                  <button
                    key={kind}
                    type="button"
                    role="menuitem"
                    className="flex w-full items-center gap-2.5 rounded-md px-2.5 py-1.5 text-left text-xs text-popover-foreground outline-none transition-colors hover:bg-accent focus-visible:bg-accent"
                    onClick={() =>
                      contextMenu.source && contextMenu.output
                        ? createConnectedNode(kind, {
                            source: contextMenu.source,
                            output: contextMenu.output,
                            position: contextMenu.position,
                          })
                        : addNode(kind, contextMenu.position)
                    }
                  >
                    <Icon className="size-4 text-primary" weight="duotone" />
                    {label}
                  </button>
                ))}
                <div className="my-1.5 h-px bg-border" />
                <p className="px-2 pb-1 pt-1 text-[10px] font-semibold text-muted-foreground">
                  Flow
                </p>
                {(contextMenu.source
                  ? CONNECTED_FLOW_PALETTE
                  : FLOW_PALETTE
                ).map(({ kind, label, Icon }) => (
                  <button
                    key={kind}
                    type="button"
                    role="menuitem"
                    className="flex w-full items-center gap-2.5 rounded-md px-2.5 py-1.5 text-left text-xs text-popover-foreground outline-none transition-colors hover:bg-accent focus-visible:bg-accent"
                    onClick={() =>
                      contextMenu.source && contextMenu.output
                        ? createConnectedNode(kind, {
                            source: contextMenu.source,
                            output: contextMenu.output,
                            position: contextMenu.position,
                          })
                        : addNode(kind, contextMenu.position)
                    }
                  >
                    <Icon className="size-4 text-primary" weight="duotone" />
                    {label}
                  </button>
                ))}
                {!contextMenu.source && (
                  <>
                    <div className="my-1.5 h-px bg-border" />
                    <button
                      type="button"
                      role="menuitem"
                      aria-haspopup="menu"
                      aria-expanded={templateMenu !== null}
                      className="flex w-full items-center gap-2.5 rounded-md px-2.5 py-1.5 text-left text-xs font-medium text-popover-foreground outline-none transition-colors hover:bg-accent focus-visible:bg-accent"
                      onClick={(event) => {
                        const canvas = nodeCanvasRef.current;
                        if (!canvas) return;
                        const canvasBounds = canvas.getBoundingClientRect();
                        const anchor = event.currentTarget.getBoundingClientRect();
                        const parentMenu =
                          event.currentTarget.closest('[role="menu"]');
                        const parentBounds =
                          parentMenu?.getBoundingClientRect() ?? anchor;
                        const menuWidth = 224;
                        const menuHeight = 120;
                        const right =
                          parentBounds.right - canvasBounds.left + 8;
                        const left =
                          right + menuWidth <= canvasBounds.width - 8
                            ? right
                            : Math.max(
                                8,
                                parentBounds.left -
                                  canvasBounds.left -
                                  menuWidth -
                                  8,
                              );
                        setTemplateMenu({
                          left,
                          top: Math.max(
                            8,
                            Math.min(
                              anchor.top - canvasBounds.top,
                              canvasBounds.height - menuHeight - 8,
                            ),
                          ),
                        });
                      }}
                    >
                      <BookOpenText className="size-4 text-primary" weight="duotone" />
                      <span className="flex-1">Templates</span>
                      <CaretRight className="size-3.5 text-muted-foreground" />
                    </button>
                    <button
                      type="button"
                      role="menuitem"
                      className="flex w-full items-center gap-2.5 rounded-md px-2.5 py-1.5 text-left text-xs font-medium text-popover-foreground outline-none transition-colors hover:bg-accent focus-visible:bg-accent"
                      onClick={() => {
                        setContextMenu(null);
                        void onCreateLoop();
                      }}
                    >
                      <Plus className="size-4 text-primary" weight="bold" />
                      New Loop
                    </button>
                  </>
                )}
              </div>
            )}
            {templateMenu && (
              <div
                role="menu"
                aria-label="Loop templates"
                className="absolute z-30 w-56 rounded-xl border border-border bg-popover p-1.5 shadow-[0_18px_42px_rgba(0,0,0,0.18)]"
                style={{ left: templateMenu.left, top: templateMenu.top }}
                onPointerDown={(event) => event.stopPropagation()}
                onContextMenu={(event) => event.preventDefault()}
              >
                {LOOP_TEMPLATES.map((template) => (
                  <button
                    key={template.id}
                    type="button"
                    role="menuitem"
                    className="flex w-full items-start gap-2.5 rounded-md px-2.5 py-2 text-left text-popover-foreground outline-none transition-colors hover:bg-accent focus-visible:bg-accent"
                    onClick={() => {
                      setTemplateMenu(null);
                      setContextMenu(null);
                      void onCreateLoop(template.id);
                    }}
                  >
                    <BookOpenText
                      className="mt-0.5 size-4 shrink-0 text-primary"
                      weight="duotone"
                    />
                    <span className="min-w-0">
                      <span className="block text-xs font-medium">
                        {template.name}
                      </span>
                      <span className="mt-0.5 block text-[10px] leading-4 text-muted-foreground">
                        {template.description}
                      </span>
                    </span>
                  </button>
                ))}
              </div>
            )}
            {autocomplete && (
              <div
                role="dialog"
                aria-label="Add connected node"
                className="node-autocomplete absolute z-30 w-80 overflow-hidden rounded-xl border border-border bg-popover shadow-[0_20px_52px_rgba(0,0,0,0.22)]"
                style={{ left: autocomplete.left, top: autocomplete.top }}
                onPointerDown={(event) => event.stopPropagation()}
                onClick={(event) => event.stopPropagation()}
                onContextMenu={(event) => event.preventDefault()}
              >
                <div className="border-b border-border p-2">
                  <div className="flex items-center gap-2 rounded-lg border border-input bg-background px-2.5 focus-within:border-primary/55 focus-within:ring-2 focus-within:ring-primary/12">
                    <Command className="size-4 shrink-0 text-muted-foreground" />
                    <input
                      ref={autocompleteInputRef}
                      autoFocus
                      aria-label="Search nodes"
                      value={autocomplete.query}
                      placeholder="Search actions and flow"
                      className="h-10 min-w-0 flex-1 bg-transparent text-sm text-foreground outline-none placeholder:text-muted-foreground"
                      onChange={(event) =>
                        setAutocomplete((current) =>
                          current
                            ? { ...current, query: event.target.value, activeIndex: 0 }
                            : null,
                        )
                      }
                      onKeyDown={(event) => {
                        if (event.key === "ArrowDown") {
                          event.preventDefault();
                          setAutocomplete((current) =>
                            current
                              ? {
                                  ...current,
                                  activeIndex:
                                    (current.activeIndex + 1) %
                                    Math.max(1, autocompleteResults.length),
                                }
                              : null,
                          );
                        } else if (event.key === "ArrowUp") {
                          event.preventDefault();
                          setAutocomplete((current) =>
                            current
                              ? {
                                  ...current,
                                  activeIndex:
                                    (current.activeIndex - 1 + Math.max(1, autocompleteResults.length)) %
                                    Math.max(1, autocompleteResults.length),
                                }
                              : null,
                          );
                        } else if (event.key === "Enter") {
                          event.preventDefault();
                          const result = autocompleteResults[autocomplete.activeIndex];
                          if (result) createConnectedNode(result.kind);
                        } else if (event.key === "Escape") {
                          event.preventDefault();
                          setAutocomplete(null);
                        }
                      }}
                    />
                    <kbd className="rounded border border-border bg-muted/45 px-1.5 py-0.5 text-[9px] text-muted-foreground">
                      Esc
                    </kbd>
                  </div>
                </div>
                <div className="workspace-scrollbar max-h-[330px] overflow-y-auto p-1.5">
                  {autocompleteResults.length === 0 ? (
                    <p className="px-3 py-8 text-center text-xs text-muted-foreground">
                      No matching nodes
                    </p>
                  ) : (
                    autocompleteResults.map((item, index) => {
                      const palette = PALETTE.find((candidate) => candidate.kind === item.kind);
                      const Icon = palette?.Icon ?? Plus;
                      const showCategory =
                        index === 0 ||
                        autocompleteResults[index - 1]?.category !== item.category;
                      return (
                        <div key={item.kind}>
                          {showCategory && (
                            <p className="px-2.5 pb-1 pt-2 text-[10px] font-semibold text-muted-foreground">
                              {item.category}
                            </p>
                          )}
                          <button
                            type="button"
                            role="option"
                            aria-selected={index === autocomplete.activeIndex}
                            className={cn(
                              "flex w-full items-center gap-3 rounded-lg px-2.5 py-2 text-left outline-none transition-colors",
                              index === autocomplete.activeIndex
                                ? "bg-accent text-accent-foreground"
                                : "text-popover-foreground hover:bg-accent/65",
                            )}
                            onMouseEnter={() =>
                              setAutocomplete((current) =>
                                current ? { ...current, activeIndex: index } : null,
                              )
                            }
                            onClick={() => createConnectedNode(item.kind)}
                          >
                            <span className="grid size-8 shrink-0 place-items-center rounded-lg border border-border bg-background text-primary">
                              <Icon className="size-4" weight="duotone" />
                            </span>
                            <span className="min-w-0">
                              <span className="block text-xs font-semibold">{item.label}</span>
                              <span className="mt-0.5 block truncate text-[10px] text-muted-foreground">
                                {item.description}
                              </span>
                            </span>
                          </button>
                        </div>
                      );
                    })
                  )}
                </div>
              </div>
            )}
            {loopContextMenu && (
              <div
                role="menu"
                aria-label="Loop actions"
                data-state="open"
                className="ui-floating-surface fixed z-[80] w-40 rounded-lg border border-border bg-popover p-1.5 shadow-[0_16px_36px_rgba(0,0,0,0.18)]"
                style={{ left: loopContextMenu.left, top: loopContextMenu.top }}
                onPointerDown={(event) => event.stopPropagation()}
                onContextMenu={(event) => event.preventDefault()}
              >
                <button
                  type="button"
                  role="menuitem"
                  className="flex w-full items-center gap-2 rounded-md px-2 py-2 text-left text-xs text-popover-foreground outline-none hover:bg-accent focus-visible:bg-accent"
                  onClick={() => beginLoopRename(loopContextMenu.name)}
                >
                  <PencilSimple className="size-4 text-muted-foreground" />
                  Rename
                </button>
                <button
                  type="button"
                  role="menuitem"
                  className="flex w-full items-center gap-2 rounded-md px-2 py-2 text-left text-xs text-destructive outline-none hover:bg-accent focus-visible:bg-accent"
                  onClick={() => {
                    const name = loopContextMenu.name;
                    setLoopContextMenu(null);
                    onDeleteLoop(name);
                  }}
                >
                  <Trash className="size-4" />
                  Delete
                </button>
              </div>
            )}
            {(visibleErrors.length > 0 || visibleWarnings.length > 0) && (
              <div
                className={cn(
                  "pointer-events-none absolute bottom-4 left-20 z-10 flex max-w-[min(440px,calc(100%-20rem))] items-center gap-2 rounded-lg border bg-card/95 px-3 py-2 text-xs shadow-lg backdrop-blur-sm",
                  visibleErrors.length > 0
                    ? "border-destructive/35 text-destructive"
                    : "border-primary/25 text-muted-foreground",
                )}
                role="status"
              >
                <WarningCircle
                  className="size-4 shrink-0"
                  weight={visibleErrors.length > 0 ? "fill" : "regular"}
                />
                <span className="truncate">
                  {visibleErrors[0] || visibleWarnings[0]}
                </span>
              </div>
            )}
          </div>

          {selectedGraphNode && nodeCanvasRef.current &&
            createPortal(
              <aside
                role="complementary"
                aria-label="Node inspector"
                className={cn(
                  "node-floating-inspector absolute right-6 top-6 z-20 flex max-h-[calc(100%-48px)] w-[min(440px,calc(100%-48px))] flex-col overflow-hidden rounded-2xl border border-border bg-card shadow-[0_18px_48px_rgba(0,0,0,0.14)]",
                  selectedGraphNode.type === "chain" && "w-[min(540px,calc(100%-48px))]",
                )}
                onPointerDown={(event) => event.stopPropagation()}
                onClick={(event) => event.stopPropagation()}
                onContextMenu={(event) => event.stopPropagation()}
              >
                <div className="workspace-scrollbar min-h-0 overflow-y-auto">
                  <div className="grid gap-4 p-4">
                <div className="flex items-center justify-between gap-2 border-b border-border pb-3">
                  <div>
                    <p className="text-sm font-semibold">Inspector</p>
                    <p className="mt-0.5 text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
                      {selectedGraphNode.type === "loop"
                        ? "repeat"
                        : selectedGraphNode.type.replace("_", " ")}
                    </p>
                  </div>
                  <div className="flex items-center">
                    {selectedGraphNode.type !== "start" && (
                      <Button
                        variant="ghost"
                        size="icon-sm"
                        onClick={duplicateSelected}
                        title="Duplicate node"
                        aria-label="Duplicate node"
                      >
                        <Copy className="size-4" />
                      </Button>
                    )}
                    {canReconnectSelected && (
                      <Button
                        variant="ghost"
                        size="icon-sm"
                        onClick={removeSelectedAndReconnect}
                        title="Delete and reconnect"
                        aria-label="Delete and reconnect"
                      >
                        <LinkSimple className="size-4" />
                      </Button>
                    )}
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      onClick={removeSelected}
                      title="Delete node"
                      aria-label="Delete node"
                    >
                      <Trash className="size-4" />
                    </Button>
                  </div>
                </div>

                <label className="grid gap-1.5 text-xs text-muted-foreground">
                  Label
                  <Input
                    className="h-9"
                    value={selectedGraphNode.label}
                    onChange={(event) => updateSelected({ label: event.target.value })}
                  />
                </label>

                {selectedGraphNode.type !== "start" && (
                  <label className="flex items-center justify-between gap-3 border-y border-border py-2.5 text-xs">
                    Enabled
                    <Switch
                      checked={selectedGraphNode.enabled}
                      onCheckedChange={(enabled) => updateSelected({ enabled })}
                    />
                  </label>
                )}

                {selectedStep?.type === "click" && (
                  <div className="grid gap-3">
                    <Button
                      type="button"
                      className="h-10 w-full"
                      onClick={() => void pickClickPoint()}
                      disabled={pointPickBusy}
                    >
                      {pointPickBusy ? (
                        <SpinnerGap className="size-4 animate-spin" />
                      ) : (
                        <Crosshair className="size-4" />
                      )}
                      Pick on screen
                    </Button>
                    <div className="grid grid-cols-2 gap-2">
                      <NumberField label="X" value={selectedStep.x} onChange={(x) => updateStep({ x })} />
                      <NumberField label="Y" value={selectedStep.y} onChange={(y) => updateStep({ y })} />
                    </div>
                    {pickedMonitor && (
                      <p className="flex items-center gap-2 text-[10px] text-muted-foreground">
                        <Crosshair className="size-3.5 text-primary" />
                        {pickedMonitor}
                      </p>
                    )}
                  </div>
                )}
                {["action", "sub_macro", "chain"].includes(selectedGraphNode.type) && (
                  <label className="grid gap-1.5 text-xs text-muted-foreground">
                    On failure
                    <Select
                      value={effectiveFailureMode(selectedGraphNode, currentGraphEdges)}
                      onValueChange={(value) =>
                        setFailureMode(value as "stop" | "continue" | "recovery")
                      }
                    >
                      <SelectTrigger className="w-full bg-background" aria-label="On failure">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent position="popper" align="start">
                        <SelectItem value="stop">Stop Loop</SelectItem>
                        <SelectItem value="continue">Continue</SelectItem>
                        <SelectItem value="recovery">Recovery path</SelectItem>
                      </SelectContent>
                    </Select>
                  </label>
                )}
                {selectedStep?.type === "key" && (
                  <label className="grid gap-1.5 text-xs text-muted-foreground">
                    Key
                    <Input className="h-9" value={selectedStep.key} onChange={(event) => updateStep({ key: event.target.value })} />
                  </label>
                )}
                {selectedStep?.type === "type" && (
                  <label className="grid gap-1.5 text-xs text-muted-foreground">
                    Text
                    <Input className="h-9" value={selectedStep.text} onChange={(event) => updateStep({ text: event.target.value })} />
                  </label>
                )}
                {selectedStep?.type === "scroll" && (
                  <NumberField
                    label="Scroll amount"
                    value={selectedStep.scroll_amount}
                    onChange={(scroll_amount) => updateStep({ scroll_amount })}
                  />
                )}
                {selectedStep?.type === "delay" && (
                  <NumberField label="Seconds" value={selectedStep.delay} min={0} onChange={(delay) => updateStep({ delay })} />
                )}
                {selectedGraphNode.type === "vision" && selectedStep && (
                  <div className="grid gap-3 border-t border-border pt-4">
                    <label className="grid gap-1.5 text-xs text-muted-foreground">
                      Watch for
                      <Select
                        value={selectedStep.detect_mode}
                        onValueChange={(value) =>
                          updateStep({ detect_mode: value as Step["detect_mode"] })
                        }
                      >
                        <SelectTrigger className="w-full bg-background" aria-label="Watch for">
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent position="popper" align="start">
                          <SelectItem value="template">An image or object</SelectItem>
                          <SelectItem value="color">A colour</SelectItem>
                        </SelectContent>
                      </Select>
                    </label>

                    {selectedStep.detect_mode === "template" ? (
                      <ImageCandidateGallery
                        candidates={selectedImageCandidates}
                        thumbnails={selectedTemplateThumbs}
                        busy={imageBusy}
                        dragOver={imageDragOver}
                        onDragOverChange={setImageDragOver}
                        onChoose={() => void chooseImage()}
                        onMagicSelect={() => void magicSelectImage()}
                        onRemove={removeTemplate}
                        onDrop={(file) => void uploadDroppedImage(file)}
                      />
                    ) : (
                      <div className="grid gap-2">
                        <Button variant="outline" size="sm" onClick={() => void pickColour()}>
                          <Crosshair className="size-4" /> Pick colour
                        </Button>
                      </div>
                    )}

                    <div className="grid grid-cols-2 gap-2">
                      <Button variant="outline" size="sm" onClick={() => void pickRegion()}>
                        Select region
                      </Button>
                      <Button
                        variant="outline"
                        size="sm"
                        onClick={() => void testStep()}
                        disabled={
                          testNodeId === selectedId ||
                          (selectedStep.detect_mode === "template" &&
                            selectedImageCandidates.length === 0)
                        }
                      >
                        {testNodeId === selectedId ? (
                          <SpinnerGap className="size-4 animate-spin" />
                        ) : (
                          <Play className="size-4" />
                        )}
                        Test
                      </Button>
                    </div>

                    <div className="rounded-md border border-border bg-muted/30 px-3 py-2 text-[10px] leading-4 text-muted-foreground">
                      {selectedStep.type === "wait_for"
                        ? "When it appears, the graph follows Found. If time runs out, it follows Not found."
                        : "This node clicks the match, then follows Found. If nothing is visible, it follows Not found."}
                    </div>

                    <NumberField
                      label="Minimum confidence"
                      value={selectedStep.confidence}
                      min={0.1}
                      max={1}
                      step={0.05}
                      onChange={(confidence) => updateStep({ confidence })}
                    />
                    {selectedStep.type === "wait_for" && (
                      <NumberField
                        label="Timeout seconds"
                        value={selectedStep.timeout}
                        min={0}
                        onChange={(timeout) => updateStep({ timeout })}
                      />
                    )}
                  </div>
                )}
                {selectedGraphNode.type === "branch" && (
                  <label className="grid gap-1.5 text-xs text-muted-foreground">
                    Condition
                    <Select
                      value={String(selectedGraphNode.config.condition || "last_ok")}
                      onValueChange={(condition) => updateConfig({ condition })}
                    >
                      <SelectTrigger className="w-full bg-background" aria-label="Condition">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent position="popper" align="start">
                        <SelectItem value="last_ok">Last node succeeded</SelectItem>
                        <SelectItem value="last_failed">Last node failed</SelectItem>
                        <SelectItem value="always">Always true</SelectItem>
                        <SelectItem value="never">Always false</SelectItem>
                      </SelectContent>
                    </Select>
                  </label>
                )}
                {selectedGraphNode.type === "loop" && (
                  <div className="grid gap-3">
                    <div>
                      <p className="text-xs font-medium text-foreground">Repeat</p>
                      <p className="mt-1 text-[10px] leading-4 text-muted-foreground">
                        Choose how many times the Do path should run.
                      </p>
                    </div>
                    <div className="grid grid-cols-5 gap-1.5">
                      {[2, 3, 5, 0, "custom"].map((option) => {
                        const activeCount = Number(selectedGraphNode.config.count ?? 1);
                        const custom = option === "custom";
                        const active = custom
                          ? ![0, 2, 3, 5].includes(activeCount)
                          : activeCount === option;
                        return (
                          <button
                            key={option}
                            type="button"
                            className={cn(
                              "h-8 rounded-md border text-[10px] font-semibold transition-colors",
                              active
                                ? "border-primary/55 bg-primary/12 text-primary"
                                : "border-border bg-background text-muted-foreground hover:text-foreground",
                            )}
                            onClick={() =>
                              updateConfig({
                                count: custom
                                  ? ([0, 2, 3, 5].includes(activeCount) ? 1 : activeCount)
                                  : option,
                              })
                            }
                          >
                            {custom ? "Custom" : option === 0 ? "Forever" : `${option}x`}
                          </button>
                        );
                      })}
                    </div>
                    {![0, 2, 3, 5].includes(
                      Number(selectedGraphNode.config.count ?? 1),
                    ) && (
                      <NumberField
                        label="Count"
                        value={Number(selectedGraphNode.config.count ?? 1)}
                        min={1}
                        onChange={(count) =>
                          updateConfig({ count: Math.max(1, Math.floor(count)) })
                        }
                      />
                    )}
                    {repeatWrapCandidates.length > 0 && (
                      <Button
                        type="button"
                        variant="outline"
                        size="sm"
                        onClick={wrapRepeatSelection}
                      >
                        <Repeat className="size-4" />
                        Wrap {repeatWrapCandidates.length} selected node
                        {repeatWrapCandidates.length === 1 ? "" : "s"}
                      </Button>
                    )}
                  </div>
                )}
                {selectedGraphNode.type === "sub_macro" && (
                  <div className="grid gap-3">
                    <label className="grid gap-1.5 text-xs text-muted-foreground">
                      Macro to import
                      <Select
                        value={String(selectedGraphNode.config.macro_name || "")}
                        disabled={macroImportBusy === selectedId}
                        onValueChange={(name) => void importMacroSnapshot(name)}
                      >
                        <SelectTrigger className="w-full bg-background" aria-label="Macro to import">
                          <SelectValue placeholder="Choose a macro…" />
                        </SelectTrigger>
                        <SelectContent position="popper" align="start">
                          {macros.map((macro) => (
                            <SelectItem key={macro.name} value={macro.name}>
                              {macro.name}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                    </label>

                    {macroImportBusy === selectedId ? (
                      <div className="flex items-center gap-2 rounded-md border border-border bg-muted/25 px-3 py-3 text-xs text-muted-foreground">
                        <SpinnerGap className="size-4 animate-spin text-primary" />
                        Importing actions…
                      </div>
                    ) : selectedEmbeddedSteps.length > 0 ? (
                      <div className="grid gap-3 rounded-md border border-border bg-muted/20 p-3">
                        <div className="flex items-start justify-between gap-3">
                          <div>
                            <p className="text-xs font-medium text-foreground">Independent copy</p>
                            <p className="mt-1 text-[10px] leading-4 text-muted-foreground">
                              {selectedEmbeddedSteps.length} action
                              {selectedEmbeddedSteps.length === 1 ? "" : "s"} embedded
                              {Number(selectedGraphNode.config.source_duration || 0) > 0
                                ? ` · ${Number(selectedGraphNode.config.source_duration).toFixed(1)}s`
                                : ""}
                            </p>
                          </div>
                          <Button
                            variant="outline"
                            size="sm"
                            aria-label="Re-import latest"
                            onClick={() =>
                              void importMacroSnapshot(
                                String(selectedGraphNode.config.macro_name || ""),
                              )
                            }
                          >
                            <ArrowClockwise className="size-4" />
                            Re-import
                          </Button>
                        </div>
                        <p className="text-[10px] leading-4 text-muted-foreground">
                          Later edits or deletion of the source macro won’t change this node.
                        </p>
                      </div>
                    ) : (
                      <p className="text-[10px] leading-4 text-muted-foreground">
                        Choosing a macro copies its actions into this node.
                      </p>
                    )}

                    <NumberField
                      label="Repeat count"
                      value={Number(selectedGraphNode.config.repeat ?? 1)}
                      min={1}
                      max={1000}
                      step={1}
                      onChange={(repeat) =>
                        updateConfig({
                          repeat: Math.max(1, Math.min(1000, Math.floor(repeat || 1))),
                        })
                      }
                    />
                  </div>
                )}
                {selectedGraphNode.type === "chain" && (
                  <div className="grid gap-4">
                    <div className="grid grid-cols-[minmax(0,1fr)_auto] items-end gap-2">
                      <label className="grid min-w-0 gap-1.5 text-xs text-muted-foreground">
                        Saved chain
                        <Select
                          value={String(selectedGraphNode.config.chain_id || "")}
                          disabled={chainBusy}
                          onValueChange={(chainId) => {
                            const chain = chains.find(
                              (item) => item.id === chainId,
                            );
                            updateConfig({
                              chain_id: chainId,
                              chain_name: chain?.name || "",
                            });
                          }}
                        >
                          <SelectTrigger className="w-full bg-background" aria-label="Chain">
                            <SelectValue placeholder="Choose a chain…" />
                          </SelectTrigger>
                          <SelectContent>
                            {chains
                              .filter((chain) => chain.id)
                              .map((chain) => (
                                <SelectItem key={chain.id} value={chain.id!}>
                                  {chain.name || "Untitled chain"}
                                </SelectItem>
                              ))}
                          </SelectContent>
                        </Select>
                      </label>
                      <Button
                        variant="outline"
                        size="icon"
                        aria-label="Create chain"
                        title="Create chain"
                        disabled={chainBusy}
                        onClick={() => void createChainForSelectedNode()}
                      >
                        {chainBusy ? (
                          <SpinnerGap className="size-4 animate-spin" />
                        ) : (
                          <Plus className="size-4" weight="bold" />
                        )}
                      </Button>
                    </div>

                    {selectedChain ? (
                      <ChainComposer
                        key={String(selectedChain.id)}
                        chain={selectedChain}
                        macros={macros}
                        disabled={chainBusy}
                        onSave={saveSelectedChain}
                      />
                    ) : (
                      <div className="border-l-2 border-primary/45 py-1 pl-3">
                        <p className="text-xs font-medium text-foreground">
                          Build a chain here
                        </p>
                        <p className="mt-1 text-[10px] leading-4 text-muted-foreground">
                          Create one, then add and reorder macros without leaving
                          this Loop.
                        </p>
                      </div>
                    )}

                    <p className="text-[10px] leading-4 text-muted-foreground">
                      The whole sequence runs here. Continue runs after every
                      macro finishes. On failure is available when recovery is enabled.
                    </p>
                  </div>
                )}
                {selectedGraphNode.type === "note" && (
                  <label className="grid gap-1.5 text-xs text-muted-foreground">
                    Note
                    <Input
                      className="h-9"
                      value={String(selectedGraphNode.config.text || "")}
                      placeholder="Add context for this workflow"
                      onChange={(event) => updateConfig({ text: event.target.value })}
                    />
                  </label>
                )}
                {selectedGraphNode.type === "stop" && (
                  <label className="flex items-center justify-between gap-3 rounded-lg border border-border px-3 py-2 text-xs">
                    Successful finish
                    <Switch
                      checked={selectedGraphNode.config.success !== false}
                      onCheckedChange={(success) => updateConfig({ success })}
                    />
                  </label>
                )}
                  </div>
                </div>
              </aside>,
              nodeCanvasRef.current,
            )}
        </div>

    </div>
  );
}
