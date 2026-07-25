import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { animate } from "animejs";
import {
  BookmarkPlus,
  Copy,
  Download,
  Eye,
  FolderInput,
  Layers,
  ListVideo,
  MoreVertical,
  Package,
  Pause,
  Pencil,
  Play,
  Plus,
  Repeat,
  Search,
  Shield,
  SlidersHorizontal,
  Square,
  StickyNote,
  Tag,
  Trash2,
  Upload,
} from "lucide-react";

import {
  bulkDelete,
  bulkExport,
  createFromTemplate,
  deleteMacro,
  deleteTemplate,
  duplicateMacro,
  exportBundle,
  exportMacro,
  getAllGuardCounts,
  importBundle,
  importMacro,
  listMacros,
  listTemplates,
  pauseRecord,
  playMacro,
  renameMacro,
  saveAsTemplate,
  setCategory,
  setNotes,
  setRepeat,
  startRecord,
  stopPlayback,
  stopRecord,
  type MacroListItem,
  type TemplateItem,
} from "@/api";
import { STOPS, fmtAgo, fmtDur, fmtHotkey, repeatToIndex, repsFor } from "@/format";
import { notify } from "@/lib/toast";
import { cn } from "@/lib/utils";
import { useStaggerIn } from "@/lib/anime";
import type { ViewProps } from "./types";

import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";
import { Progress } from "@/components/ui/progress";
import { Checkbox } from "@/components/ui/checkbox";
import { Textarea } from "@/components/ui/textarea";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { GuardsSheet } from "@/components/editors/GuardsSheet";
import { StepsSheet } from "@/components/editors/StepsSheet";
import { CheckpointsSheet } from "@/components/editors/CheckpointsSheet";

const SPEEDS = ["0.25", "0.5", "1", "1.5", "2", "4"] as const;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const CANCELLED = "cancelled";

// Playback speed is a per-run argument to `play_macro`, not a stored macro field,
// so the chosen value lives here. Keeping it in localStorage is what stops a
// deliberate 2× from silently reverting to 1× the next time the app opens.
const SPEED_KEY = "clawmation.speeds";

function readSpeeds(): Record<string, string> {
  const raw = globalThis.localStorage?.getItem(SPEED_KEY);
  if (!raw) return {};
  try {
    const parsed: unknown = JSON.parse(raw);
    return parsed && typeof parsed === "object" ? (parsed as Record<string, string>) : {};
  } catch {
    return {}; // hand-edited or truncated storage — start over rather than crash the view
  }
}

type SortKey = "recent" | "name" | "plays" | "duration" | "events";

// ── The 3-2-1 pre-record overlay. anime.js pops each number as it lands. ───────
function Countdown({ n }: { n: number }) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (ref.current) {
      animate(ref.current, { scale: [1.7, 1], opacity: [0, 1], duration: 480, ease: "out(4)" });
    }
  }, [n]);
  return (
    <div className="fixed inset-0 z-[100] flex flex-col items-center justify-center bg-background/85 backdrop-blur-md">
      <p className="mb-4 text-xs font-medium uppercase tracking-[0.3em] text-muted-foreground">
        Get ready to record
      </p>
      <div key={n} ref={ref} className="text-[10rem] font-bold leading-none text-primary">
        {n}
      </div>
      <p className="mt-4 text-sm text-muted-foreground">Switch to your game window…</p>
    </div>
  );
}

// ── A single reusable text prompt (name / category / notes), replacing the old
//    window.prompt calls with something that matches the app. ──────────────────
interface PromptSpec {
  title: string;
  label: string;
  value: string;
  placeholder?: string;
  multiline?: boolean;
  submitLabel?: string;
  onSubmit: (value: string) => void;
}

function PromptDialog({ spec, onClose }: { spec: PromptSpec | null; onClose: () => void }) {
  const [value, setValue] = useState("");
  useEffect(() => setValue(spec?.value ?? ""), [spec]);
  if (!spec) return null;
  const submit = () => {
    onClose();
    spec.onSubmit(value.trim());
  };
  return (
    <Dialog open onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>{spec.title}</DialogTitle>
        </DialogHeader>
        <label className="text-sm font-medium text-muted-foreground">{spec.label}</label>
        {spec.multiline ? (
          <Textarea
            autoFocus
            rows={4}
            value={value}
            placeholder={spec.placeholder}
            onChange={(e) => setValue(e.target.value)}
          />
        ) : (
          <Input
            autoFocus
            value={value}
            placeholder={spec.placeholder}
            onChange={(e) => setValue(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && submit()}
          />
        )}
        <DialogFooter>
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button onClick={submit}>{spec.submitLabel ?? "Save"}</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export function Macros({ status }: ViewProps) {
  const mode = status?.mode ?? "idle";
  const [macros, setMacros] = useState<MacroListItem[]>([]);
  const [templates, setTemplates] = useState<TemplateItem[]>([]);
  const [guardCounts, setGuardCounts] = useState<Record<string, number>>({});
  const [speeds, setSpeeds] = useState<Record<string, string>>(readSpeeds);
  const [selected, setSelected] = useState<Set<string>>(new Set());

  const [query, setQuery] = useState("");
  const [sort, setSort] = useState<SortKey>("recent");
  const [categoryFilter, setCategoryFilter] = useState("all");

  const [countdown, setCountdown] = useState<number | null>(null);
  const [renaming, setRenaming] = useState<string | null>(null);
  const [renameValue, setRenameValue] = useState("");
  const [prompt, setPrompt] = useState<PromptSpec | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<string | null>(null);
  const [bulkConfirm, setBulkConfirm] = useState(false);

  const [guardsFor, setGuardsFor] = useState<string | null>(null);
  const [stepsFor, setStepsFor] = useState<string | null>(null);
  const [checkpointsFor, setCheckpointsFor] = useState<string | null>(null);

  const pendingRename = useRef<string | null>(null);
  const playingName = useRef<string | null>(null);
  const prevMode = useRef(mode);
  const [playingCard, setPlayingCard] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const [ms, ts, gc] = await Promise.all([listMacros(), listTemplates(), getAllGuardCounts()]);
      setMacros(ms ?? []);
      setTemplates(ts ?? []);
      setGuardCounts(gc?.ok ? (gc.counts ?? {}) : {});
    } catch {
      /* best-effort refresh */
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  // Single place that reacts to a run ending: refresh + arm rename after a
  // recording, toast + clear the playing card after a playback.
  useEffect(() => {
    const prev = prevMode.current;
    if (prev === mode) return;
    if ((prev === "recording" || prev === "paused") && mode === "idle") {
      load().then(() => {
        if (pendingRename.current) {
          setRenaming(pendingRename.current);
          setRenameValue(pendingRename.current);
          pendingRename.current = null;
        }
      });
    }
    if (prev === "playing" && mode === "idle") {
      if (playingName.current) {
        notify("success", `Finished playing “${playingName.current}”`);
        playingName.current = null;
      }
      setPlayingCard(null);
      load();
    }
    prevMode.current = mode;
  }, [mode, load]);

  const busy = mode !== "idle" || countdown !== null;

  // ── Recording ────────────────────────────────────────────────────────────
  const toggleRecord = async () => {
    if (mode === "recording" || mode === "paused") {
      try {
        const res = await stopRecord();
        if (res?.ok) {
          pendingRename.current = res.name ?? null;
          notify("success", `Saved “${res.name}” — ${res.events} actions`);
        } else {
          notify("error", res?.error || "Couldn't save the recording");
        }
      } catch {
        notify("error", "Couldn't stop recording");
      }
      return;
    }
    if (mode === "playing" || countdown !== null) return;
    for (let n = 3; n >= 1; n--) {
      setCountdown(n);
      await sleep(750);
    }
    setCountdown(null);
    try {
      const res = await startRecord();
      if (!res?.ok) notify("error", res?.error || "Couldn't start recording");
    } catch {
      notify("error", "Couldn't start recording");
    }
  };

  const setSpeed = (name: string, value: string) =>
    setSpeeds((prev) => {
      const next = { ...prev, [name]: value };
      globalThis.localStorage?.setItem(SPEED_KEY, JSON.stringify(next));
      return next;
    });

  const togglePause = async () => {
    try {
      await pauseRecord();
    } catch {
      notify("error", "Couldn't pause");
    }
  };

  // ── Playback ─────────────────────────────────────────────────────────────
  const play = async (m: MacroListItem) => {
    const reps = repsFor(repeatToIndex(m.loop, m.loop_count));
    const speed = parseFloat(speeds[m.name] ?? "1") || 1;
    playingName.current = m.name;
    setPlayingCard(m.name);
    try {
      const res = await playMacro(m.name, reps, speed);
      if (res?.ok) {
        const speedLabel = speed !== 1 ? ` at ${speed}×` : "";
        notify("info", `Playing “${m.name}”${speedLabel}`);
      } else {
        notify("error", res?.error || "Couldn't play that macro");
        setPlayingCard(null);
        playingName.current = null;
      }
    } catch {
      notify("error", "Couldn't play that macro");
      setPlayingCard(null);
      playingName.current = null;
    }
  };

  const stopPlay = async () => {
    try {
      await stopPlayback();
    } catch {
      /* the heartbeat will settle the UI */
    }
  };

  // ── Per-macro settings ───────────────────────────────────────────────────
  const changeRepeat = async (m: MacroListItem, idx: number) => {
    setMacros((prev) =>
      prev.map((x) =>
        x.name === m.name
          ? { ...x, loop: idx !== 1, loop_count: idx === 0 ? 0 : Number(STOPS[idx]) }
          : x,
      ),
    );
    try {
      await setRepeat(m.name, repsFor(idx));
    } catch {
      notify("error", "Couldn't change repeat");
      load();
    }
  };

  const commitRename = async () => {
    const from = renaming;
    const to = renameValue.trim();
    setRenaming(null);
    if (!from || !to || to === from) return;
    try {
      const res = await renameMacro(from, to);
      if (res?.ok) {
        notify("success", `Renamed to “${res.name ?? to}”`);
        load();
      } else {
        notify("error", res?.error || "Couldn't rename");
      }
    } catch {
      notify("error", "Couldn't rename");
    }
  };

  const onDuplicate = async (m: MacroListItem) => {
    try {
      const res = await duplicateMacro(m.name);
      if (res?.ok) {
        notify("success", `Copied to “${res.name}”`);
        load();
      } else notify("error", res?.error || "Couldn't duplicate");
    } catch {
      notify("error", "Couldn't duplicate");
    }
  };

  const doDelete = async (name: string) => {
    setDeleteTarget(null);
    try {
      const res = await deleteMacro(name);
      if (res?.ok !== false) {
        notify("info", `Deleted “${name}”`);
        setSelected((s) => {
          const next = new Set(s);
          next.delete(name);
          return next;
        });
        load();
      } else notify("error", res?.error || "Couldn't delete");
    } catch {
      notify("error", "Couldn't delete");
    }
  };

  const askCategory = (m: MacroListItem) =>
    setPrompt({
      title: "Category",
      label: "Group this macro under a label (leave blank to clear).",
      value: m.category ?? "",
      placeholder: "e.g. Farming",
      onSubmit: async (v) => {
        try {
          await setCategory(m.name, v);
          load();
        } catch {
          notify("error", "Couldn't set category");
        }
      },
    });

  const askNotes = (m: MacroListItem) =>
    setPrompt({
      title: "Notes",
      label: "A reminder for later — what this macro does, where to run it.",
      value: m.notes ?? "",
      placeholder: "Runs the daily reward loop…",
      multiline: true,
      onSubmit: async (v) => {
        try {
          await setNotes(m.name, v);
          load();
        } catch {
          notify("error", "Couldn't save notes");
        }
      },
    });

  const askSaveAsTemplate = (m: MacroListItem) =>
    setPrompt({
      title: "Save as preset",
      label: "Reuse this macro's setup as a starting point for new ones.",
      value: `${m.name} preset`,
      submitLabel: "Save preset",
      onSubmit: async (v) => {
        if (!v) return;
        try {
          const res = await saveAsTemplate(m.name, v);
          if (res?.ok) {
            notify("success", `Saved preset “${v}”`);
            load();
          } else notify("error", res?.error || "Couldn't save preset");
        } catch {
          notify("error", "Couldn't save preset");
        }
      },
    });

  const useTemplate = (t: TemplateItem) =>
    setPrompt({
      title: `New macro from “${t.name}”`,
      label: "Name the new macro.",
      value: t.name.replace(/ preset$/i, ""),
      submitLabel: "Create",
      onSubmit: async (v) => {
        if (!v) return;
        try {
          const res = await createFromTemplate(t.name, v);
          if (res?.ok) {
            notify("success", `Created “${res.name ?? v}”`);
            load();
          } else notify("error", res?.error || "Couldn't create macro");
        } catch {
          notify("error", "Couldn't create macro");
        }
      },
    });

  const removeTemplate = async (t: TemplateItem) => {
    try {
      await deleteTemplate(t.name);
      notify("info", `Removed preset “${t.name}”`);
      load();
    } catch {
      notify("error", "Couldn't remove preset");
    }
  };

  // ── Import / export ──────────────────────────────────────────────────────
  const onImportMacro = async () => {
    try {
      const res = await importMacro();
      if (res?.ok) {
        notify("success", `Imported “${res.name}”`);
        load();
      } else if (res?.error && res.error !== CANCELLED) notify("error", res.error);
    } catch {
      notify("error", "Import failed");
    }
  };

  const onImportBundle = async () => {
    try {
      const res = await importBundle();
      if (res?.ok) {
        notify("success", `Imported “${res.name}”`);
        load();
      } else if (res?.error && res.error !== CANCELLED) notify("error", res.error);
    } catch {
      notify("error", "Import failed");
    }
  };

  const onExportMacro = async (m: MacroListItem) => {
    try {
      const res = await exportMacro(m.name);
      if (res?.ok) notify("success", "Exported to file");
      else if (res?.error && res.error !== CANCELLED) notify("error", res.error);
    } catch {
      notify("error", "Export failed");
    }
  };

  const onExportBundle = async (m: MacroListItem) => {
    try {
      const res = await exportBundle(m.name);
      if (res?.ok) notify("success", "Exported bundle (macro + images)");
      else if (res?.error && res.error !== CANCELLED) notify("error", res.error);
    } catch {
      notify("error", "Export failed");
    }
  };

  // ── Bulk selection ───────────────────────────────────────────────────────
  const toggleSelect = (name: string) =>
    setSelected((s) => {
      const next = new Set(s);
      if (next.has(name)) next.delete(name);
      else next.add(name);
      return next;
    });

  const doBulkDelete = async () => {
    setBulkConfirm(false);
    const names = [...selected];
    try {
      await bulkDelete(names);
      notify("info", `Deleted ${names.length} macro${names.length === 1 ? "" : "s"}`);
      setSelected(new Set());
      load();
    } catch {
      notify("error", "Bulk delete failed");
    }
  };

  const onBulkExport = async () => {
    const names = [...selected];
    try {
      const res = await bulkExport(names);
      if (res?.ok) notify("success", `Exported ${res.exported?.length ?? names.length} macros`);
      else notify("error", res?.error || "Export failed");
    } catch {
      notify("error", "Export failed");
    }
  };

  // ── Derived list ─────────────────────────────────────────────────────────
  const categories = useMemo(
    () => Array.from(new Set(macros.map((m) => m.category).filter(Boolean))) as string[],
    [macros],
  );

  const visible = useMemo(() => {
    const q = query.trim().toLowerCase();
    const filtered = macros.filter((m) => {
      if (categoryFilter !== "all" && m.category !== categoryFilter) return false;
      if (!q) return true;
      return (
        m.name.toLowerCase().includes(q) ||
        (m.notes ?? "").toLowerCase().includes(q) ||
        (m.category ?? "").toLowerCase().includes(q)
      );
    });
    const cmp: Record<SortKey, (a: MacroListItem, b: MacroListItem) => number> = {
      recent: (a, b) => (b.last_played ?? 0) - (a.last_played ?? 0),
      name: (a, b) => a.name.localeCompare(b.name),
      plays: (a, b) => (b.play_count ?? 0) - (a.play_count ?? 0),
      duration: (a, b) => (b.duration ?? 0) - (a.duration ?? 0),
      events: (a, b) => (b.events ?? 0) - (a.events ?? 0),
    };
    return [...filtered].sort(cmp[sort]);
  }, [macros, query, categoryFilter, sort]);

  const listRef = useStaggerIn<HTMLDivElement>(`${visible.length}:${sort}:${categoryFilter}`);
  const recordHotkey = status?.config?.hotkey_record
    ? fmtHotkey(status.config.hotkey_record)
    : null;
  const recording = mode === "recording" || mode === "paused";

  return (
    <div className="flex flex-col gap-6">
      {countdown !== null && <Countdown n={countdown} />}

      {/* ── Record hero ─────────────────────────────────────────────────── */}
      <Card
        className={cn(
          "flex flex-col gap-5 p-6 transition-colors",
          recording && "border-destructive/40 bg-destructive/5",
        )}
      >
        {recording ? (
          <div className="flex flex-col gap-4 sm:flex-row sm:items-center sm:justify-between">
            <div className="flex items-center gap-4">
              <span className="relative flex size-3">
                <span
                  className={cn(
                    "absolute inline-flex size-full rounded-full bg-destructive opacity-70",
                    mode === "recording" && "animate-ping",
                  )}
                />
                <span className="relative inline-flex size-3 rounded-full bg-destructive" />
              </span>
              <div>
                <p className="font-mono text-3xl font-semibold tabular-nums">
                  {fmtDur(status?.elapsed ?? 0)}
                </p>
                <p className="text-sm text-muted-foreground">
                  {mode === "paused" ? "Paused" : "Recording"} · {status?.recorded_count ?? 0} action
                  {(status?.recorded_count ?? 0) === 1 ? "" : "s"} captured
                </p>
              </div>
            </div>
            <div className="flex items-center gap-2">
              <Button variant="outline" onClick={togglePause}>
                {mode === "paused" ? <Play className="size-4" /> : <Pause className="size-4" />}
                {mode === "paused" ? "Resume" : "Pause"}
              </Button>
              <Button variant="destructive" onClick={toggleRecord}>
                <Square className="size-4 fill-current" />
                Stop &amp; save
              </Button>
            </div>
          </div>
        ) : (
          <div className="flex flex-col gap-4 sm:flex-row sm:items-center sm:justify-between">
            <div className="flex items-center gap-4">
              <button
                type="button"
                onClick={toggleRecord}
                disabled={mode === "playing" || countdown !== null}
                className="group flex size-16 shrink-0 items-center justify-center rounded-full bg-primary text-primary-foreground shadow-sm transition hover:brightness-105 focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background disabled:opacity-40"
                title="Start recording"
              >
                <span className="size-5 rounded-full bg-current transition group-hover:scale-90" />
              </button>
              <div>
                <h1 className="text-lg font-semibold">Record a macro</h1>
                <p className="max-w-md text-sm text-muted-foreground">
                  Open your game, then press Record. Every click, key and pause is captured until
                  you stop
                  {recordHotkey ? (
                    <>
                      {" "}
                      — or press <Kbd>{recordHotkey}</Kbd> hands-free
                    </>
                  ) : null}
                  .
                </p>
              </div>
            </div>
            <div className="flex items-center gap-2">
              <Button variant="ghost" size="sm" onClick={onImportMacro}>
                <Upload className="size-4" />
                Import
              </Button>
              <Button variant="ghost" size="sm" onClick={onImportBundle}>
                <Package className="size-4" />
                Bundle
              </Button>
            </div>
          </div>
        )}
      </Card>

      {/* ── Toolbar ─────────────────────────────────────────────────────── */}
      {macros.length > 0 && (
        <div className="flex flex-wrap items-center gap-2">
          <div className="relative min-w-[200px] flex-1">
            <Search className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
            <Input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Search macros…"
              className="pl-9"
            />
          </div>
          {categories.length > 0 && (
            <Select value={categoryFilter} onValueChange={setCategoryFilter}>
              <SelectTrigger className="min-w-[150px]">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="all">All categories</SelectItem>
                {categories.map((c) => (
                  <SelectItem key={c} value={c}>
                    {c}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          )}
          <Select value={sort} onValueChange={(v) => setSort(v as SortKey)}>
            <SelectTrigger className="min-w-[150px]">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="recent">Recently played</SelectItem>
              <SelectItem value="name">Name</SelectItem>
              <SelectItem value="plays">Most played</SelectItem>
              <SelectItem value="duration">Longest</SelectItem>
              <SelectItem value="events">Most actions</SelectItem>
            </SelectContent>
          </Select>
        </div>
      )}

      {/* ── Bulk bar ────────────────────────────────────────────────────── */}
      {selected.size > 0 && (
        <div className="flex flex-wrap items-center gap-3 rounded-lg border border-primary/30 bg-primary/5 px-4 py-2.5">
          <span className="text-sm font-medium">{selected.size} selected</span>
          <div className="flex-1" />
          <Button variant="ghost" size="sm" onClick={onBulkExport}>
            <Download className="size-4" />
            Export
          </Button>
          <Button
            variant="ghost"
            size="sm"
            onClick={() => setBulkConfirm(true)}
            className="text-destructive hover:bg-destructive/10 hover:text-destructive"
          >
            <Trash2 className="size-4" />
            Delete
          </Button>
          <Button variant="ghost" size="sm" onClick={() => setSelected(new Set())}>
            Clear
          </Button>
        </div>
      )}

      {/* ── List / empty state ──────────────────────────────────────────── */}
      {macros.length === 0 ? (
        <EmptyState onRecord={toggleRecord} disabled={busy} />
      ) : visible.length === 0 ? (
        <p className="py-12 text-center text-sm text-muted-foreground">
          No macros match “{query}”.
        </p>
      ) : (
        <div ref={listRef} className="flex flex-col gap-3">
          {visible.map((m) => {
            const repeatIdx = repeatToIndex(m.loop, m.loop_count);
            const speed = speeds[m.name] ?? "1";
            const isPlaying = playingCard === m.name && mode === "playing";
            const guards = guardCounts[m.name] ?? 0;
            const meta: string[] = [
              `${m.events} action${m.events === 1 ? "" : "s"}`,
              fmtDur(m.duration),
            ];
            if (m.play_count) meta.push(`played ${m.play_count}×`);
            const ago = fmtAgo(m.last_played ?? 0);
            if (ago) meta.push(ago);
            return (
              <Card key={m.name} className="gap-0 overflow-hidden p-0">
                <div className="flex items-start gap-3 p-4">
                  <Checkbox
                    checked={selected.has(m.name)}
                    onCheckedChange={() => toggleSelect(m.name)}
                    className="mt-1"
                    aria-label={`Select ${m.name}`}
                  />
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2">
                      {renaming === m.name ? (
                        <Input
                          autoFocus
                          value={renameValue}
                          onChange={(e) => setRenameValue(e.target.value)}
                          onBlur={commitRename}
                          onKeyDown={(e) => {
                            if (e.key === "Enter") commitRename();
                            if (e.key === "Escape") setRenaming(null);
                          }}
                          className="h-7 max-w-xs"
                        />
                      ) : (
                        <button
                          type="button"
                          onClick={() => {
                            setRenaming(m.name);
                            setRenameValue(m.name);
                          }}
                          className="truncate rounded text-left font-medium hover:text-primary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                          title="Click to rename"
                        >
                          {m.name}
                        </button>
                      )}
                      {m.category && (
                        <Badge variant="secondary" className="shrink-0 font-normal">
                          {m.category}
                        </Badge>
                      )}
                      {guards > 0 && (
                        <Badge asChild variant="outline" className="shrink-0 font-normal">
                          <button
                            type="button"
                            onClick={() => setGuardsFor(m.name)}
                            title={`${guards} safety guard${guards === 1 ? "" : "s"} — click to edit`}
                            className="gap-1 text-muted-foreground hover:border-primary/50 hover:text-foreground"
                          >
                            <Shield className="size-3" />
                            {guards}
                          </button>
                        </Badge>
                      )}
                    </div>
                    <p className="mt-0.5 truncate text-sm text-muted-foreground">
                      {meta.join(" · ")}
                    </p>
                    {m.notes && (
                      <p className="mt-1 truncate text-xs text-muted-foreground/80">{m.notes}</p>
                    )}
                  </div>

                  <div className="flex shrink-0 items-center gap-2">
                    <RepeatChips value={repeatIdx} onChange={(i) => changeRepeat(m, i)} />
                    {isPlaying ? (
                      <Button variant="destructive" size="sm" onClick={stopPlay}>
                        <Square className="size-4 fill-current" />
                        Stop
                      </Button>
                    ) : (
                      <Button size="sm" onClick={() => play(m)} disabled={busy}>
                        <Play className="size-4 fill-current" />
                        {speed === "1" ? "Run" : `Run ${speed}×`}
                      </Button>
                    )}
                    <MacroMenu
                      speed={speed}
                      onSpeed={(v) => setSpeed(m.name, v)}
                      repeat={repeatIdx}
                      onRepeat={(i) => changeRepeat(m, i)}
                      onGuards={() => setGuardsFor(m.name)}
                      onSteps={() => setStepsFor(m.name)}
                      onCheckpoints={() => setCheckpointsFor(m.name)}
                      onDuplicate={() => onDuplicate(m)}
                      onRename={() => {
                        setRenaming(m.name);
                        setRenameValue(m.name);
                      }}
                      onCategory={() => askCategory(m)}
                      onNotes={() => askNotes(m)}
                      onSaveTemplate={() => askSaveAsTemplate(m)}
                      onExport={() => onExportMacro(m)}
                      onBundle={() => onExportBundle(m)}
                      onDelete={() => setDeleteTarget(m.name)}
                    />
                  </div>
                </div>
                {isPlaying && (
                  <div className="flex items-center gap-3 border-t border-border bg-secondary/30 px-4 py-2">
                    <Progress
                      value={
                        (status?.play_total_reps ?? 0) > 0
                          ? ((status?.play_iteration ?? 0) / (status?.play_total_reps ?? 1)) * 100
                          : undefined
                      }
                      className="h-1.5 flex-1"
                    />
                    <span className="shrink-0 text-xs font-medium text-primary">
                      {(status?.play_total_reps ?? 0) > 0
                        ? `Rep ${status?.play_iteration ?? 0} / ${status?.play_total_reps}`
                        : `Rep ${status?.play_iteration ?? 0} / ∞`}
                    </span>
                  </div>
                )}
              </Card>
            );
          })}
        </div>
      )}

      {/* ── Presets ─────────────────────────────────────────────────────── */}
      {templates.length > 0 && (
        <section className="flex flex-col gap-3">
          <div className="flex items-center gap-2 text-sm font-medium text-muted-foreground">
            <Layers className="size-4" />
            Presets
          </div>
          <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
            {templates.map((t) => (
              <Card key={t.name} className="flex flex-row items-center justify-between gap-2 p-3">
                <div className="min-w-0">
                  <p className="truncate text-sm font-medium">{t.name}</p>
                  <p className="text-xs text-muted-foreground">
                    {t.events} actions · {fmtDur(t.duration)}
                  </p>
                </div>
                <div className="flex shrink-0 items-center gap-1">
                  <Button size="sm" variant="secondary" onClick={() => useTemplate(t)}>
                    Use
                  </Button>
                  <Button
                    size="icon-sm"
                    variant="ghost"
                    onClick={() => removeTemplate(t)}
                    className="text-muted-foreground hover:text-destructive"
                    title="Remove preset"
                  >
                    <Trash2 className="size-4" />
                  </Button>
                </div>
              </Card>
            ))}
          </div>
        </section>
      )}

      {/* ── Overlays ────────────────────────────────────────────────────── */}
      <PromptDialog spec={prompt} onClose={() => setPrompt(null)} />

      <AlertDialog open={!!deleteTarget} onOpenChange={(o) => !o && setDeleteTarget(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete “{deleteTarget}”?</AlertDialogTitle>
            <AlertDialogDescription>
              This removes the macro and its guards. It can't be undone.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Keep it</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => deleteTarget && doDelete(deleteTarget)}
              className="bg-destructive text-white hover:bg-destructive/90"
            >
              Delete
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      <AlertDialog open={bulkConfirm} onOpenChange={setBulkConfirm}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete {selected.size} macros?</AlertDialogTitle>
            <AlertDialogDescription>This can't be undone.</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Keep them</AlertDialogCancel>
            <AlertDialogAction
              onClick={doBulkDelete}
              className="bg-destructive text-white hover:bg-destructive/90"
            >
              Delete all
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      {guardsFor && (
        <GuardsSheet
          macroName={guardsFor}
          open
          onOpenChange={(o) => !o && setGuardsFor(null)}
          onChanged={load}
        />
      )}
      {stepsFor && (
        <StepsSheet
          macroName={stepsFor}
          open
          onOpenChange={(o) => !o && setStepsFor(null)}
          onChanged={load}
        />
      )}
      {checkpointsFor && (
        <CheckpointsSheet
          macroName={checkpointsFor}
          open
          onOpenChange={(o) => !o && setCheckpointsFor(null)}
          onChanged={load}
        />
      )}
    </div>
  );
}

// ── Small pieces ─────────────────────────────────────────────────────────────

function Kbd({ children }: { children: React.ReactNode }) {
  return (
    <kbd className="rounded border border-border bg-muted px-1.5 py-0.5 font-mono text-xs font-medium text-foreground">
      {children}
    </kbd>
  );
}

function RepeatChips({ value, onChange }: { value: number; onChange: (i: number) => void }) {
  return (
    <div
      className="hidden items-center rounded-md border border-border p-0.5 md:flex"
      title="How many times to repeat"
    >
      {STOPS.map((s, i) => (
        <button
          key={s}
          type="button"
          onClick={() => onChange(i)}
          className={cn(
            "min-w-6 rounded px-1.5 py-0.5 text-xs font-medium transition-colors",
            value === i
              ? "bg-primary text-primary-foreground"
              : "text-muted-foreground hover:text-foreground",
          )}
        >
          {s}
        </button>
      ))}
    </div>
  );
}

interface MacroMenuProps {
  speed: string;
  onSpeed: (v: string) => void;
  repeat: number;
  onRepeat: (i: number) => void;
  onGuards: () => void;
  onSteps: () => void;
  onCheckpoints: () => void;
  onDuplicate: () => void;
  onRename: () => void;
  onCategory: () => void;
  onNotes: () => void;
  onSaveTemplate: () => void;
  onExport: () => void;
  onBundle: () => void;
  onDelete: () => void;
}

function MacroMenu(p: MacroMenuProps) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="ghost" size="icon-sm" className="text-muted-foreground">
          <MoreVertical className="size-4" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-56">
        <DropdownMenuItem onSelect={p.onGuards}>
          <Shield className="size-4" />
          Safety guards
        </DropdownMenuItem>
        <DropdownMenuItem onSelect={p.onCheckpoints}>
          <Eye className="size-4" />
          Vision checkpoints
        </DropdownMenuItem>
        <DropdownMenuItem onSelect={p.onSteps}>
          <SlidersHorizontal className="size-4" />
          Fine-tune actions
        </DropdownMenuItem>
        <DropdownMenuSub>
          <DropdownMenuSubTrigger>
            <Play className="size-4" />
            Speed
          </DropdownMenuSubTrigger>
          <DropdownMenuSubContent>
            <DropdownMenuRadioGroup value={p.speed} onValueChange={p.onSpeed}>
              {SPEEDS.map((s) => (
                <DropdownMenuRadioItem key={s} value={s}>
                  {s}×{s === "1" ? " (normal)" : ""}
                </DropdownMenuRadioItem>
              ))}
            </DropdownMenuRadioGroup>
          </DropdownMenuSubContent>
        </DropdownMenuSub>
        {/* The repeat chips on the card are md-and-up only; below that this is the
            only way to reach the setting at all. */}
        <DropdownMenuSub>
          <DropdownMenuSubTrigger className="md:hidden">
            <Repeat className="size-4" />
            Repeat
          </DropdownMenuSubTrigger>
          <DropdownMenuSubContent>
            <DropdownMenuRadioGroup
              value={String(p.repeat)}
              onValueChange={(v) => p.onRepeat(Number(v))}
            >
              {STOPS.map((s, i) => (
                <DropdownMenuRadioItem key={s} value={String(i)}>
                  {s === "∞" ? "Forever" : s === "1" ? "Once" : `${s} times`}
                </DropdownMenuRadioItem>
              ))}
            </DropdownMenuRadioGroup>
          </DropdownMenuSubContent>
        </DropdownMenuSub>
        <DropdownMenuSeparator />
        <DropdownMenuItem onSelect={p.onRename}>
          <Pencil className="size-4" />
          Rename
        </DropdownMenuItem>
        <DropdownMenuItem onSelect={p.onDuplicate}>
          <Copy className="size-4" />
          Duplicate
        </DropdownMenuItem>
        <DropdownMenuItem onSelect={p.onCategory}>
          <Tag className="size-4" />
          Category
        </DropdownMenuItem>
        <DropdownMenuItem onSelect={p.onNotes}>
          <StickyNote className="size-4" />
          Notes
        </DropdownMenuItem>
        <DropdownMenuItem onSelect={p.onSaveTemplate}>
          <BookmarkPlus className="size-4" />
          Save as preset
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem onSelect={p.onExport}>
          <Download className="size-4" />
          Export to file
        </DropdownMenuItem>
        <DropdownMenuItem onSelect={p.onBundle}>
          <FolderInput className="size-4" />
          Export bundle
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem
          onSelect={p.onDelete}
          className="text-destructive focus:bg-destructive/10 focus:text-destructive"
        >
          <Trash2 className="size-4" />
          Delete
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function EmptyState({ onRecord, disabled }: { onRecord: () => void; disabled: boolean }) {
  return (
    <Card className="flex flex-col items-center gap-5 px-6 py-14 text-center">
      <div className="flex size-14 items-center justify-center rounded-full bg-secondary text-muted-foreground">
        <ListVideo className="size-7" />
      </div>
      <div>
        <h2 className="text-lg font-semibold">No macros yet</h2>
        <p className="mx-auto mt-1 max-w-sm text-sm text-muted-foreground">
          A macro records what you do on screen so Clawmation can repeat it for you.
        </p>
      </div>
      <ol className="mx-auto flex max-w-md flex-col gap-2 text-left text-sm text-muted-foreground">
        <NumStep n={1}>Open the game or app you want to automate.</NumStep>
        <NumStep n={2}>Press Record and do the task once, yourself.</NumStep>
        <NumStep n={3}>Stop, then press Run to let it repeat.</NumStep>
      </ol>
      <Button size="lg" onClick={onRecord} disabled={disabled}>
        <Plus className="size-4" />
        Record your first macro
      </Button>
    </Card>
  );
}

function NumStep({ n, children }: { n: number; children: React.ReactNode }) {
  return (
    <li className="flex gap-3">
      <span className="flex size-5 shrink-0 items-center justify-center rounded-full bg-primary/15 text-xs font-semibold text-primary">
        {n}
      </span>
      {children}
    </li>
  );
}
