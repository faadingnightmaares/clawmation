import { useCallback, useEffect, useId, useMemo, useRef, useState } from "react";
import { animate } from "animejs";
import {
  IconAdjustmentsHorizontal,
  IconAlertTriangle,
  IconCheck,
  IconChevronDown,
  IconClock,
  IconDownload,
  IconFilter,
  IconPackage,
  IconPlayerPause,
  IconPlayerPlay,
  IconPlayerStop,
  IconPlus,
  IconRefresh,
  IconSearch,
  IconStar,
  IconTrash,
  IconUpload,
} from "@tabler/icons-react";
import {
  BookmarkPlus,
  ChevronDown,
  Copy,
  Download,
  Eye,
  FolderInput,
  ListVideo,
  Play,
  Plus,
  Shield,
  Square,
  Trash2,
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
import { STOPS, fmtAgo, fmtDur, repeatToIndex, repsFor } from "@/format";
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
import { Skeleton } from "@/components/ui/skeleton";
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
import { GuardsEditor } from "@/components/editors/GuardsSheet";
import { CheckpointsEditor } from "@/components/editors/CheckpointsSheet";
import {
  MacroEditorPanel,
  MacroInspector,
  MacroLibrarySummary,
  MacroRow,
} from "@/components/macros/MacroWorkspaceCards";

const SPEEDS = ["0.25", "0.5", "1", "1.5", "2", "4"] as const;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const CANCELLED = "cancelled";
const FAVORITES_KEY = "clawmation.macro-favorites";

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
type MacroTab = "all" | "recent" | "favorites" | "ready";
type AutomationTab = "guards" | "checkpoints";

function readFavorites(): Set<string> {
  try {
    const stored = JSON.parse(
      globalThis.localStorage?.getItem(FAVORITES_KEY) ?? "[]",
    );
    return new Set(Array.isArray(stored) ? stored : []);
  } catch {
    return new Set();
  }
}

const BROWSER_PREVIEW_MACROS: MacroListItem[] = [
  {
    name: "raid",
    events: 4573,
    duration: 371,
    resolution: "1920x1080",
    loop: true,
    loop_count: 1,
    category: "",
    notes: "Full raid routine with buffs, pulls, and rotation.",
    play_count: 1,
    last_played: Math.floor(Date.now() / 1000) - 172800,
    played: 371,
  },
  {
    name: "event",
    events: 4480,
    duration: 304,
    resolution: "1920x1080",
    loop: false,
    loop_count: 1,
    category: "",
    notes: "Event farming route.",
    play_count: 3,
    last_played: Math.floor(Date.now() / 1000) - 432000,
    played: 912,
  },
  {
    name: "farming loop",
    events: 2913,
    duration: 272,
    resolution: "1920x1080",
    loop: true,
    loop_count: 2,
    category: "Farming",
    notes: "Daily resource farming loop.",
    play_count: 8,
    last_played: Math.floor(Date.now() / 1000) - 1209600,
    played: 2176,
  },
];

type MacroWorkspaceLoadState = "loading" | "ready" | "error";

interface MacroWorkspaceSnapshot {
  macros: MacroListItem[];
  templates: TemplateItem[];
  guardCounts: Record<string, number>;
}

// Views are unmounted as the user switches workspaces. Keep the last successful
// macro payload in memory so returning to Macros is instant while a quiet
// background refresh checks for changes. Tests intentionally skip this cache.
let macroWorkspaceCache: MacroWorkspaceSnapshot | null = null;

function readMacroWorkspaceCache() {
  return import.meta.env.MODE === "test" ? null : macroWorkspaceCache;
}

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

export function Macros({ status, active = true }: ViewProps) {
  const mode = status?.mode ?? "idle";
  const initialWorkspace = useRef(readMacroWorkspaceCache()).current;
  const hasLoadedWorkspace = useRef(initialWorkspace !== null);
  const [loadState, setLoadState] = useState<MacroWorkspaceLoadState>(
    initialWorkspace ? "ready" : "loading",
  );
  const [macros, setMacros] = useState<MacroListItem[]>(
    initialWorkspace?.macros ?? [],
  );
  const [templates, setTemplates] = useState<TemplateItem[]>(
    initialWorkspace?.templates ?? [],
  );
  const [guardCounts, setGuardCounts] = useState<Record<string, number>>(
    initialWorkspace?.guardCounts ?? {},
  );
  const [speeds, setSpeeds] = useState<Record<string, string>>(readSpeeds);
  const [selected, setSelected] = useState<Set<string>>(new Set());

  const [query, setQuery] = useState("");
  const [sort, setSort] = useState<SortKey>("recent");
  const [categoryFilter, setCategoryFilter] = useState("all");
  const [tab, setTab] = useState<MacroTab>("all");
  const [favorites, setFavorites] = useState<Set<string>>(readFavorites);

  const [countdown, setCountdown] = useState<number | null>(null);
  // The one row showing its settings. Only ever one: the panel is tall, and two
  // open at once turns the list back into the stack of cards this replaced.
  const [openRow, setOpenRow] = useState<string | null>(null);
  const [editingName, setEditingName] = useState<string | null>(null);
  // Set only when the open row is a *just-recorded* macro, which is the one time
  // the name field should take focus by itself: it still has its generated name.
  const [nameFresh, setNameFresh] = useState(false);
  const [prompt, setPrompt] = useState<PromptSpec | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<string | null>(null);
  const [bulkConfirm, setBulkConfirm] = useState(false);

  const [automationTab, setAutomationTab] = useState<AutomationTab>("guards");

  const pendingRename = useRef<string | null>(null);
  const playingName = useRef<string | null>(null);
  const prevMode = useRef(mode);
  const [playingCard, setPlayingCard] = useState<string | null>(null);
  const searchRef = useRef<HTMLInputElement>(null);

  const load = useCallback(async () => {
    try {
      // The macro list is the only payload required to render this workspace.
      // Presets and guard counts are useful decoration, so a failure in either
      // must never turn a healthy macro library into a false error/empty state.
      const [macrosResult, templatesResult, guardsResult] =
        await Promise.allSettled([
          listMacros(),
          listTemplates(),
          getAllGuardCounts(),
        ]);
      if (macrosResult.status === "rejected") throw macrosResult.reason;
      const ms = macrosResult.value ?? [];
      const ts =
        templatesResult.status === "fulfilled"
          ? (templatesResult.value ?? [])
          : [];
      const gc =
        guardsResult.status === "fulfilled" && guardsResult.value?.ok
          ? (guardsResult.value.counts ?? {})
          : {};
      const snapshot = { macros: ms, templates: ts, guardCounts: gc };

      setMacros(snapshot.macros);
      setTemplates(snapshot.templates);
      setGuardCounts(snapshot.guardCounts);
      if (import.meta.env.MODE !== "test") macroWorkspaceCache = snapshot;
      hasLoadedWorkspace.current = true;
      setLoadState("ready");
    } catch {
      const hasTauri = Boolean(
        (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__,
      );
      if (
        import.meta.env.DEV &&
        import.meta.env.MODE !== "test" &&
        !hasTauri
      ) {
        const snapshot = {
          macros: BROWSER_PREVIEW_MACROS,
          templates: [],
          guardCounts: {},
        };
        setMacros(snapshot.macros);
        setTemplates(snapshot.templates);
        setGuardCounts(snapshot.guardCounts);
        if (import.meta.env.MODE !== "test") macroWorkspaceCache = snapshot;
        hasLoadedWorkspace.current = true;
        setLoadState("ready");
      } else if (!hasLoadedWorkspace.current) {
        setLoadState("error");
      }
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  useEffect(() => {
    if (active) return;
    setPrompt(null);
    setDeleteTarget(null);
    setBulkConfirm(false);
  }, [active]);

  // Single place that reacts to a run ending: refresh + open the new recording's
  // row with its name ready to replace, toast + clear the playing row after a run.
  useEffect(() => {
    const prev = prevMode.current;
    if (prev === mode) return;
    if ((prev === "recording" || prev === "paused") && mode === "idle") {
      load().then(() => {
        if (pendingRename.current) {
          setOpenRow(pendingRename.current);
          setEditingName(pendingRename.current);
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
    if (!active) return;
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
  }, [active]);

  const busy = mode !== "idle" || countdown !== null;

  const toggleRow = (name: string) => {
    setNameFresh(false);
    setOpenRow(name);
    setEditingName((current) => (current === name ? current : null));
  };

  const toggleFavorite = (name: string) => {
    setFavorites((current) => {
      const next = new Set(current);
      if (next.has(name)) next.delete(name);
      else next.add(name);
      globalThis.localStorage?.setItem(
        FAVORITES_KEY,
        JSON.stringify([...next]),
      );
      return next;
    });
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
  const play = async (m: MacroListItem, repeatOverride?: number) => {
    const reps =
      repeatOverride ?? repsFor(repeatToIndex(m.loop, m.loop_count));
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
      setEditingName((cur) => (cur === from ? name : cur));
      setNameFresh(false);
      setFavorites((current) => {
        if (!current.has(from)) return current;
        const next = new Set(current);
        next.delete(from);
        next.add(name);
        globalThis.localStorage?.setItem(
          FAVORITES_KEY,
          JSON.stringify([...next]),
        );
        return next;
      });
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
        await load();
        if (res.name) {
          setOpenRow(res.name);
          setEditingName(null);
          setNameFresh(false);
        }
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
        setEditingName((cur) => (cur === name ? null : cur));
        setFavorites((current) => {
          if (!current.has(name)) return current;
          const next = new Set(current);
          next.delete(name);
          globalThis.localStorage?.setItem(
            FAVORITES_KEY,
            JSON.stringify([...next]),
          );
          return next;
        });
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
      if (tab === "recent" && !m.last_played) return false;
      if (tab === "favorites" && !favorites.has(m.name)) return false;
      if (tab === "ready" && m.events < 1) return false;
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
  }, [macros, query, categoryFilter, sort, tab, favorites]);

  // Animate the library only when its initial payload replaces the skeleton.
  // Filtering, sorting, duplicating, and deleting should feel immediate rather
  // than replaying an entrance animation across every existing row.
  const listRef = useStaggerIn<HTMLDivElement>(loadState);
  const activeMacro =
    visible.find((macro) => macro.name === openRow) ?? visible[0] ?? null;
  const recording = mode === "recording" || mode === "paused";

  if (loadState === "loading") return <MacrosLoadingState />;

  if (loadState === "error") {
    return (
      <MacrosLoadError
        onRetry={() => {
          setLoadState("loading");
          void load();
        }}
      />
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      {countdown !== null && <Countdown n={countdown} />}

      <div className="grid min-h-0 flex-1 gap-5 overflow-y-auto lg:grid-cols-[minmax(300px,0.82fr)_minmax(0,3fr)] lg:overflow-hidden">
        {activeMacro && (
          <div className="workspace-scrollbar flex min-h-0 flex-col gap-5 overflow-y-auto pr-2">
            <MacroEditorPanel
              key={activeMacro.name}
              macro={activeMacro}
              speed={speeds[activeMacro.name] ?? "1"}
              focusName={editingName === activeMacro.name && nameFresh}
              playing={playingCard === activeMacro.name && mode === "playing"}
              onStop={stopPlay}
              onSpeed={(value) => setSpeed(activeMacro.name, value)}
              onRepeat={(index) => changeRepeat(activeMacro, index)}
              onRename={(to) => rename(activeMacro.name, to)}
              onCategory={(value) => saveCategory(activeMacro, value)}
              onNotes={(value) => saveNotes(activeMacro, value)}
              onSavePreset={() => askSaveAsTemplate(activeMacro)}
              onExport={() => onExportMacro(activeMacro)}
              onBundle={() => onExportBundle(activeMacro)}
            />
            <section
              aria-label={`Screen safeguards for ${activeMacro.name}`}
              className="flex min-h-[360px] shrink-0 flex-col overflow-hidden rounded-xl border border-border bg-card shadow-[0_12px_34px_rgba(50,35,18,0.045)]"
            >
              <div className="border-b border-border px-4 py-3">
                <p className="text-sm font-semibold text-foreground">
                  Screen safeguards
                </p>
                <p className="mt-0.5 text-xs text-muted-foreground">
                  Recovery rules and visual waits in one place.
                </p>
                <div
                  role="tablist"
                  aria-label="Screen safeguard tools"
                  className="mt-3 grid grid-cols-2 overflow-hidden rounded-lg border border-border"
                >
                  <button
                    type="button"
                    role="tab"
                    aria-selected={automationTab === "guards"}
                    onClick={() => setAutomationTab("guards")}
                    className={cn(
                      "border-r border-border px-3 py-2 text-xs font-medium transition-colors",
                      automationTab === "guards"
                        ? "bg-primary/10 text-primary"
                        : "text-muted-foreground hover:text-foreground",
                    )}
                  >
                    Safety
                    {guardCounts[activeMacro.name]
                      ? ` · ${guardCounts[activeMacro.name]}`
                      : ""}
                  </button>
                  <button
                    type="button"
                    role="tab"
                    aria-selected={automationTab === "checkpoints"}
                    onClick={() => setAutomationTab("checkpoints")}
                    className={cn(
                      "px-3 py-2 text-xs font-medium transition-colors",
                      automationTab === "checkpoints"
                        ? "bg-primary/10 text-primary"
                        : "text-muted-foreground hover:text-foreground",
                    )}
                  >
                    Vision
                  </button>
                </div>
              </div>
              <div
                key={`${activeMacro.name}:${automationTab}`}
                className="min-h-0"
              >
                {automationTab === "guards" ? (
                  <GuardsEditor
                    macroName={activeMacro.name}
                    onChanged={load}
                    embedded
                  />
                ) : (
                  <CheckpointsEditor
                    macroName={activeMacro.name}
                    onChanged={load}
                    embedded
                  />
                )}
              </div>
            </section>
          </div>
        )}

        <div
          className={cn(
            "flex min-h-0 flex-col gap-5 lg:overflow-hidden",
            !activeMacro && "lg:col-span-2",
          )}
        >
      {/* ── Header: what this is, and the one thing you came here to press ── */}
      <header className="flex flex-wrap items-center justify-between gap-5 border-b border-border/70 pb-5">
        <div className="space-y-1">
          <h1 className="text-[28px] font-semibold tracking-[-0.035em] text-foreground">
            Macros
          </h1>
          <p className="text-sm text-muted-foreground">
            {recording
              ? "Every click and key is being captured."
              : "Create, manage, and run your macros with ease."}
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
              {mode === "paused" ? (
                <IconPlayerPlay className="size-4" />
              ) : (
                <IconPlayerPause className="size-4" />
              )}
              {mode === "paused" ? "Resume" : "Pause"}
            </Button>
            <Button variant="destructive" size="sm" onClick={toggleRecord}>
              <IconPlayerStop className="size-4 fill-current" />
              Stop &amp; save
            </Button>
          </div>
        ) : (
          <div className="flex items-center gap-3">
            <Button
              size="lg"
              onClick={toggleRecord}
              disabled={busy}
              className="min-w-32 rounded-lg shadow-[0_8px_20px_color-mix(in_srgb,var(--primary)_20%,transparent)]"
            >
              <span className="size-2.5 rounded-full bg-current" />
              Record
            </Button>
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button variant="outline" size="lg" className="rounded-lg">
                  <IconPlus className="size-[18px]" strokeWidth={1.8} />
                  New macro
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" className="w-56">
                <DropdownMenuItem onSelect={onImportMacro}>
                  <IconUpload className="size-4" />
                  Add a macro from a file
                </DropdownMenuItem>
                <DropdownMenuItem onSelect={onImportBundle}>
                  <IconPackage className="size-4" />
                  Add a bundle (macro + images)
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
        )}
      </header>

      {/* ── Search, category, sort ──────────────────────────────────────── */}
      {macros.length > 0 && (
        <div className="grid gap-4 lg:grid-cols-[minmax(0,1.75fr)_minmax(340px,1fr)]">
          <div className="grid gap-3 sm:grid-cols-[minmax(220px,1fr)_145px_180px]">
            <div className="relative min-w-0">
              <IconSearch className="pointer-events-none absolute left-3.5 top-1/2 size-[18px] -translate-y-1/2 text-muted-foreground" />
              <Input
                ref={searchRef}
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="Search macros…"
                className="h-11 rounded-lg pl-10"
              />
            </div>
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button
                  variant="outline"
                  className="h-11 justify-between rounded-lg px-4"
                >
                  <span className="flex min-w-0 items-center gap-2">
                    <IconFilter className="size-[18px]" strokeWidth={1.7} />
                    <span className="truncate">
                      {categoryFilter === "all" ? "Filter" : categoryFilter}
                    </span>
                  </span>
                  <IconChevronDown className="size-4" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="start" className="w-52">
                <DropdownMenuItem onSelect={() => setCategoryFilter("all")}>
                  {categoryFilter === "all" && <IconCheck className="size-4" />}
                  All categories
                </DropdownMenuItem>
                {categories.map((category) => (
                  <DropdownMenuItem
                    key={category}
                    onSelect={() => setCategoryFilter(category)}
                  >
                    {categoryFilter === category && (
                      <IconCheck className="size-4" />
                    )}
                    {category}
                  </DropdownMenuItem>
                ))}
              </DropdownMenuContent>
            </DropdownMenu>
            <Select value={sort} onValueChange={(v) => setSort(v as SortKey)}>
              <SelectTrigger className="h-11 w-full rounded-lg" aria-label="Sort macros">
                <IconAdjustmentsHorizontal className="size-[18px]" />
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="recent">Last played</SelectItem>
                <SelectItem value="name">Name</SelectItem>
                <SelectItem value="plays">Most played</SelectItem>
                <SelectItem value="duration">Longest</SelectItem>
                <SelectItem value="events">Most actions</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <div
            role="tablist"
            aria-label="Macro views"
            className="grid h-11 grid-cols-4 overflow-hidden rounded-lg border border-border bg-card"
          >
            {(
              [
                ["all", "All", null],
                ["recent", "Recent", IconClock],
                ["favorites", "Favorites", IconStar],
                ["ready", "Ready", IconPlayerPlay],
              ] as const
            ).map(([value, label, Icon]) => (
              <button
                key={value}
                type="button"
                role="tab"
                aria-selected={tab === value}
                onClick={() => setTab(value)}
                className={cn(
                  "relative flex items-center justify-center gap-1.5 border-l border-border/70 px-2 text-xs font-medium text-muted-foreground transition-colors first:border-l-0 hover:text-foreground",
                  tab === value && "bg-primary/5 text-primary",
                )}
              >
                {Icon && <Icon className="size-4" strokeWidth={1.7} />}
                <span>{label}</span>
                {tab === value && (
                  <span className="absolute inset-x-0 bottom-0 h-0.5 bg-primary" />
                )}
              </button>
            ))}
          </div>
        </div>
      )}

      {/* ── Bulk bar ────────────────────────────────────────────────────── */}
      {selected.size > 0 && (
        <div className="flex flex-wrap items-center gap-3 rounded-lg border border-primary/30 bg-primary/5 px-4 py-2.5">
          <span className="text-sm font-medium">{selected.size} selected</span>
          <div className="flex-1" />
          <Button variant="ghost" size="sm" onClick={onBulkExport}>
            <IconDownload className="size-4" />
            Export
          </Button>
          <Button
            variant="ghost"
            size="sm"
            onClick={() => setBulkConfirm(true)}
            className="text-destructive hover:bg-destructive/10 hover:text-destructive"
          >
            <IconTrash className="size-4" />
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
        <div className="grid min-h-0 flex-1 items-stretch gap-5 overflow-hidden lg:grid-cols-[minmax(0,1.75fr)_minmax(320px,1fr)]">
          <div className="flex min-h-0 min-w-0 flex-col">
            <div
              ref={listRef}
              role="region"
              aria-label="Saved macros"
              tabIndex={0}
              data-visible-rows="6"
              className="macro-list workspace-scrollbar grid min-h-0 min-w-0 max-h-[42.75rem] flex-1 content-start gap-3 overflow-y-auto overscroll-contain pr-2 outline-none focus-visible:ring-2 focus-visible:ring-ring"
            >
              {visible.map((macro) => (
                <MacroRow
                  key={macro.name}
                  macro={macro}
                  guards={guardCounts[macro.name] ?? 0}
                  speed={speeds[macro.name] ?? "1"}
                  active={activeMacro?.name === macro.name}
                  favorite={favorites.has(macro.name)}
                  selected={selected.has(macro.name)}
                  playing={playingCard === macro.name && mode === "playing"}
                  iteration={status?.play_iteration ?? 0}
                  totalReps={status?.play_total_reps ?? 0}
                  busy={busy}
                  onToggle={() => toggleRow(macro.name)}
                  onSelect={() => toggleSelect(macro.name)}
                  onFavorite={() => toggleFavorite(macro.name)}
                  onRun={() => play(macro)}
                  onStop={stopPlay}
                  onRepeat={(index) => changeRepeat(macro, index)}
                  onDuplicate={() => onDuplicate(macro)}
                  onExport={() => onExportMacro(macro)}
                  onBundle={() => onExportBundle(macro)}
                  onDelete={() => setDeleteTarget(macro.name)}
                />
              ))}
            </div>
            <MacroLibrarySummary macros={visible} />
          </div>
          {activeMacro && (
            <div className="min-h-0 overflow-hidden">
              <MacroInspector
                key={activeMacro.name}
                macro={activeMacro}
                favorite={favorites.has(activeMacro.name)}
                busy={busy}
                onFavorite={() => toggleFavorite(activeMacro.name)}
                onRun={(repeat) => play(activeMacro, repeat)}
                onDuplicate={() => onDuplicate(activeMacro)}
                onDelete={() => setDeleteTarget(activeMacro.name)}
              />
            </div>
          )}
        </div>
      )}

      {/* ── Presets ─────────────────────────────────────────────────────── */}
      {templates.length > 0 && (
        <section className="flex flex-col gap-3">
          <div>
            <h2 className="flex items-center gap-2 text-sm font-semibold text-foreground">
              <IconPackage className="size-4 text-muted-foreground" />
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
                  <IconTrash className="size-4" />
                </Button>
              </div>
            ))}
          </div>
        </section>
      )}
        </div>
      </div>

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

    </div>
  );
}

// ── One row ──────────────────────────────────────────────────────────────────

interface LegacyMacroRowProps {
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
function LegacyMacroRow(p: LegacyMacroRowProps) {
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

void LegacyMacroRow;
void FilterPill;

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

function MacrosLoadingState() {
  return (
    <div
      role="status"
      aria-label="Loading macro workspace"
      aria-live="polite"
      className="flex h-full min-h-0 flex-col"
    >
      <span className="sr-only">Loading your macros</span>
      <div className="grid min-h-0 flex-1 gap-5 overflow-y-auto lg:grid-cols-[minmax(300px,0.82fr)_minmax(0,3fr)] lg:overflow-hidden">
        <div className="workspace-scrollbar flex min-h-0 flex-col gap-5 overflow-hidden pr-2">
          <div className="shrink-0 rounded-xl border border-border bg-card p-5">
            <Skeleton className="h-4 w-24" />
            <Skeleton className="mt-2 h-3 w-16" />
            <div className="mt-6 space-y-5">
              {["w-full", "w-4/5", "w-full"].map((width, index) => (
                <div key={index}>
                  <Skeleton className="mb-2 h-3 w-20" />
                  <Skeleton className={cn("h-9 rounded-md", width)} />
                </div>
              ))}
              <Skeleton className="h-9 w-full rounded-md" />
            </div>
          </div>
          <div className="min-h-[360px] shrink-0 rounded-xl border border-border bg-card p-5">
            <Skeleton className="h-4 w-32" />
            <Skeleton className="mt-2 h-3 w-52" />
            <Skeleton className="mt-5 h-9 w-full rounded-lg" />
            <Skeleton className="mt-5 h-40 w-full rounded-lg" />
          </div>
        </div>

        <div className="flex min-h-0 flex-col gap-5 overflow-hidden">
          <header className="flex items-center justify-between gap-5 border-b border-border/70 pb-5">
            <div>
              <Skeleton className="h-8 w-28" />
              <Skeleton className="mt-2 h-4 w-64" />
            </div>
            <div className="flex gap-3">
              <Skeleton className="h-10 w-32 rounded-lg" />
              <Skeleton className="h-10 w-36 rounded-lg" />
            </div>
          </header>

          <div className="grid gap-4 lg:grid-cols-[minmax(0,1.75fr)_minmax(340px,1fr)]">
            <div className="grid gap-3 sm:grid-cols-[minmax(220px,1fr)_145px_180px]">
              <Skeleton className="h-11 rounded-lg" />
              <Skeleton className="h-11 rounded-lg" />
              <Skeleton className="h-11 rounded-lg" />
            </div>
            <Skeleton className="h-11 rounded-lg" />
          </div>

          <div className="grid min-h-0 flex-1 gap-5 overflow-hidden lg:grid-cols-[minmax(0,1.75fr)_minmax(320px,1fr)]">
            <div className="grid min-h-0 content-start gap-3 overflow-hidden pr-2">
              {Array.from({ length: 6 }, (_, index) => (
                <Skeleton key={index} className="h-[92px] w-full rounded-xl" />
              ))}
            </div>
            <Skeleton className="min-h-[360px] rounded-xl" />
          </div>
        </div>
      </div>
    </div>
  );
}

function MacrosLoadError({ onRetry }: { onRetry: () => void }) {
  return (
    <div className="flex h-full min-h-0 items-center justify-center">
      <div
        role="alert"
        className="flex max-w-md flex-col items-center rounded-xl border border-border bg-card px-8 py-10 text-center shadow-[0_12px_34px_rgba(50,35,18,0.045)]"
      >
        <span className="flex size-11 items-center justify-center rounded-full bg-destructive/10 text-destructive">
          <IconAlertTriangle className="size-5" strokeWidth={1.8} />
        </span>
        <h1 className="mt-4 text-lg font-semibold">Couldn&apos;t load macros</h1>
        <p className="mt-1.5 text-sm leading-6 text-muted-foreground">
          Your macros are still safe. Clawmation couldn&apos;t reach the local
          library just now.
        </p>
        <Button className="mt-5" onClick={onRetry}>
          <IconRefresh className="size-4" />
          Try again
        </Button>
      </div>
    </div>
  );
}

function EmptyState({ onRecord, disabled }: { onRecord: () => void; disabled: boolean }) {
  return (
    <div className="flex w-full max-w-2xl self-center flex-col items-center gap-5 rounded-xl border border-border bg-card px-6 py-14 text-center">
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
