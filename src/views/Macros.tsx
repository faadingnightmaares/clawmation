import { useCallback, useEffect, useId, useMemo, useRef, useState } from "react";
import { animate } from "animejs";
import {
  BookmarkPlus,
  ChevronDown,
  Copy,
  Download,
  Eye,
  FolderInput,
  Layers,
  ListVideo,
  Package,
  Pause,
  Play,
  Plus,
  Search,
  Shield,
  Square,
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
import { reducedMotion, useStaggerIn } from "@/lib/anime";
import type { ViewProps } from "./types";

import { Button } from "@/components/ui/button";
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
import { CheckpointsSheet } from "@/components/editors/CheckpointsSheet";

const SPEEDS = ["0.25", "0.5", "1", "1.5", "2", "4"] as const;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const CANCELLED = "cancelled";

/** The repeat stops in words, for the places there is room for words. Index
 *  matches `STOPS`; the compact chips on a row show the glyph instead. */
const REPEAT_WORDS = ["Until I stop", "Once", "Twice", "3 times", "5 times", "10 times"];

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
    return {}; // hand-edited or truncated storage; start over rather than crash the view
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
      <p className="mb-4 text-sm text-muted-foreground">Get ready to record</p>
      <div key={n} ref={ref} className="text-[10rem] font-bold leading-none text-primary">
        {n}
      </div>
      <p className="mt-4 text-sm text-muted-foreground">Switch to your game window…</p>
    </div>
  );
}

// ── A single reusable text prompt, for the two actions that name a *new* thing
//    (everything a macro already owns is edited inline on its own row). ────────
interface PromptSpec {
  title: string;
  label: string;
  value: string;
  placeholder?: string;
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
        <label className="text-sm text-muted-foreground">{spec.label}</label>
        <Input
          autoFocus
          value={value}
          placeholder={spec.placeholder}
          onChange={(e) => setValue(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && submit()}
        />
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
  // The one row showing its settings. Only ever one: the panel is tall, and two
  // open at once turns the list back into the stack of cards this replaced.
  const [openRow, setOpenRow] = useState<string | null>(null);
  // Set only when the open row is a *just-recorded* macro, which is the one time
  // the name field should take focus by itself: it still has its generated name.
  const [nameFresh, setNameFresh] = useState(false);
  const [prompt, setPrompt] = useState<PromptSpec | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<string | null>(null);
  const [bulkConfirm, setBulkConfirm] = useState(false);

  const [guardsFor, setGuardsFor] = useState<string | null>(null);
  const [checkpointsFor, setCheckpointsFor] = useState<string | null>(null);

  const pendingRename = useRef<string | null>(null);
  const playingName = useRef<string | null>(null);
  const prevMode = useRef(mode);
  const [playingCard, setPlayingCard] = useState<string | null>(null);
  const searchRef = useRef<HTMLInputElement>(null);

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

  // Single place that reacts to a run ending: refresh + open the new recording's
  // row with its name ready to replace, toast + clear the playing row after a run.
  useEffect(() => {
    const prev = prevMode.current;
    if (prev === mode) return;
    if ((prev === "recording" || prev === "paused") && mode === "idle") {
      load().then(() => {
        if (pendingRename.current) {
          setOpenRow(pendingRename.current);
          setNameFresh(true);
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

  // "/" jumps to the search box, Escape closes the open row: the two things a
  // list this long is otherwise a mouse trip for.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const el = e.target as HTMLElement | null;
      const typing = !!el?.closest("input, textarea, [contenteditable='true']");
      if (e.key === "/" && !typing) {
        e.preventDefault();
        searchRef.current?.focus();
      }
      if (e.key === "Escape" && !typing) setOpenRow(null);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const busy = mode !== "idle" || countdown !== null;

  const toggleRow = (name: string) => {
    setNameFresh(false);
    setOpenRow((cur) => (cur === name ? null : name));
  };

  // ── Recording ────────────────────────────────────────────────────────────
  const toggleRecord = async () => {
    if (mode === "recording" || mode === "paused") {
      try {
        const res = await stopRecord();
        if (res?.ok) {
          pendingRename.current = res.name ?? null;
          notify("success", `Saved “${res.name}” with ${res.events} actions`);
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

  const persistSpeeds = (next: Record<string, string>) => {
    globalThis.localStorage?.setItem(SPEED_KEY, JSON.stringify(next));
    return next;
  };

  const setSpeed = (name: string, value: string) =>
    setSpeeds((prev) => persistSpeeds({ ...prev, [name]: value }));

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

  const rename = async (from: string, to: string) => {
    try {
      const res = await renameMacro(from, to);
      if (!res?.ok) {
        notify("error", res?.error || "Couldn't rename");
        return;
      }
      const name = res.name ?? to;
      // The row is keyed by name, so follow it: the settings panel stays open on
      // the macro the user is still editing, and its chosen speed goes with it.
      setOpenRow((cur) => (cur === from ? name : cur));
      setNameFresh(false);
      setSpeeds((prev) => {
        if (!prev[from]) return prev;
        const next = { ...prev, [name]: prev[from] };
        delete next[from];
        return persistSpeeds(next);
      });
      notify("success", `Renamed to “${name}”`);
      load();
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
        setOpenRow((cur) => (cur === name ? null : cur));
        load();
      } else notify("error", res?.error || "Couldn't delete");
    } catch {
      notify("error", "Couldn't delete");
    }
  };

  const saveCategory = async (m: MacroListItem, value: string) => {
    try {
      await setCategory(m.name, value);
      load();
    } catch {
      notify("error", "Couldn't set category");
    }
  };

  const saveNotes = async (m: MacroListItem, value: string) => {
    try {
      await setNotes(m.name, value);
      load();
    } catch {
      notify("error", "Couldn't save notes");
    }
  };

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

      {/* ── Header: what this is, and the one thing you came here to press ── */}
      <header className="flex flex-wrap items-start justify-between gap-4">
        <div className="space-y-1">
          <h1 className="text-2xl font-semibold tracking-tight text-foreground">Macros</h1>
          <p className="text-sm text-muted-foreground">
            {recording
              ? "Do the task once. Every click and key is being captured."
              : macros.length === 0
                ? "Record what you do once, and Clawmation repeats it for you."
                : `${macros.length} saved. Press Run on any of them.`}
            {!recording && recordHotkey ? (
              <>
                {" "}
                Or press <Kbd>{recordHotkey}</Kbd> without leaving your game.
              </>
            ) : null}
          </p>
        </div>

        {recording ? (
          <div className="flex items-center gap-3 rounded-xl border border-destructive/40 bg-destructive/5 px-3 py-2">
            <span className="relative flex size-2.5">
              <span
                className={cn(
                  "absolute inline-flex size-full rounded-full bg-destructive opacity-70",
                  mode === "recording" && "animate-ping",
                )}
              />
              <span className="relative inline-flex size-2.5 rounded-full bg-destructive" />
            </span>
            <div className="leading-tight">
              <p className="font-mono text-lg font-semibold tabular-nums">
                {fmtDur(status?.elapsed ?? 0)}
              </p>
              <p className="text-xs text-muted-foreground">
                {mode === "paused" ? "Paused" : "Recording"} · {status?.recorded_count ?? 0} action
                {(status?.recorded_count ?? 0) === 1 ? "" : "s"}
              </p>
            </div>
            <Button variant="outline" size="sm" onClick={togglePause}>
              {mode === "paused" ? <Play className="size-4" /> : <Pause className="size-4" />}
              {mode === "paused" ? "Resume" : "Pause"}
            </Button>
            <Button variant="destructive" size="sm" onClick={toggleRecord}>
              <Square className="size-4 fill-current" />
              Stop &amp; save
            </Button>
          </div>
        ) : (
          <div className="flex items-center gap-2">
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button variant="ghost" size="icon" title="Add a macro from a file">
                  <Plus className="size-5" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" className="w-56">
                <DropdownMenuItem onSelect={onImportMacro}>
                  <Upload className="size-4" />
                  Add a macro from a file
                </DropdownMenuItem>
                <DropdownMenuItem onSelect={onImportBundle}>
                  <Package className="size-4" />
                  Add a bundle (macro + images)
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
            <Button size="lg" onClick={toggleRecord} disabled={busy}>
              <span className="size-2.5 rounded-full bg-current" />
              Record
            </Button>
          </div>
        )}
      </header>

      {/* ── Search, category, sort ──────────────────────────────────────── */}
      {macros.length > 0 && (
        <div className="flex flex-col gap-3">
          <div className="flex flex-wrap items-center gap-2">
            <div className="relative min-w-[200px] flex-1">
              <Search className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
              <Input
                ref={searchRef}
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="Search macros…"
                className="pl-9"
              />
            </div>
            <Select value={sort} onValueChange={(v) => setSort(v as SortKey)}>
              <SelectTrigger className="w-[170px]">
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
          {/* One click to filter, where the old dropdown took two. Only ever
              rendered once the user has actually made categories. */}
          {categories.length > 0 && (
            <div className="flex flex-wrap items-center gap-1.5">
              <FilterPill
                active={categoryFilter === "all"}
                onClick={() => setCategoryFilter("all")}
              >
                All
              </FilterPill>
              {categories.map((c) => (
                <FilterPill
                  key={c}
                  active={categoryFilter === c}
                  onClick={() => setCategoryFilter(categoryFilter === c ? "all" : c)}
                >
                  {c}
                </FilterPill>
              ))}
            </div>
          )}
        </div>
      )}

      {/* ── Bulk bar ────────────────────────────────────────────────────── */}
      {selected.size > 0 && (
        <div className="flex flex-wrap items-center gap-3 rounded-xl border border-primary/30 bg-primary/5 px-4 py-2.5">
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

      {/* ── The list ────────────────────────────────────────────────────── */}
      {macros.length === 0 ? (
        <EmptyState onRecord={toggleRecord} disabled={busy} />
      ) : visible.length === 0 ? (
        <p className="py-12 text-center text-sm text-muted-foreground">
          No macros match “{query}”.
        </p>
      ) : (
        <div
          ref={listRef}
          className="divide-y divide-border overflow-hidden rounded-xl border border-border bg-card"
        >
          {visible.map((m) => (
            <MacroRow
              key={m.name}
              macro={m}
              guards={guardCounts[m.name] ?? 0}
              speed={speeds[m.name] ?? "1"}
              open={openRow === m.name}
              focusName={openRow === m.name && nameFresh}
              selected={selected.has(m.name)}
              playing={playingCard === m.name && mode === "playing"}
              iteration={status?.play_iteration ?? 0}
              totalReps={status?.play_total_reps ?? 0}
              busy={busy}
              onToggle={() => toggleRow(m.name)}
              onSelect={() => toggleSelect(m.name)}
              onRun={() => play(m)}
              onStop={stopPlay}
              onSpeed={(v) => setSpeed(m.name, v)}
              onRepeat={(i) => changeRepeat(m, i)}
              onRename={(to) => rename(m.name, to)}
              onCategory={(v) => saveCategory(m, v)}
              onNotes={(v) => saveNotes(m, v)}
              onGuards={() => setGuardsFor(m.name)}
              onCheckpoints={() => setCheckpointsFor(m.name)}
              onDuplicate={() => onDuplicate(m)}
              onSavePreset={() => askSaveAsTemplate(m)}
              onExport={() => onExportMacro(m)}
              onBundle={() => onExportBundle(m)}
              onDelete={() => setDeleteTarget(m.name)}
            />
          ))}
        </div>
      )}

      {/* ── Presets ─────────────────────────────────────────────────────── */}
      {templates.length > 0 && (
        <section className="flex flex-col gap-3">
          <div>
            <h2 className="flex items-center gap-2 text-sm font-semibold text-foreground">
              <Layers className="size-4 text-muted-foreground" />
              Presets
            </h2>
            <p className="mt-0.5 text-xs text-muted-foreground">
              Saved setups you can start a new macro from.
            </p>
          </div>
          <div className="divide-y divide-border overflow-hidden rounded-xl border border-border bg-card">
            {templates.map((t) => (
              <div key={t.name} className="flex items-center gap-3 px-4 py-3">
                <div className="min-w-0 flex-1">
                  <p className="truncate text-sm font-medium text-foreground">{t.name}</p>
                  <p className="text-xs text-muted-foreground">
                    {t.events.toLocaleString()} actions · {fmtDur(t.duration)}
                  </p>
                </div>
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

// ── One row ──────────────────────────────────────────────────────────────────

interface MacroRowProps {
  macro: MacroListItem;
  guards: number;
  speed: string;
  open: boolean;
  focusName: boolean;
  selected: boolean;
  playing: boolean;
  iteration: number;
  totalReps: number;
  busy: boolean;
  onToggle: () => void;
  onSelect: () => void;
  onRun: () => void;
  onStop: () => void;
  onSpeed: (v: string) => void;
  onRepeat: (i: number) => void;
  onRename: (to: string) => void;
  onCategory: (v: string) => void;
  onNotes: (v: string) => void;
  onGuards: () => void;
  onCheckpoints: () => void;
  onDuplicate: () => void;
  onSavePreset: () => void;
  onExport: () => void;
  onBundle: () => void;
  onDelete: () => void;
}

/**
 * A macro as a hairline row: name, plain meta, repeat, and the one verb, Run.
 * Everything else lives in a panel the row opens onto, so a setting is one click
 * away instead of a menu, a submenu and a dialog. The whole row is the toggle
 * (an overlay button behind the content), which is why the content layer is
 * pointer-transparent and each real control opts back in.
 */
function MacroRow(p: MacroRowProps) {
  const { macro } = p;
  const panelId = useId();
  const panelRef = useRef<HTMLDivElement>(null);
  const nameRef = useRef<HTMLInputElement>(null);

  const [draftName, setDraftName] = useState(macro.name);
  const [draftCategory, setDraftCategory] = useState(macro.category ?? "");
  const [draftNotes, setDraftNotes] = useState(macro.notes ?? "");
  useEffect(() => setDraftName(macro.name), [macro.name]);
  useEffect(() => setDraftCategory(macro.category ?? ""), [macro.category]);
  useEffect(() => setDraftNotes(macro.notes ?? ""), [macro.notes]);

  useEffect(() => {
    if (!p.open || !panelRef.current || reducedMotion()) return;
    animate(panelRef.current, { opacity: [0, 1], translateY: [-6, 0], duration: 200, ease: "out(3)" });
  }, [p.open]);

  useEffect(() => {
    if (p.open && p.focusName) {
      nameRef.current?.focus();
      nameRef.current?.select();
    }
  }, [p.open, p.focusName]);

  const commitName = () => {
    const to = draftName.trim();
    if (!to || to === macro.name) {
      setDraftName(macro.name);
      return;
    }
    p.onRename(to);
  };
  const commitCategory = () => {
    const v = draftCategory.trim();
    if (v !== (macro.category ?? "")) p.onCategory(v);
  };
  const commitNotes = () => {
    const v = draftNotes.trim();
    if (v !== (macro.notes ?? "")) p.onNotes(v);
  };

  const repeatIdx = repeatToIndex(macro.loop, macro.loop_count);
  const meta = [
    `${macro.events.toLocaleString()} action${macro.events === 1 ? "" : "s"}`,
    fmtDur(macro.duration),
  ];
  if (macro.play_count) meta.push(`played ${macro.play_count}×`);
  const ago = fmtAgo(macro.last_played ?? 0);
  if (ago) meta.push(ago);

  return (
    <div className={cn("transition-colors", p.playing && "bg-primary/5", p.open && "bg-muted/40")}>
      <div className="group relative">
        <button
          type="button"
          onClick={p.onToggle}
          aria-expanded={p.open}
          aria-controls={panelId}
          className="absolute inset-0 z-0 outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
        >
          <span className="sr-only">
            {p.open ? "Hide settings for" : "Settings for"} {macro.name}
          </span>
        </button>

        <div className="pointer-events-none relative z-10 flex items-center gap-3 px-4 py-3">
          {/* Glyph until you reach for the row, checkbox once you do: selection
              without a checkbox column standing in every row all the time. */}
          <div className="pointer-events-auto relative size-8 shrink-0">
            <span
              className={cn(
                "absolute inset-0 flex items-center justify-center rounded-lg bg-secondary text-muted-foreground transition-opacity",
                p.selected
                  ? "opacity-0"
                  : "group-hover:opacity-0 group-focus-within:opacity-0",
              )}
            >
              <ListVideo className="size-4" />
            </span>
            <span
              className={cn(
                "absolute inset-0 flex items-center justify-center opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100",
                p.selected && "opacity-100",
              )}
            >
              <Checkbox
                checked={p.selected}
                onCheckedChange={p.onSelect}
                aria-label={`Select ${macro.name}`}
              />
            </span>
          </div>

          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2">
              <span className="truncate font-medium text-foreground">{macro.name}</span>
              {macro.category && (
                <Badge variant="secondary" className="shrink-0 font-normal">
                  {macro.category}
                </Badge>
              )}
              {p.guards > 0 && (
                <Badge asChild variant="outline" className="pointer-events-auto shrink-0 font-normal">
                  <button
                    type="button"
                    onClick={p.onGuards}
                    title={`${p.guards} safety guard${p.guards === 1 ? "" : "s"}. Click to edit`}
                    className="gap-1 text-muted-foreground hover:border-primary/50 hover:text-foreground"
                  >
                    <Shield className="size-3" />
                    {p.guards}
                  </button>
                </Badge>
              )}
            </div>
            <p className="mt-0.5 truncate text-sm text-muted-foreground">{meta.join(" · ")}</p>
            {macro.notes && !p.open && (
              <p className="mt-1 truncate text-xs text-muted-foreground/80">{macro.notes}</p>
            )}
          </div>

          <RepeatChips
            value={repeatIdx}
            onChange={p.onRepeat}
            className="pointer-events-auto hidden md:flex"
          />
          {p.playing ? (
            <Button
              variant="destructive"
              size="sm"
              onClick={p.onStop}
              className="pointer-events-auto"
            >
              <Square className="size-4 fill-current" />
              Stop
            </Button>
          ) : (
            <Button
              size="sm"
              onClick={p.onRun}
              disabled={p.busy}
              className="pointer-events-auto"
            >
              <Play className="size-4 fill-current" />
              {p.speed === "1" ? "Run" : `Run ${p.speed}×`}
            </Button>
          )}
          <ChevronDown
            className={cn(
              "size-4 shrink-0 text-muted-foreground transition-transform",
              p.open && "rotate-180",
            )}
          />
        </div>
      </div>

      {p.playing && (
        <div className="flex items-center gap-3 border-t border-border px-4 py-2">
          <Progress
            value={p.totalReps > 0 ? (p.iteration / p.totalReps) * 100 : undefined}
            className="h-1.5 flex-1"
          />
          <span className="shrink-0 text-xs font-medium text-primary">
            {p.totalReps > 0 ? `Rep ${p.iteration} / ${p.totalReps}` : `Rep ${p.iteration} / ∞`}
          </span>
        </div>
      )}

      {p.open && (
        <div ref={panelRef} id={panelId} className="border-t border-border px-4 py-4">
          <div className="grid gap-4 sm:grid-cols-2">
            <Field label="Name">
              <Input
                ref={nameRef}
                value={draftName}
                onChange={(e) => setDraftName(e.target.value)}
                onBlur={commitName}
                onKeyDown={(e) => {
                  if (e.key === "Enter") e.currentTarget.blur();
                  if (e.key === "Escape") setDraftName(macro.name);
                }}
              />
            </Field>
            <Field label="Category" hint="Groups it under a label. Leave blank for none.">
              <Input
                value={draftCategory}
                onChange={(e) => setDraftCategory(e.target.value)}
                onBlur={commitCategory}
                onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
                placeholder="e.g. Farming"
              />
            </Field>
            <div className="sm:col-span-2">
              <Field label="Notes" hint="A reminder for later: what it does, where to run it.">
                <Textarea
                  rows={2}
                  value={draftNotes}
                  onChange={(e) => setDraftNotes(e.target.value)}
                  onBlur={commitNotes}
                  placeholder="Runs the daily reward loop…"
                />
              </Field>
            </div>
            <Field label="Playback speed" hint="1× is exactly how you recorded it.">
              <div className="flex flex-wrap gap-1.5">
                {SPEEDS.map((s) => (
                  <Chip key={s} active={p.speed === s} onClick={() => p.onSpeed(s)}>
                    {s}×
                  </Chip>
                ))}
              </div>
            </Field>
            {/* The compact chips on the row are md-and-up; below that this is the
                only way to reach the setting at all. */}
            <div className="md:hidden">
              <Field label="Repeat">
                <div className="flex flex-wrap gap-1.5">
                  {STOPS.map((s, i) => (
                    <Chip key={s} active={repeatIdx === i} onClick={() => p.onRepeat(i)}>
                      {REPEAT_WORDS[i]}
                    </Chip>
                  ))}
                </div>
              </Field>
            </div>
          </div>

          <div className="mt-4 flex flex-wrap gap-2 border-t border-border pt-4">
            <Button variant="outline" size="sm" onClick={p.onGuards}>
              <Shield className="size-4" />
              Safety guards{p.guards > 0 ? ` · ${p.guards}` : ""}
            </Button>
            <Button variant="outline" size="sm" onClick={p.onCheckpoints}>
              <Eye className="size-4" />
              Vision checkpoints
            </Button>
          </div>

          <div className="mt-3 flex flex-wrap items-center gap-1">
            <Button variant="ghost" size="sm" onClick={p.onDuplicate}>
              <Copy className="size-4" />
              Duplicate
            </Button>
            <Button variant="ghost" size="sm" onClick={p.onSavePreset}>
              <BookmarkPlus className="size-4" />
              Save as preset
            </Button>
            <Button variant="ghost" size="sm" onClick={p.onExport}>
              <Download className="size-4" />
              Share as file
            </Button>
            <Button variant="ghost" size="sm" onClick={p.onBundle}>
              <FolderInput className="size-4" />
              Share with images
            </Button>
            <div className="flex-1" />
            <Button
              variant="ghost"
              size="sm"
              onClick={p.onDelete}
              className="text-destructive hover:bg-destructive/10 hover:text-destructive"
            >
              <Trash2 className="size-4" />
              Delete
            </Button>
          </div>
        </div>
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

function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <label className="flex flex-col gap-1.5">
      <span className="text-xs font-medium text-foreground">{label}</span>
      {children}
      {hint && <span className="text-xs text-muted-foreground">{hint}</span>}
    </label>
  );
}

function Chip({
  active,
  onClick,
  children,
}: {
  active: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-pressed={active}
      className={cn(
        "rounded-md border px-2.5 py-1 text-xs font-medium transition-colors",
        active
          ? "border-primary bg-primary/15 text-foreground"
          : "border-border text-muted-foreground hover:text-foreground",
      )}
    >
      {children}
    </button>
  );
}

function FilterPill({
  active,
  onClick,
  children,
}: {
  active: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-pressed={active}
      className={cn(
        "rounded-full border px-3 py-1 text-xs font-medium transition-colors",
        active
          ? "border-primary bg-primary/15 text-foreground"
          : "border-border text-muted-foreground hover:text-foreground",
      )}
    >
      {children}
    </button>
  );
}

function RepeatChips({
  value,
  onChange,
  className,
}: {
  value: number;
  onChange: (i: number) => void;
  className?: string;
}) {
  return (
    <div
      className={cn("items-center rounded-md border border-border p-0.5", className)}
      title="How many times to repeat"
    >
      {STOPS.map((s, i) => (
        <button
          key={s}
          type="button"
          onClick={() => onChange(i)}
          aria-label={`Repeat: ${REPEAT_WORDS[i]}`}
          aria-pressed={value === i}
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

function EmptyState({ onRecord, disabled }: { onRecord: () => void; disabled: boolean }) {
  return (
    <div className="flex flex-col items-center gap-5 rounded-xl border border-border bg-card px-6 py-14 text-center">
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
    </div>
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
