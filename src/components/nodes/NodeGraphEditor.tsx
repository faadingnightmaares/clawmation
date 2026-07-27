import { useCallback, useEffect, useMemo, useState } from "react";
import {
  Background,
  Controls,
  Handle,
  MarkerType,
  Position,
  ReactFlow,
  addEdge,
  useEdgesState,
  useNodesState,
  type Connection,
  type Edge,
  type NodeProps,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import {
  CheckCircle,
  Crosshair,
  CursorClick,
  Eye,
  FloppyDisk,
  GitBranch,
  Keyboard,
  MouseScroll,
  Play,
  PlayCircle,
  Repeat,
  SpinnerGap,
  StopCircle,
  TextT,
  Timer,
  Trash,
  WarningCircle,
} from "@phosphor-icons/react";

import {
  guardPickColor,
  nodeGraphLoad,
  nodeGraphRun,
  nodeGraphSave,
  nodeGraphValidate,
  type GraphNode,
  type Step,
} from "@/api";
import {
  OUTPUTS,
  createGraphNode,
  flowToGraph,
  graphToFlow,
  type MacroFlowNode,
} from "@/lib/nodeGraph";
import { notify } from "@/lib/toast";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";

const NODE_ICONS = {
  start: PlayCircle,
  action: CursorClick,
  vision: Eye,
  branch: GitBranch,
  loop: Repeat,
  sub_macro: Play,
  stop: StopCircle,
};

function nodeSummary(node: GraphNode): string {
  if (node.type === "action" || node.type === "vision") {
    const step = node.config.step as Step | undefined;
    if (!step) return "Invalid action";
    if (step.type === "click") return `${step.x}, ${step.y}`;
    if (step.type === "key") return step.key || "Choose a key";
    if (step.type === "type") return step.text || "Enter text";
    if (step.type === "scroll") return `${step.scroll_amount > 0 ? "+" : ""}${step.scroll_amount}`;
    if (step.type === "delay") return `${step.delay}s`;
    if (step.type === "wait_for") return `Timeout ${step.timeout}s`;
    return step.type === "find_click" ? "Find colour and click" : step.type;
  }
  if (node.type === "branch") return String(node.config.condition || "last_ok").replace(/_/g, " ");
  if (node.type === "loop") {
    const count = Number(node.config.count ?? 1);
    return count === 0 ? "Until stopped" : `${count} times`;
  }
  if (node.type === "sub_macro") return String(node.config.macro_name || "Choose a macro");
  if (node.type === "stop") return node.config.success === false ? "Failure" : "Success";
  return "Entry point";
}

function MacroNodeCard({ data, selected }: NodeProps<MacroFlowNode>) {
  const node = data.graphNode;
  const Icon = NODE_ICONS[node.type];
  const outputs = OUTPUTS[node.type];
  return (
    <div
      className={cn(
        "relative min-w-[190px] rounded-lg border bg-card shadow-sm transition-colors",
        selected ? "border-primary shadow-md" : "border-border",
        !node.enabled && "opacity-50",
      )}
    >
      {node.type !== "start" && (
        <Handle
          type="target"
          position={Position.Left}
          id="in"
          className="!size-3 !border-2 !border-background !bg-muted-foreground"
        />
      )}
      <div className="flex items-center gap-2.5 border-b border-border px-3 py-2.5">
        <span
          className={cn(
            "grid size-7 place-items-center rounded-md",
            node.type === "vision" || node.type === "branch"
              ? "bg-primary/10 text-primary"
              : "bg-secondary text-muted-foreground",
          )}
        >
          <Icon className="size-4" weight="bold" />
        </span>
        <div className="min-w-0">
          <p className="truncate text-xs font-semibold text-foreground">{node.label}</p>
          <p className="truncate text-[10px] text-muted-foreground">{nodeSummary(node)}</p>
        </div>
      </div>
      {outputs.length > 0 && (
        <div className="grid gap-1 px-3 py-2">
          {outputs.map((output) => (
            <div
              key={output.id}
              className={cn(
                "relative text-right text-[9px] font-medium",
                ["error", "missing", "false"].includes(output.id)
                  ? "text-destructive/80"
                  : "text-muted-foreground",
              )}
            >
              {output.label}
              <Handle
                type="source"
                position={Position.Right}
                id={output.id}
                style={{ top: "50%", right: "-18px" }}
                className="!size-3 !border-2 !border-background !bg-primary"
              />
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

const nodeTypes = { macro: MacroNodeCard };

const PALETTE = [
  { kind: "click", label: "Click", Icon: CursorClick },
  { kind: "key", label: "Key", Icon: Keyboard },
  { kind: "type", label: "Type", Icon: TextT },
  { kind: "scroll", label: "Scroll", Icon: MouseScroll },
  { kind: "delay", label: "Wait", Icon: Timer },
  { kind: "find_click", label: "Find", Icon: Crosshair },
  { kind: "wait_for", label: "Watch", Icon: Eye },
  { kind: "branch", label: "Branch", Icon: GitBranch },
  { kind: "loop", label: "Loop", Icon: Repeat },
  { kind: "sub_macro", label: "Macro", Icon: Play },
  { kind: "stop", label: "Stop", Icon: StopCircle },
];

function NumberField({
  label,
  value,
  onChange,
  min,
}: {
  label: string;
  value: number;
  onChange: (value: number) => void;
  min?: number;
}) {
  return (
    <label className="grid gap-1.5 text-xs text-muted-foreground">
      {label}
      <Input
        className="h-9"
        type="number"
        min={min}
        value={value}
        onChange={(event) => onChange(Number(event.target.value))}
      />
    </label>
  );
}

interface NodeGraphEditorProps {
  macroName: string;
  onChanged?: () => void;
}

export function NodeGraphEditor({ macroName, onChanged }: NodeGraphEditorProps) {
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

  const selected = useMemo(
    () => nodes.find((node) => node.id === selectedId) ?? null,
    [nodes, selectedId],
  );

  const buildGraph = useCallback(
    () => flowToGraph(macroName, entry, nodes, edges),
    [macroName, entry, nodes, edges],
  );

  useEffect(() => {
    let active = true;
    setLoading(true);
    setIssues({ errors: [], warnings: [] });
    void nodeGraphLoad(macroName)
      .then((result) => {
        if (!active) return;
        if (!result.ok || !result.graph) {
          notify("error", result.error || "Couldn’t load this node graph.");
          return;
        }
        const flow = graphToFlow(result.graph);
        setNodes(flow.nodes);
        setEdges(flow.edges);
        setEntry(result.graph.entry);
        setSelectedId(result.graph.entry);
      })
      .catch((error) => {
        if (active) notify("error", String(error));
      })
      .finally(() => {
        if (active) setLoading(false);
      });
    return () => {
      active = false;
    };
  }, [macroName, setEdges, setNodes]);

  const onConnect = useCallback(
    (connection: Connection) => {
      if (!connection.source || !connection.target || connection.source === connection.target) return;
      const output = connection.sourceHandle || "next";
      setEdges((current) =>
        addEdge(
          {
            ...connection,
            id: `edge-${crypto.randomUUID().slice(0, 8)}`,
            type: "smoothstep",
          },
          current.filter(
            (edge) => !(edge.source === connection.source && edge.sourceHandle === output),
          ),
        ),
      );
    },
    [setEdges],
  );

  const addNode = (kind: string) => {
    const graphNode = createGraphNode(kind, {
      x: 280 + (nodes.length % 4) * 250,
      y: 100 + Math.floor(nodes.length / 4) * 180,
    });
    setNodes((current) => [
      ...current,
      {
        id: graphNode.id,
        type: "macro",
        position: graphNode.position,
        data: { graphNode },
      },
    ]);
    setSelectedId(graphNode.id);
  };

  const updateSelected = (patch: Partial<GraphNode>) => {
    if (!selectedId) return;
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

  const removeSelected = () => {
    if (!selected || selected.data.graphNode.type === "start") return;
    setNodes((current) => current.filter((node) => node.id !== selected.id));
    setEdges((current) =>
      current.filter((edge) => edge.source !== selected.id && edge.target !== selected.id),
    );
    setSelectedId(null);
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

  const validate = async (): Promise<boolean> => {
    setBusy("validate");
    try {
      const report = await nodeGraphValidate(buildGraph());
      setIssues({ errors: report.errors, warnings: report.warnings });
      if (!report.ok) notify("error", report.errors[0] || "The graph is incomplete.");
      return report.ok;
    } catch (error) {
      notify("error", String(error));
      return false;
    } finally {
      setBusy(null);
    }
  };

  const save = async () => {
    if (!(await validate())) return;
    setBusy("save");
    try {
      const result = await nodeGraphSave(macroName, buildGraph());
      if (result.ok) {
        notify("success", "Node graph saved.");
        onChanged?.();
      } else {
        notify("error", result.error || "Couldn’t save the node graph.");
      }
    } finally {
      setBusy(null);
    }
  };

  const run = async () => {
    if (!(await validate())) return;
    setBusy("run");
    try {
      const result = await nodeGraphRun(buildGraph());
      if (result.ok) notify("success", "Node graph is running.");
      else notify("error", result.error || "Couldn’t run the node graph.");
    } finally {
      setBusy(null);
    }
  };

  const selectedGraphNode = selected?.data.graphNode;
  const selectedStep = selectedGraphNode?.config.step as Step | undefined;

  return (
    <div className="flex h-full min-h-0 min-w-0 flex-col bg-background">
        <div className="flex min-h-12 shrink-0 items-center gap-1.5 overflow-x-auto border-b border-border px-3 py-2">
          <span className="max-w-40 shrink-0 truncate text-xs font-semibold text-foreground">
            {macroName}
          </span>
          <span className="mx-1 h-5 w-px shrink-0 bg-border" />
          <span className="mr-1 shrink-0 text-xs font-medium text-muted-foreground">Add</span>
          {PALETTE.map(({ kind, label, Icon }) => (
            <Button key={kind} variant="outline" size="sm" className="h-8 shrink-0 gap-1.5" onClick={() => addNode(kind)}>
              <Icon className="size-3.5" weight="bold" />
              {label}
            </Button>
          ))}
        </div>

        <div className="flex min-h-0 min-w-0 flex-1">
          <div className="relative min-w-0 flex-1 bg-background">
            {loading ? (
              <div className="absolute inset-0 z-10 grid place-items-center bg-background/80 text-sm text-muted-foreground">
                <span className="flex items-center gap-2">
                  <SpinnerGap className="size-4 animate-spin" /> Loading nodes…
                </span>
              </div>
            ) : (
              <ReactFlow
                nodes={nodes}
                edges={edges}
                nodeTypes={nodeTypes}
                onNodesChange={onNodesChange}
                onEdgesChange={onEdgesChange}
                onConnect={onConnect}
                onNodeClick={(_, node) => setSelectedId(node.id)}
                onPaneClick={() => setSelectedId(null)}
                fitView
                fitViewOptions={{ padding: 0.2 }}
                minZoom={0.2}
                maxZoom={1.8}
                defaultEdgeOptions={{
                  type: "smoothstep",
                  markerEnd: { type: MarkerType.ArrowClosed },
                  style: { strokeWidth: 1.5 },
                }}
                proOptions={{ hideAttribution: true }}
              >
                <Background gap={24} size={1} color="hsl(var(--border))" />
                <Controls showInteractive={false} />
              </ReactFlow>
            )}
          </div>

          <aside className="w-[310px] shrink-0 overflow-y-auto border-l border-border bg-card p-4">
            {selectedGraphNode ? (
              <div className="grid gap-4">
                <div className="flex items-center justify-between gap-2">
                  <div>
                    <p className="text-sm font-semibold">Node settings</p>
                    <p className="text-xs capitalize text-muted-foreground">
                      {selectedGraphNode.type.replace("_", " ")}
                    </p>
                  </div>
                  {selectedGraphNode.type !== "start" && (
                    <Button variant="ghost" size="sm" onClick={removeSelected} title="Delete node">
                      <Trash className="size-4" />
                    </Button>
                  )}
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
                  <label className="flex items-center justify-between gap-3 rounded-lg border border-border px-3 py-2 text-xs">
                    Enabled
                    <Switch
                      checked={selectedGraphNode.enabled}
                      onCheckedChange={(enabled) => updateSelected({ enabled })}
                    />
                  </label>
                )}

                {selectedStep?.type === "click" && (
                  <div className="grid grid-cols-2 gap-2">
                    <NumberField label="X" value={selectedStep.x} onChange={(x) => updateStep({ x })} />
                    <NumberField label="Y" value={selectedStep.y} onChange={(y) => updateStep({ y })} />
                  </div>
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
                  <div className="grid gap-3">
                    <Button variant="outline" size="sm" onClick={() => void pickColour()}>
                      <Crosshair className="size-4" /> Pick colour
                    </Button>
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
                    <select
                      className="h-9 rounded-md border border-input bg-background px-3 text-sm text-foreground"
                      value={String(selectedGraphNode.config.condition || "last_ok")}
                      onChange={(event) => updateConfig({ condition: event.target.value })}
                    >
                      <option value="last_ok">Last node succeeded</option>
                      <option value="last_failed">Last node failed</option>
                      <option value="always">Always true</option>
                      <option value="never">Always false</option>
                    </select>
                  </label>
                )}
                {selectedGraphNode.type === "loop" && (
                  <NumberField
                    label="Iterations · 0 means forever"
                    value={Number(selectedGraphNode.config.count ?? 1)}
                    min={0}
                    onChange={(count) => updateConfig({ count: Math.max(0, Math.floor(count)) })}
                  />
                )}
                {selectedGraphNode.type === "sub_macro" && (
                  <label className="grid gap-1.5 text-xs text-muted-foreground">
                    Macro name
                    <Input
                      className="h-9"
                      value={String(selectedGraphNode.config.macro_name || "")}
                      onChange={(event) => updateConfig({ macro_name: event.target.value })}
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
            ) : (
              <div className="grid place-items-center py-20 text-center">
                <div>
                  <GitBranch className="mx-auto size-8 text-muted-foreground/50" />
                  <p className="mt-3 text-sm font-medium">Select a node</p>
                  <p className="mt-1 text-xs text-muted-foreground">Its settings will appear here.</p>
                </div>
              </div>
            )}
          </aside>
        </div>

        <footer className="flex min-h-14 shrink-0 items-center gap-3 border-t border-border px-4 py-2.5">
          <div className="min-w-0 flex-1">
            {issues.errors.length > 0 ? (
              <p className="flex items-center gap-1.5 truncate text-xs text-destructive">
                <WarningCircle className="size-4 shrink-0" weight="fill" /> {issues.errors[0]}
              </p>
            ) : issues.warnings.length > 0 ? (
              <p className="flex items-center gap-1.5 truncate text-xs text-muted-foreground">
                <WarningCircle className="size-4 shrink-0" /> {issues.warnings[0]}
              </p>
            ) : (
              <p className="flex items-center gap-1.5 text-xs text-muted-foreground">
                <CheckCircle className="size-4" /> {nodes.length} nodes · {edges.length} connections
              </p>
            )}
          </div>
          <Button variant="outline" onClick={() => void validate()} disabled={busy !== null}>
            {busy === "validate" ? <SpinnerGap className="size-4 animate-spin" /> : <CheckCircle className="size-4" />}
            Check
          </Button>
          <Button variant="outline" onClick={() => void run()} disabled={busy !== null || nodes.length === 0}>
            {busy === "run" ? <SpinnerGap className="size-4 animate-spin" /> : <Play className="size-4" weight="fill" />}
            Run graph
          </Button>
          <Button onClick={() => void save()} disabled={busy !== null}>
            {busy === "save" ? <SpinnerGap className="size-4 animate-spin" /> : <FloppyDisk className="size-4" />}
            Save
          </Button>
        </footer>
    </div>
  );
}
