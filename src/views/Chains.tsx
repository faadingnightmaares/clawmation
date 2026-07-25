import { useCallback, useEffect, useRef, useState } from "react";
import { animate } from "animejs";
import {
  CalendarClock,
  ChevronDown,
  ChevronUp,
  Copy,
  Download,
  Loader2,
  Pencil,
  Play,
  Plus,
  Square,
  Trash2,
  TriangleAlert,
  Upload,
  Workflow,
  X,
} from "lucide-react";

import {
  addChain,
  duplicateChain,
  exportChain,
  getChainDuration,
  getRunningChain,
  importChain,
  listChains,
  listMacros,
  removeChain,
  runChain,
  scheduleChain,
  stopChain,
  updateChain,
  validateChain,
  type Chain,
  type MacroListItem,
  type RunningChain,
} from "@/api";
import { fmtDur } from "@/format";
import { useStaggerIn } from "@/lib/anime";
import { notify } from "@/lib/toast";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Progress } from "@/components/ui/progress";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Separator } from "@/components/ui/separator";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import type { ViewProps } from "./types";

// The 500 ms heartbeat that keeps the running-chain banner in step with the run.
const POLL_MS = 500;

/** The editor's working copy of a chain, plus its own macro-picker selection. */
interface ChainEditorState {
  id: string | null;
  name: string;
  macro_names: string[];
  delay_between: number;
  repeat: number;
  selectedMacro: string;
}

/** A listed chain enriched with the per-row validation + duration. */
interface EnrichedChain extends Chain {
  missingMacros: string[];
  duration: number | null;
}

/** One-line plain-language summary shown under a chain's name. */
function chainSummary(c: EnrichedChain): string {
  const n = c.macro_names?.length ?? 0;
  const parts = [`${n} ${n === 1 ? "macro" : "macros"}`];
  if (c.delay_between) parts.push(`${c.delay_between}s between`);
  const rep = c.repeat ?? 1;
  if (rep === 0) parts.push("loops forever");
  else if (rep > 1) parts.push(`${rep}× through`);
  if (c.duration != null) parts.push(`~${fmtDur(c.duration)}`);
  return parts.join(" · ");
}

/** The Chains half of Autopilot — macros run back to back, optionally on a
 *  schedule. `Autopilot` owns the page header, so this starts at the content. */
export function Chains(_props: ViewProps) {
  const [chains, setChains] = useState<EnrichedChain[]>([]);
  const [macros, setMacros] = useState<MacroListItem[]>([]);
  const [loading, setLoading] = useState(true);
  const [editor, setEditor] = useState<ChainEditorState | null>(null);
  const [runningChain, setRunningChain] = useState<RunningChain | null>(null);

  // Schedule dialog state.
  const [scheduleFor, setScheduleFor] = useState<string | null>(null);
  const [scheduleKind, setScheduleKind] = useState<"interval" | "daily">("interval");
  const [scheduleMins, setScheduleMins] = useState("30");
  const [scheduleTime, setScheduleTime] = useState("09:00");
  const [scheduling, setScheduling] = useState(false);

  const listRef = useStaggerIn<HTMLDivElement>(chains.length);
  const bannerRef = useRef<HTMLDivElement>(null);

  // Validate each chain and total its duration — mirrors the old loadChains.
  const loadChains = useCallback(async () => {
    try {
      const list = await listChains();
      const enriched = await Promise.all(
        (list || []).map(async (c): Promise<EnrichedChain> => {
          try {
            const [v, d] = await Promise.all([
              validateChain(String(c.id)),
              getChainDuration(String(c.id)),
            ]);
            return {
              ...c,
              missingMacros: v.missing ?? [],
              duration: d.ok ? d.duration ?? null : null,
            };
          } catch {
            return { ...c, missingMacros: [], duration: null };
          }
        }),
      );
      setChains(enriched);
    } catch {
      setChains([]);
    } finally {
      setLoading(false);
    }
  }, []);

  // On mount: load chains + the macro library (for the picker and durations).
  useEffect(() => {
    let alive = true;
    void loadChains();
    void (async () => {
      try {
        const list = await listMacros();
        if (alive) setMacros(list);
      } catch {
        /* picker just starts empty until macros arrive */
      }
    })();
    return () => {
      alive = false;
    };
  }, [loadChains]);

  // Poll the running chain so the progress banner tracks the live run.
  useEffect(() => {
    let alive = true;
    const tick = async () => {
      try {
        const rc = await getRunningChain();
        if (alive) setRunningChain(rc);
      } catch {
        /* transient — retry next tick */
      }
    };
    void tick();
    const id = setInterval(tick, POLL_MS);
    return () => {
      alive = false;
      clearInterval(id);
    };
  }, []);

  const chainRunning = !!runningChain?.running;

  useEffect(() => {
    if (chainRunning && bannerRef.current) {
      animate(bannerRef.current, { opacity: [0, 1], translateY: [10, 0], duration: 420, ease: "out(3)" });
    }
  }, [chainRunning]);

  const macroDuration = (name: string): number | undefined =>
    macros.find((m) => m.name === name)?.duration;

  // ── Editor ────────────────────────────────────────────────────────────────
  const openNewChain = () =>
    setEditor({ id: null, name: "", macro_names: [], delay_between: 1.0, repeat: 1, selectedMacro: "" });

  const openEditChain = (c: EnrichedChain) =>
    setEditor({
      id: c.id ?? null,
      name: c.name ?? "",
      macro_names: [...(c.macro_names ?? [])],
      delay_between: c.delay_between ?? 0,
      repeat: c.repeat ?? 1,
      selectedMacro: "",
    });

  const patchEditor = (patch: Partial<ChainEditorState>) =>
    setEditor((e) => (e ? { ...e, ...patch } : e));

  const addMacroToEditor = (name: string) => {
    if (!name) return;
    setEditor((e) => (e ? { ...e, macro_names: [...e.macro_names, name], selectedMacro: "" } : e));
  };

  const removeMacroFromEditor = (idx: number) =>
    setEditor((e) => (e ? { ...e, macro_names: e.macro_names.filter((_, i) => i !== idx) } : e));

  const moveMacroInEditor = (idx: number, dir: number) =>
    setEditor((e) => {
      if (!e) return e;
      const names = [...e.macro_names];
      const j = idx + dir;
      if (j < 0 || j >= names.length) return e;
      [names[idx], names[j]] = [names[j], names[idx]];
      return { ...e, macro_names: names };
    });

  const saveEditor = async () => {
    const ed = editor;
    if (!ed) return;
    if (!ed.name.trim()) {
      notify("error", "Give your chain a name.");
      return;
    }
    if (!ed.macro_names.length) {
      notify("error", "Add at least one macro to the chain.");
      return;
    }
    try {
      const res = ed.id
        ? await updateChain(ed.id, ed.name.trim(), ed.macro_names, ed.delay_between, ed.repeat)
        : await addChain(ed.name.trim(), ed.macro_names, ed.delay_between, ed.repeat);
      if (res.ok) {
        notify("success", `“${ed.name.trim()}” saved.`);
        setEditor(null);
        await loadChains();
      } else {
        notify("error", res.error || "Couldn’t save the chain.");
      }
    } catch (e) {
      notify("error", String(e));
    }
  };

  // ── Row actions ─────────────────────────────────────────────────────────────
  const handleRun = async (chainId: string, name: string) => {
    try {
      const res = await runChain(chainId);
      if ((res as { ok?: boolean }).ok) notify("success", `Playing “${name}” now.`);
      else notify("error", "Couldn’t start — something may already be running.");
    } catch (e) {
      notify("error", String(e));
    }
  };

  const handleDelete = async (chainId: string) => {
    try {
      await removeChain(chainId);
      notify("info", "Chain removed.");
      await loadChains();
    } catch {
      notify("error", "Couldn’t remove the chain.");
    }
  };

  const handleDuplicate = async (chainId: string) => {
    try {
      const res = await duplicateChain(chainId);
      if (res.ok) {
        notify("success", `Duplicated as “${res.name ?? "copy"}”.`);
        await loadChains();
      } else {
        notify("error", res.error || "Couldn’t duplicate the chain.");
      }
    } catch (e) {
      notify("error", String(e));
    }
  };

  const handleExport = async (chainId: string) => {
    try {
      const res = await exportChain(chainId);
      if (res.ok) notify("success", "Chain saved to a file.");
      else if (res.error && res.error !== "cancelled") notify("error", res.error);
    } catch (e) {
      notify("error", String(e));
    }
  };

  const handleImport = async () => {
    try {
      const res = await importChain();
      if (res.ok) {
        notify("success", `Imported “${res.name ?? "chain"}”.`);
        await loadChains();
      } else if (res.error && res.error !== "cancelled") {
        notify("error", res.error);
      }
    } catch (e) {
      notify("error", String(e));
    }
  };

  const handleStop = async () => {
    try {
      const res = await stopChain();
      if (res.ok) {
        notify("info", "Stopping the chain.");
        setRunningChain(null);
      }
    } catch (e) {
      notify("error", String(e));
    }
  };

  const openSchedule = (chainId: string) => {
    setScheduleKind("interval");
    setScheduleMins("30");
    setScheduleTime("09:00");
    setScheduleFor(chainId);
  };

  const confirmSchedule = async () => {
    if (scheduleFor === null) return;
    if (scheduleKind === "daily" && !scheduleTime) {
      notify("error", "Pick a time first.");
      return;
    }
    setScheduling(true);
    try {
      const intervalMin = scheduleKind === "interval" ? parseFloat(scheduleMins) || 30 : undefined;
      const atTime = scheduleKind === "daily" ? scheduleTime : undefined;
      const res = await scheduleChain(scheduleFor, scheduleKind, intervalMin, atTime);
      if (res.ok) {
        notify("success", "Scheduled — it’ll run on its own.");
        setScheduleFor(null);
      } else {
        notify("error", res.error || "Couldn’t schedule that.");
      }
    } catch (e) {
      notify("error", String(e));
    } finally {
      setScheduling(false);
    }
  };

  // ── Running-banner values (fields read straight off RunningChain) ──
  const runStep = (runningChain?.current_index ?? 0) + 1;
  const runTotal = runningChain?.total_macros ?? 0;
  const runHasIterations = !!(runningChain?.total_iterations && runningChain.total_iterations > 1);
  const runProgressPct = runTotal ? Math.round((runStep / runTotal) * 100) : 0;

  // ── Editor-derived values ──
  const editorNames = editor?.macro_names ?? [];
  const editorTotalDur = editorNames.length
    ? fmtDur(
        editorNames.reduce((sum, name) => sum + (macroDuration(name) || 0), 0) +
          (editorNames.length > 1 ? editorNames.length - 1 : 0) * (editor?.delay_between || 0),
      )
    : "";
  const availableMacros = editor
    ? macros.map((m) => m.name).filter((n) => !editor.macro_names.includes(n))
    : [];

  return (
    <div className="space-y-8">
      {/* Live run banner */}
      {chainRunning && (
        <div
          ref={bannerRef}
          className="relative overflow-hidden rounded-2xl border border-primary/40 bg-primary/[0.07] p-6"
        >
          <div className="flex flex-col items-start gap-5 sm:flex-row sm:items-center sm:justify-between">
            <div className="flex min-w-0 items-center gap-4">
              <span className="relative flex size-3 shrink-0">
                <span className="absolute inline-flex size-full animate-ping rounded-full bg-primary opacity-70" />
                <span className="relative inline-flex size-3 rounded-full bg-primary" />
              </span>
              <div className="min-w-0">
                <p className="truncate text-base font-semibold text-foreground">
                  Running “{runningChain?.name ?? ""}”
                </p>
                <p className="mt-0.5 truncate text-sm text-muted-foreground">
                  Step {runStep} of {runTotal}
                  {runningChain?.current_macro ? ` · ${runningChain.current_macro}` : ""}
                  {runHasIterations ? ` · round ${runningChain?.iteration ?? 1} of ${runningChain?.total_iterations}` : ""}
                </p>
              </div>
            </div>
            <Button variant="secondary" onClick={handleStop} className="shrink-0">
              <Square className="size-4" /> Stop
            </Button>
          </div>
          <Progress value={runProgressPct} className="mt-4" />
        </div>
      )}

      {/* Chain list */}
      <section className="space-y-4">
        <div className="flex items-center justify-between">
          <div>
            <h2 className="text-sm font-semibold text-foreground">Your chains</h2>
            <p className="text-xs text-muted-foreground">Each one plays its macros in order, top to bottom.</p>
          </div>
          {chains.length > 0 && (
            <div className="flex gap-2">
              <Button variant="outline" size="sm" onClick={handleImport}>
                <Upload className="size-4" /> Import
              </Button>
              <Button size="sm" onClick={openNewChain}>
                <Plus className="size-4" /> New chain
              </Button>
            </div>
          )}
        </div>

        {loading ? (
          <div className="flex items-center justify-center gap-2 py-16 text-sm text-muted-foreground">
            <Loader2 className="size-4 animate-spin" /> Loading…
          </div>
        ) : chains.length ? (
          <div ref={listRef} className="grid gap-4 lg:grid-cols-2">
            {chains.map((c) => {
              const id = String(c.id);
              const missing = new Set(c.missingMacros);
              return (
                <div key={id} className="flex flex-col gap-4 rounded-2xl border border-border bg-card p-5">
                  <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0">
                      <h3 className="truncate text-base font-semibold text-foreground">
                        {c.name ?? "Untitled chain"}
                      </h3>
                      <p className="mt-0.5 text-xs text-muted-foreground">{chainSummary(c)}</p>
                    </div>
                    <Button size="sm" onClick={() => handleRun(id, c.name ?? "chain")} className="shrink-0">
                      <Play className="size-4" /> Run
                    </Button>
                  </div>

                  {c.missingMacros.length > 0 && (
                    <div className="flex items-start gap-2 rounded-lg border border-destructive/30 bg-destructive/[0.06] p-2.5 text-xs text-destructive">
                      <TriangleAlert className="mt-0.5 size-3.5 shrink-0" />
                      <span>
                        Missing {c.missingMacros.length === 1 ? "a macro" : "macros"}: {c.missingMacros.join(", ")}. Edit
                        the chain to fix it.
                      </span>
                    </div>
                  )}

                  {(c.macro_names?.length ?? 0) > 0 && (
                    <div className="flex flex-wrap gap-1.5">
                      {c.macro_names!.map((name, i) => (
                        <span
                          key={`${name}-${i}`}
                          className={cn(
                            "inline-flex items-center gap-1.5 rounded-md border px-2 py-1 text-xs",
                            missing.has(name)
                              ? "border-destructive/40 bg-destructive/[0.06] text-destructive"
                              : "border-border bg-secondary/50 text-muted-foreground",
                          )}
                        >
                          <span className="tabular-nums opacity-60">{i + 1}</span>
                          {name}
                        </span>
                      ))}
                    </div>
                  )}

                  <Separator />

                  <div className="flex flex-wrap gap-1">
                    <Button variant="ghost" size="sm" className="text-muted-foreground" onClick={() => openSchedule(id)}>
                      <CalendarClock className="size-4" /> Schedule
                    </Button>
                    <Button variant="ghost" size="sm" className="text-muted-foreground" onClick={() => openEditChain(c)}>
                      <Pencil className="size-4" /> Edit
                    </Button>
                    <Button variant="ghost" size="sm" className="text-muted-foreground" onClick={() => handleDuplicate(id)}>
                      <Copy className="size-4" /> Duplicate
                    </Button>
                    <Button variant="ghost" size="sm" className="text-muted-foreground" onClick={() => handleExport(id)}>
                      <Download className="size-4" /> Export
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      className="ml-auto text-muted-foreground hover:text-destructive"
                      onClick={() => handleDelete(id)}
                    >
                      <Trash2 className="size-4" /> Delete
                    </Button>
                  </div>
                </div>
              );
            })}
          </div>
        ) : (
          <div className="flex flex-col items-center gap-5 rounded-2xl border border-dashed border-border px-6 py-16 text-center">
            <div className="flex size-14 items-center justify-center rounded-full bg-secondary text-muted-foreground">
              <Workflow className="size-7" />
            </div>
            <div>
              <h3 className="text-lg font-semibold text-foreground">No chains yet</h3>
              <p className="mx-auto mt-1 max-w-sm text-sm text-muted-foreground">
                A chain runs a few macros back to back — great for multi-step routines you’d rather not babysit. Pick
                your macros, set the order, and let it go.
              </p>
            </div>
            <div className="flex gap-2">
              <Button size="lg" onClick={openNewChain}>
                <Plus className="size-4" /> Build your first chain
              </Button>
              <Button size="lg" variant="outline" onClick={handleImport}>
                <Upload className="size-4" /> Import one
              </Button>
            </div>
          </div>
        )}
      </section>

      {/* Editor */}
      <Sheet open={!!editor} onOpenChange={(o) => !o && setEditor(null)}>
        <SheetContent className="flex w-full flex-col gap-0 p-0 sm:max-w-lg">
          {editor && (
            <>
              <SheetHeader className="px-6 pb-2 pr-12 pt-6">
                <SheetTitle>{editor.id ? "Edit chain" : "New chain"}</SheetTitle>
                <SheetDescription>Run several macros back to back, in the order you set.</SheetDescription>
              </SheetHeader>

              <div className="flex-1 space-y-6 overflow-y-auto px-6 py-5">
                <div className="space-y-1.5">
                  <Label htmlFor="chain-name">Name</Label>
                  <Input
                    id="chain-name"
                    value={editor.name}
                    onChange={(e) => patchEditor({ name: e.target.value })}
                    placeholder="Morning routine"
                    autoComplete="off"
                  />
                </div>

                <div className="grid grid-cols-2 gap-4">
                  <div className="space-y-1.5">
                    <Label htmlFor="chain-delay">Pause between macros</Label>
                    <div className="flex items-center gap-2">
                      <Input
                        id="chain-delay"
                        value={editor.delay_between}
                        onChange={(e) => patchEditor({ delay_between: parseFloat(e.target.value) || 0 })}
                        inputMode="decimal"
                        className="w-20"
                      />
                      <span className="text-xs text-muted-foreground">seconds</span>
                    </div>
                  </div>
                  <div className="space-y-1.5">
                    <Label htmlFor="chain-repeat">Repeat</Label>
                    <div className="flex items-center gap-2">
                      <Input
                        id="chain-repeat"
                        value={editor.repeat}
                        onChange={(e) => patchEditor({ repeat: parseInt(e.target.value) || 0 })}
                        inputMode="numeric"
                        className="w-20"
                      />
                      <span className="text-xs text-muted-foreground">times · 0 = forever</span>
                    </div>
                  </div>
                </div>

                <section className="space-y-2">
                  <div className="flex items-center justify-between">
                    <h3 className="text-sm font-semibold text-foreground">Macros in order</h3>
                    {editorTotalDur && (
                      <span className="text-xs text-muted-foreground">about {editorTotalDur} total</span>
                    )}
                  </div>

                  {editor.macro_names.length ? (
                    <div className="space-y-2">
                      {editor.macro_names.map((name, i) => (
                        <div
                          key={`${name}-${i}`}
                          className="flex items-center gap-2 rounded-lg border border-border bg-card p-2.5"
                        >
                          <span className="flex size-6 shrink-0 items-center justify-center rounded-md bg-secondary text-xs tabular-nums text-muted-foreground">
                            {i + 1}
                          </span>
                          <span className="min-w-0 flex-1 truncate text-sm text-foreground">{name}</span>
                          <span className="text-xs tabular-nums text-muted-foreground">
                            {fmtDur(macroDuration(name) || 0)}
                          </span>
                          <Button
                            variant="ghost"
                            size="sm"
                            className="px-1.5 text-muted-foreground"
                            disabled={i === 0}
                            onClick={() => moveMacroInEditor(i, -1)}
                            title="Move up"
                          >
                            <ChevronUp className="size-4" />
                          </Button>
                          <Button
                            variant="ghost"
                            size="sm"
                            className="px-1.5 text-muted-foreground"
                            disabled={i === editor.macro_names.length - 1}
                            onClick={() => moveMacroInEditor(i, 1)}
                            title="Move down"
                          >
                            <ChevronDown className="size-4" />
                          </Button>
                          <Button
                            variant="ghost"
                            size="sm"
                            className="px-1.5 text-muted-foreground hover:text-destructive"
                            onClick={() => removeMacroFromEditor(i)}
                            title="Remove"
                          >
                            <X className="size-4" />
                          </Button>
                        </div>
                      ))}
                    </div>
                  ) : (
                    <p className="rounded-lg border border-dashed border-border px-3 py-4 text-center text-xs text-muted-foreground">
                      No macros yet — add one below to start the sequence.
                    </p>
                  )}

                  <div className="flex gap-2 pt-1">
                    <Select
                      value={editor.selectedMacro}
                      onValueChange={(v) => patchEditor({ selectedMacro: v })}
                      disabled={availableMacros.length === 0}
                    >
                      <SelectTrigger className="flex-1">
                        <SelectValue placeholder={availableMacros.length ? "Add a macro…" : "No more macros to add"} />
                      </SelectTrigger>
                      <SelectContent>
                        {availableMacros.map((name) => (
                          <SelectItem key={name} value={name}>
                            {name}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                    <Button
                      variant="secondary"
                      onClick={() => addMacroToEditor(editor.selectedMacro)}
                      disabled={!editor.selectedMacro}
                    >
                      <Plus className="size-4" /> Add
                    </Button>
                  </div>
                </section>
              </div>

              <div className="flex justify-end gap-2 border-t border-border px-6 py-4">
                <Button variant="outline" onClick={() => setEditor(null)}>
                  Cancel
                </Button>
                <Button onClick={saveEditor}>Save chain</Button>
              </div>
            </>
          )}
        </SheetContent>
      </Sheet>

      {/* Schedule */}
      <Dialog open={scheduleFor !== null} onOpenChange={(o) => !o && setScheduleFor(null)}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>Schedule this chain</DialogTitle>
            <DialogDescription>Let it run on its own, on a rhythm you choose.</DialogDescription>
          </DialogHeader>

          <div className="space-y-4">
            <div className="grid grid-cols-2 gap-2">
              <Button
                variant={scheduleKind === "interval" ? "default" : "outline"}
                onClick={() => setScheduleKind("interval")}
              >
                Every so often
              </Button>
              <Button
                variant={scheduleKind === "daily" ? "default" : "outline"}
                onClick={() => setScheduleKind("daily")}
              >
                At a set time
              </Button>
            </div>

            {scheduleKind === "interval" ? (
              <div className="flex items-center gap-2 text-sm text-muted-foreground">
                <span>Every</span>
                <Input
                  value={scheduleMins}
                  onChange={(e) => setScheduleMins(e.target.value)}
                  inputMode="numeric"
                  aria-label="minutes"
                  className="w-20"
                />
                <span>minutes</span>
              </div>
            ) : (
              <div className="flex items-center gap-2 text-sm text-muted-foreground">
                <span>At</span>
                <Input
                  type="time"
                  value={scheduleTime}
                  onChange={(e) => setScheduleTime(e.target.value)}
                  aria-label="time of day"
                  className="w-32"
                />
                <span>each day</span>
              </div>
            )}
          </div>

          <DialogFooter>
            <Button variant="outline" onClick={() => setScheduleFor(null)}>
              Cancel
            </Button>
            <Button onClick={confirmSchedule} disabled={scheduling}>
              {scheduling ? <Loader2 className="size-4 animate-spin" /> : <CalendarClock className="size-4" />}
              Schedule it
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
