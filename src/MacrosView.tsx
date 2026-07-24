import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type CSSProperties,
  type PointerEvent,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";
import {
  Copy,
  DotsThreeVertical,
  DownloadSimple,
  Eye,
  List,
  MagnifyingGlass,
  NotePencil,
  Package,
  Pause,
  PencilSimple,
  Play,
  Plus,
  Record,
  ShieldCheck,
  Stop,
  Tag,
  Trash,
  UploadSimple,
} from "@phosphor-icons/react";

import {
  addCheckpoint as apiAddCheckpoint,
  bulkDelete,
  bulkExport,
  createFromTemplate,
  deleteMacro,
  deleteTemplate,
  duplicateMacro,
  emergencyStop,
  exportBundle,
  exportMacro,
  getAllGuardCounts,
  guardList,
  guardPickColor,
  guardSave,
  guardTest,
  importBundle,
  importMacro,
  listCheckpoints,
  listMacros,
  listTemplates,
  macroToSteps,
  pauseRecord,
  playMacro,
  removeCheckpoint as apiRemoveCheckpoint,
  renameMacro,
  saveAsTemplate,
  setCategory as apiSetCategory,
  setNotes as apiSetNotes,
  setRepeat,
  startRecord,
  stepsRun,
  stepsSave,
  stepsTest,
  stopRecord,
  type CheckpointItem,
  type Guard,
  type Status,
  type Step,
  type TemplateItem,
} from "./api";
import { useToast } from "./components/Toasts";
import {
  CheckpointEditor,
  freshCheckpointDraft,
  type CheckpointDraft,
} from "./CheckpointEditor";
import { GuardEditor, type GuardDraft } from "./GuardEditor";
import { StepEditor, type StepEditorTestResult } from "./StepEditor";
import { hsvToCss } from "./format";
import {
  AddButton,
  Card,
  CardHead,
  EmptyState,
  GhostButton,
  LABEL,
  Page,
  PrimaryButton,
} from "./components/ui";

// The Macros view is the heart of the app: record a macro, then play, repeat,
// organize, and template it. This is a faithful port of the Python UI's Macros
// screen — every prompt, confirm, and toast string is preserved character for
// character. The one sub-panel whose backend has not landed yet (schedules) is
// intentionally omitted rather than rendered as a dead button.

// The repeat dial's six stops; index 0 is "loop forever".
const STOPS = ["∞", "1", "2", "3", "5", "10"];

function fmtDur(sec: number): string {
  const m = Math.floor(sec / 60);
  const s = Math.round(sec % 60);
  return m + ":" + String(s).padStart(2, "0");
}

function fmtAgo(epoch: number): string {
  if (!epoch) return "";
  const diff = Math.floor(Date.now() / 1000 - epoch);
  if (diff < 60) return "just now";
  if (diff < 3600) return Math.floor(diff / 60) + "m ago";
  if (diff < 86400) return Math.floor(diff / 3600) + "h ago";
  return Math.floor(diff / 86400) + "d ago";
}

/** Slider index → the `repeat` argument the backend expects (`∞` → 0). */
function repsForIndex(idx: number): number {
  const v = STOPS[idx];
  return v === "∞" ? 0 : parseInt(v, 10);
}

/** Backend `(loop, loop_count)` → slider index in STOPS. */
function repeatToIndex(loop: boolean, loopCount: number): number {
  if (loop && loopCount === 0) return 0;
  if (!loop) return 1;
  const idx = STOPS.indexOf(String(loopCount));
  return idx >= 0 ? idx : 1;
}

/** The label-spin order when the dial lands, for the slot-machine flourish. */
function spinSeqFor(finalIdx: number): number[] {
  return [
    (finalIdx + 3) % 6,
    (finalIdx + 5) % 6,
    (finalIdx + 1) % 6,
    (finalIdx + 4) % 6,
    (finalIdx + 2) % 6,
    finalIdx,
  ];
}

/** A macro list row, enriched with the local-only UI state Python keeps per macro. */
interface MacroRow {
  id: string;
  name: string;
  events: number;
  durationSec: number;
  resolution: string;
  repeatIndex: number;
  category: string;
  notes: string;
  guardCount: number;
  play_count: number;
  last_played: number;
  speed: string;
}

const iconBtnBase: CSSProperties = {
  display: "inline-flex",
  alignItems: "center",
  justifyContent: "center",
  width: 30,
  height: 30,
  borderRadius: "var(--radius-sm)",
  background: "transparent",
  cursor: "pointer",
};

// A per-row "⋯" overflow menu. Every macro used to spray a dozen equal buttons
// across its row; now Run is the one visible action and everything secondary
// lives here — opened by a single kebab, closed on outside-click, Escape, or
// when a row invokes the `close` it is handed.
const MENU_WIDTH = 236;

function OverflowMenu({
  title,
  children,
}: {
  title: string;
  children: (close: () => void) => ReactNode;
}) {
  const [open, setOpen] = useState(false);
  const [pos, setPos] = useState<{ top: number; left: number } | null>(null);
  const btnRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);

  // The macro card clips its overflow (for rounded corners and the progress
  // bar), so the popover can't live inside it — it's portalled to <body> and
  // pinned under the kebab in viewport space.
  const place = useCallback(() => {
    const b = btnRef.current;
    if (!b) return;
    const r = b.getBoundingClientRect();
    setPos({ top: r.bottom + 6, left: Math.max(8, r.right - MENU_WIDTH) });
  }, []);

  useEffect(() => {
    if (!open) return;
    place();
    const onDown = (e: MouseEvent) => {
      const t = e.target as Node;
      if (btnRef.current?.contains(t) || menuRef.current?.contains(t)) return;
      setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    const dismiss = () => setOpen(false);
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    window.addEventListener("scroll", dismiss, true);
    window.addEventListener("resize", dismiss);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
      window.removeEventListener("scroll", dismiss, true);
      window.removeEventListener("resize", dismiss);
    };
  }, [open, place]);

  return (
    <>
      <button
        ref={btnRef}
        type="button"
        title={title}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
        style={{
          ...iconBtnBase,
          border: "1px solid " + (open ? "var(--color-gold)" : "var(--border-default)"),
          background: open ? "rgba(194,163,112,0.08)" : "transparent",
          color: open ? "var(--color-gold)" : "var(--text-50)",
        }}
      >
        <DotsThreeVertical size={18} weight="bold" />
      </button>
      {open &&
        pos &&
        createPortal(
          <div
            ref={menuRef}
            role="menu"
            style={{
              position: "fixed",
              top: pos.top,
              left: pos.left,
              zIndex: 1000,
              width: MENU_WIDTH,
              padding: 6,
              background: "var(--surface-card-strong)",
              backdropFilter: "blur(10px)",
              border: "1px solid var(--border-strong)",
              borderRadius: "var(--radius-md)",
              boxShadow: "var(--shadow-3)",
              display: "flex",
              flexDirection: "column",
              gap: 1,
            }}
          >
            {children(() => setOpen(false))}
          </div>,
          document.body,
        )}
    </>
  );
}

// One line inside an OverflowMenu: leading icon, label, optional right-side
// hint. `active` tints it gold (a panel is open / a value is set); `danger`
// tints it red for destructive rows.
function MenuRow({
  icon,
  label,
  hint,
  active,
  danger,
  onClick,
}: {
  icon: ReactNode;
  label: ReactNode;
  hint?: ReactNode;
  active?: boolean;
  danger?: boolean;
  onClick: () => void;
}) {
  const rest = active ? "rgba(194,163,112,0.10)" : "transparent";
  return (
    <button
      type="button"
      role="menuitem"
      onClick={onClick}
      style={{
        display: "flex",
        alignItems: "center",
        gap: 10,
        width: "100%",
        height: 34,
        padding: "0 10px",
        borderRadius: "var(--radius-sm)",
        border: "none",
        background: rest,
        color: danger ? "var(--verdict-blocked)" : active ? "var(--color-gold)" : "var(--text-78)",
        fontFamily: "var(--font-sans)",
        fontSize: 12.5,
        fontWeight: 600,
        cursor: "pointer",
        textAlign: "left",
        whiteSpace: "nowrap",
      }}
      onMouseEnter={(e) =>
        (e.currentTarget.style.background = danger ? "var(--verdict-blocked-bg)" : "rgba(26,24,22,0.05)")
      }
      onMouseLeave={(e) => (e.currentTarget.style.background = rest)}
    >
      <span style={{ display: "inline-flex", flexShrink: 0, width: 15 }}>{icon}</span>
      <span style={{ flex: 1 }}>{label}</span>
      {hint != null && (
        <span
          style={{
            flexShrink: 0,
            fontFamily: "var(--font-mono)",
            fontSize: 10.5,
            color: active ? "var(--color-gold)" : "var(--text-40)",
          }}
        >
          {hint}
        </span>
      )}
    </button>
  );
}

export function MacrosView({ status }: { status: Status | null }) {
  // Destructure the stable `push` callback (not the wrapper object, whose
  // identity churns on every toast) so effects depending on it don't re-run.
  const { push: pushToast } = useToast();

  // Live/optimistic mode mirror. `modeRef` always tracks the latest so async
  // handlers and the heartbeat effect read a fresh value (like Python's
  // `this.state.mode`).
  const [mode, setMode] = useState<string>("idle");
  const modeRef = useRef(mode);
  modeRef.current = mode;

  const [macros, setMacros] = useState<MacroRow[]>([]);
  const macrosRef = useRef(macros);
  macrosRef.current = macros;

  const [templates, setTemplates] = useState<TemplateItem[]>([]);

  const [macroSearch, setMacroSearch] = useState("");
  const [categoryFilter, setCategoryFilter] = useState("");
  const [macroSort, setMacroSort] = useState("recent");

  const [selectedMacros, setSelectedMacros] = useState<string[]>([]);
  const [recordNamePromptId, setRecordNamePromptId] = useState<string | null>(null);
  const recordNamePromptRef = useRef(recordNamePromptId);
  recordNamePromptRef.current = recordNamePromptId;

  const [deleteConfirmId, setDeleteConfirmId] = useState<string | null>(null);
  const deleteConfirmRef = useRef(deleteConfirmId);
  deleteConfirmRef.current = deleteConfirmId;

  const [countdown, setCountdown] = useState(0);

  // Live recording/playback metrics, mirrored off the heartbeat.
  const [recordPaused, setRecordPaused] = useState(false);
  const [recordElapsed, setRecordElapsed] = useState(0);
  const [recordEvents, setRecordEvents] = useState(0);
  const [playingMacroId, setPlayingMacroId] = useState<string | null>(null);
  const [playElapsed, setPlayElapsed] = useState(0);
  const [playIteration, setPlayIteration] = useState(0);
  const [playTotalReps, setPlayTotalReps] = useState(0);

  // Repeat-dial interaction state.
  const [dragMacroId, setDragMacroId] = useState<string | null>(null);
  const [dragIndex, setDragIndex] = useState<number | null>(null);
  const [spinMacroId, setSpinMacroId] = useState<string | null>(null);
  const [spinTick, setSpinTick] = useState(0);
  const [landedMacroId, setLandedMacroId] = useState<string | null>(null);
  const dragRef = useRef<{ id: string; index: number } | null>(null);
  const dragRectRef = useRef<DOMRect | null>(null);

  const busyRef = useRef(false);
  const deleteTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // ── Guards (per-macro watchers) ──────────────────────────────────────────
  // Lazy-loaded per macro: guardsByMacro[id] is populated only when a card's
  // guards panel is first opened, mirroring Python's toggleGuards. guardEditor
  // is the shared editor's working copy. Refs track the latest for the async
  // save / test / toggle handlers, matching the file's modeRef idiom.
  const [guardsOpenId, setGuardsOpenId] = useState<string | null>(null);
  const guardsOpenIdRef = useRef(guardsOpenId);
  guardsOpenIdRef.current = guardsOpenId;

  const [guardsByMacro, setGuardsByMacro] = useState<Record<string, GuardDraft[]>>({});
  const guardsByMacroRef = useRef(guardsByMacro);
  guardsByMacroRef.current = guardsByMacro;

  const [guardEditor, setGuardEditor] = useState<GuardDraft | null>(null);
  const guardEditorRef = useRef(guardEditor);
  guardEditorRef.current = guardEditor;

  // ── Vision checkpoints (per-macro mid-run detections) ────────────────────
  // Same lazy-load + ref-mirror shape as guards: checkpointsByMacro[id] fills
  // when a card's vision panel opens; visionDraft is the add-form's working
  // copy. Ports Python's toggleVision / loadCheckpoints / visionDraft.
  const [visionOpenId, setVisionOpenId] = useState<string | null>(null);
  const visionOpenIdRef = useRef(visionOpenId);
  visionOpenIdRef.current = visionOpenId;

  const [checkpointsByMacro, setCheckpointsByMacro] = useState<Record<string, CheckpointItem[]>>(
    {},
  );

  const [visionDraft, setVisionDraft] = useState<CheckpointDraft | null>(null);
  const visionDraftRef = useRef(visionDraft);
  visionDraftRef.current = visionDraft;

  // ── Step editor (recorded macro → editable action list) ──────────────────
  // The host owns the list + the async seam; StepEditor owns the in-memory
  // edits. stepsRef mirrors the list so the async Save / Run / Test handlers
  // read the latest edits. Ports Python's stepEditor* state (index.html:3211).
  const [stepsOpenId, setStepsOpenId] = useState<string | null>(null);
  const stepsOpenIdRef = useRef(stepsOpenId);
  stepsOpenIdRef.current = stepsOpenId;

  const [steps, setSteps] = useState<Step[]>([]);
  const stepsRef = useRef(steps);
  stepsRef.current = steps;

  const [stepsLoading, setStepsLoading] = useState(false);
  const [stepsTestResult, setStepsTestResult] = useState<StepEditorTestResult | null>(null);

  const refreshMacros = useCallback(async () => {
    try {
      const [list, gc] = await Promise.all([listMacros(), getAllGuardCounts()]);
      const guardCounts = gc && gc.ok ? gc.counts || {} : {};
      setMacros((prev) => {
        const byName = Object.fromEntries(prev.map((m) => [m.name, m]));
        return (list || []).map((m) => ({
          id: m.name,
          name: m.name,
          events: m.events,
          durationSec: m.duration,
          resolution: (m.resolution || "").replace("x", "×"),
          repeatIndex:
            byName[m.name] && byName[m.name].repeatIndex != null
              ? byName[m.name].repeatIndex
              : repeatToIndex(m.loop, m.loop_count),
          category: m.category || "",
          // Parity: the Python row-builder carries no notes, so the notes
          // display / green-pencil / search-on-notes never activate there.
          notes: "",
          guardCount: guardCounts[m.name] || 0,
          play_count: m.play_count || 0,
          last_played: m.last_played || 0,
          speed: "1",
        }));
      });
    } catch (e) {
      console.error("list_macros", e);
    }
  }, []);

  const loadTemplates = useCallback(async () => {
    try {
      const list = await listTemplates();
      setTemplates(list || []);
    } catch (e) {
      console.error("list_templates", e);
    }
  }, []);

  useEffect(() => {
    refreshMacros();
    loadTemplates();
    return () => {
      if (deleteTimerRef.current) clearTimeout(deleteTimerRef.current);
    };
  }, [refreshMacros, loadTemplates]);

  // Heartbeat sync: mirror the server mode/metrics and fire the two transition
  // effects (refresh on record-end, toast on playback-end). Suppressed while a
  // user action is mid-flight, exactly like Python's `_busyAction` guard.
  useEffect(() => {
    if (!status || busyRef.current) return;
    const prevMode = modeRef.current;
    setRecordEvents(status.recorded_count || 0);
    if (status.mode === "recording" || status.mode === "paused") {
      setRecordElapsed(Math.floor(status.elapsed || 0));
      setRecordPaused(status.mode === "paused");
    } else if (status.mode === "playing") {
      setPlayElapsed(Math.floor(status.elapsed || 0));
      setPlayIteration(status.play_iteration || 0);
      setPlayTotalReps(status.play_total_reps || 0);
    }
    if ((prevMode === "recording" || prevMode === "paused") && status.mode === "idle") {
      refreshMacros();
    }
    if (prevMode === "playing" && status.mode === "idle" && status.last_macro) {
      setPlayingMacroId(null);
      pushToast("success", 'Finished playing "' + status.last_macro + '"');
    }
    setMode(status.mode || "idle");
  }, [status, refreshMacros, pushToast]);

  // ── Recording ──────────────────────────────────────────────────────────────

  const toggleRecord = useCallback(async () => {
    if (busyRef.current) return;
    busyRef.current = true;
    try {
      if (modeRef.current === "recording") {
        const res = await stopRecord();
        if (res && res.ok) {
          pushToast("success", "Saved " + res.name + " — " + res.events + " events");
          setMode("idle");
          setRecordElapsed(0);
          setRecordEvents(res.events || 0);
          setRecordNamePromptId(res.name ?? null);
          setCountdown(0);
          await refreshMacros();
        } else {
          pushToast("error", (res && res.error) || "Stop failed");
        }
      } else if (modeRef.current === "idle") {
        setCountdown(3);
        for (let c = 3; c >= 1; c--) {
          setCountdown(c);
          await new Promise((r) => setTimeout(r, 700));
        }
        setCountdown(0);
        const res = await startRecord();
        if (res && res.ok) {
          setMode("recording");
          setRecordElapsed(0);
          setRecordEvents(0);
          pushToast("info", "Recording…");
        } else {
          pushToast("error", (res && res.error) || "Could not start recording");
        }
      } else {
        pushToast("error", "Busy — stop playback first");
      }
    } catch (e) {
      console.error(e);
      pushToast("error", "Record failed: " + e);
    } finally {
      busyRef.current = false;
    }
  }, [refreshMacros, pushToast]);

  const onPauseRecord = useCallback(async () => {
    try {
      const res = await pauseRecord();
      if (res && res.ok) {
        setRecordPaused(!!res.paused);
        pushToast("info", res.paused ? "Recording paused" : "Recording resumed");
      }
    } catch (e) {
      pushToast("error", "Pause failed: " + e);
    }
  }, [pushToast]);

  // ── Inline rename ────────────────────────────────────────────────────────

  const onRenameDraft = useCallback((id: string, name: string) => {
    setMacros((prev) => prev.map((m) => (m.id === id ? { ...m, name } : m)));
  }, []);

  const commitRename = useCallback(
    async (id: string, value: string) => {
      const name = (value ?? "").trim();
      // Close the editor first so a trailing blur can't double-commit.
      setRecordNamePromptId((prev) => (prev === id ? null : prev));
      if (!name || name === id) {
        await refreshMacros();
        return;
      }
      try {
        const res = await renameMacro(id, name);
        if (res && res.ok) {
          const safe = res.name ?? name;
          setMacros((prev) => prev.map((m) => (m.id === id ? { ...m, id: safe, name: safe } : m)));
          pushToast("success", 'Renamed to "' + safe + '"');
        } else {
          pushToast("error", (res && res.error) || "Rename failed");
          await refreshMacros();
        }
      } catch (e) {
        pushToast("error", "Rename failed");
        await refreshMacros();
      }
    },
    [refreshMacros, pushToast],
  );

  const focusRenameInput = useCallback((el: HTMLInputElement | null) => {
    if (el) {
      try {
        el.focus();
        el.select();
      } catch {
        /* ignore */
      }
    }
  }, []);

  // ── Playback ─────────────────────────────────────────────────────────────

  const play = useCallback(
    async (macro: MacroRow) => {
      if (modeRef.current !== "idle") return;
      if (busyRef.current) return;
      busyRef.current = true;
      try {
        const repeat = repsForIndex(macro.repeatIndex);
        const speed = parseFloat(macro.speed) || 1.0;
        const res = await playMacro(macro.name, repeat, speed);
        if (res && res.ok) {
          setMode("playing");
          setPlayElapsed(0);
          setPlayingMacroId(macro.id);
          const speedLabel = speed !== 1.0 ? " at " + speed + "×" : "";
          pushToast("info", 'Playing "' + macro.name + '"' + speedLabel);
        } else {
          pushToast("error", (res && res.error) || "Play failed");
        }
      } catch (e) {
        console.error(e);
        pushToast("error", "Play failed: " + e);
      } finally {
        busyRef.current = false;
      }
    },
    [pushToast],
  );

  const globalStop = useCallback(async () => {
    try {
      if (modeRef.current === "recording") {
        await toggleRecord();
        return;
      }
      const res = await emergencyStop();
      setMode("idle");
      setPlayingMacroId(null);
      pushToast("info", "Stopped.");
      if (res && !res.ok) console.warn(res);
    } catch (e) {
      pushToast("error", "Stop failed");
    }
  }, [toggleRecord, pushToast]);

  // ── Card mutations ───────────────────────────────────────────────────────

  const onDelete = useCallback(
    async (id: string) => {
      // Two-click confirm: first click arms for 3s, second click deletes.
      if (deleteConfirmRef.current !== id) {
        setDeleteConfirmId(id);
        if (deleteTimerRef.current) clearTimeout(deleteTimerRef.current);
        deleteTimerRef.current = setTimeout(() => setDeleteConfirmId(null), 3000);
        return;
      }
      if (deleteTimerRef.current) clearTimeout(deleteTimerRef.current);
      setDeleteConfirmId(null);
      try {
        const res = await deleteMacro(id);
        if (res && res.ok) {
          setMacros((prev) => prev.filter((m) => m.id !== id));
          pushToast("info", "Macro deleted.");
        } else {
          pushToast("error", (res && res.error) || "Delete failed");
        }
      } catch (e) {
        pushToast("error", "Delete failed");
      }
    },
    [pushToast],
  );

  const onDuplicate = useCallback(
    async (macro: MacroRow) => {
      try {
        const res = await duplicateMacro(macro.name);
        if (res && res.ok) {
          pushToast("success", 'Duplicated as "' + res.name + '"');
          await refreshMacros();
        } else {
          pushToast("error", (res && res.error) || "Duplicate failed");
        }
      } catch (e) {
        pushToast("error", "Duplicate failed: " + e);
      }
    },
    [refreshMacros, pushToast],
  );

  const onSetCategory = useCallback(
    async (macro: MacroRow) => {
      const cat = window.prompt(
        'Category for "' + macro.name + '" (leave empty to clear):',
        macro.category || "",
      );
      if (cat === null) return;
      try {
        const res = await apiSetCategory(macro.name, cat);
        if (res && res.ok) {
          pushToast("success", cat ? 'Category set to "' + cat + '"' : "Category cleared");
          await refreshMacros();
        } else {
          pushToast("error", (res && res.error) || "Failed to set category");
        }
      } catch (e) {
        pushToast("error", "Failed: " + e);
      }
    },
    [refreshMacros, pushToast],
  );

  const onSetNotes = useCallback(
    async (macro: MacroRow) => {
      const notes = window.prompt(
        'Notes for "' + macro.name + '" (leave empty to clear):',
        macro.notes || "",
      );
      if (notes === null) return;
      try {
        const res = await apiSetNotes(macro.name, notes);
        if (res && res.ok) {
          pushToast("success", notes ? "Notes updated" : "Notes cleared");
          await refreshMacros();
        } else {
          pushToast("error", (res && res.error) || "Failed to set notes");
        }
      } catch (e) {
        pushToast("error", "Failed: " + e);
      }
    },
    [refreshMacros, pushToast],
  );

  const onSaveAsTemplate = useCallback(
    async (macro: MacroRow) => {
      const tplName = window.prompt(
        'Template name (save "' + macro.name + '" as a reusable template):',
        macro.name + " template",
      );
      if (!tplName || !tplName.trim()) return;
      try {
        const res = await saveAsTemplate(macro.name, tplName.trim());
        if (res && res.ok) {
          pushToast("success", 'Saved template "' + tplName.trim() + '"');
          await loadTemplates();
        } else {
          pushToast("error", (res && res.error) || "Failed to save template");
        }
      } catch (e) {
        pushToast("error", "Failed: " + e);
      }
    },
    [loadTemplates, pushToast],
  );

  const onSpeedChange = useCallback((id: string, val: string) => {
    setMacros((prev) => prev.map((m) => (m.id === id ? { ...m, speed: val } : m)));
  }, []);

  // ── Selection & bulk delete ──────────────────────────────────────────────

  const toggleMacroSelection = useCallback((id: string) => {
    setSelectedMacros((sel) => (sel.includes(id) ? sel.filter((x) => x !== id) : [...sel, id]));
  }, []);

  const clearMacroSelection = useCallback(() => setSelectedMacros([]), []);
  const selectAllMacros = useCallback(() => {
    setSelectedMacros(macrosRef.current.map((m) => m.id));
  }, []);

  const bulkDeleteSelected = useCallback(async () => {
    const ids = selectedMacros;
    if (!ids.length) return;
    if (!window.confirm("Delete " + ids.length + " selected macro(s)? This cannot be undone.")) return;
    try {
      const res = await bulkDelete(ids);
      if (res && res.ok) {
        pushToast("success", "Deleted " + (res.deleted || []).length + " macro(s)");
        setSelectedMacros([]);
        await refreshMacros();
      } else {
        pushToast("error", "Bulk delete failed");
      }
    } catch (e) {
      pushToast("error", "Bulk delete failed: " + e);
    }
  }, [selectedMacros, refreshMacros, pushToast]);

  const onBulkExport = useCallback(async () => {
    const ids = selectedMacros;
    if (!ids.length) return;
    try {
      const res = await bulkExport(ids);
      if (res && res.ok) {
        pushToast("success", "Exported " + (res.exported || []).length + " macro(s)");
      } else if (res && res.error === "cancelled") {
        // user cancelled the folder dialog — no toast
      } else {
        pushToast("error", (res && res.error) || "Bulk export failed");
      }
    } catch (e) {
      pushToast("error", "Bulk export failed: " + e);
    }
  }, [selectedMacros, pushToast]);

  // ── Import / export ──────────────────────────────────────────────────────

  const onExportMacro = useCallback(
    async (macro: MacroRow) => {
      try {
        const res = await exportMacro(macro.name);
        if (res && res.ok) {
          pushToast("success", 'Exported "' + macro.name + '"');
        } else if (res && res.error !== "cancelled") {
          pushToast("error", res.error || "Export failed");
        }
      } catch (e) {
        pushToast("error", "Export failed: " + e);
      }
    },
    [pushToast],
  );

  const onImportMacro = useCallback(async () => {
    try {
      const res = await importMacro();
      if (res && res.ok) {
        pushToast("success", 'Imported "' + res.name + '"');
        await refreshMacros();
      } else if (res && res.error !== "cancelled") {
        pushToast("error", res.error || "Import failed");
      }
    } catch (e) {
      pushToast("error", "Import failed: " + e);
    }
  }, [refreshMacros, pushToast]);

  const onExportBundle = useCallback(
    async (macro: MacroRow) => {
      try {
        const res = await exportBundle(macro.name);
        if (res && res.ok) {
          pushToast("success", 'Bundle exported for "' + macro.name + '"');
        } else if (res && res.error !== "cancelled") {
          pushToast("error", res.error || "Bundle export failed");
        }
      } catch (e) {
        pushToast("error", "Bundle export failed: " + e);
      }
    },
    [pushToast],
  );

  const onImportBundle = useCallback(async () => {
    try {
      const res = await importBundle();
      if (res && res.ok) {
        pushToast("success", 'Bundle imported as "' + res.name + '"');
        await refreshMacros();
      } else if (res && res.error !== "cancelled") {
        pushToast("error", res.error || "Bundle import failed");
      }
    } catch (e) {
      pushToast("error", "Bundle import failed: " + e);
    }
  }, [refreshMacros, pushToast]);

  // ── Templates ────────────────────────────────────────────────────────────

  const onUseTemplate = useCallback(
    async (template: TemplateItem) => {
      const newName = window.prompt(
        'New macro name (from template "' + template.name + '"):',
        template.name + " copy",
      );
      if (!newName || !newName.trim()) return;
      try {
        const res = await createFromTemplate(template.name, newName.trim());
        if (res && res.ok) {
          pushToast("success", 'Created "' + newName.trim() + '" from template');
          await refreshMacros();
        } else {
          pushToast("error", (res && res.error) || "Failed to create from template");
        }
      } catch (e) {
        pushToast("error", "Failed: " + e);
      }
    },
    [refreshMacros, pushToast],
  );

  const onDeleteTemplate = useCallback(
    async (templateName: string) => {
      if (!window.confirm('Delete template "' + templateName + '"?')) return;
      try {
        const res = await deleteTemplate(templateName);
        if (res && res.ok) {
          pushToast("success", 'Deleted template "' + templateName + '"');
          await loadTemplates();
        } else {
          pushToast("error", (res && res.error) || "Failed to delete template");
        }
      } catch (e) {
        pushToast("error", "Failed: " + e);
      }
    },
    [loadTemplates, pushToast],
  );

  // ── Guards ───────────────────────────────────────────────────────────────

  const loadGuards = useCallback(async (macroId: string) => {
    try {
      const res = await guardList(macroId);
      setGuardsByMacro((prev) => ({ ...prev, [macroId]: (res.guards || []) as GuardDraft[] }));
    } catch (e) {
      console.error("guard_list", e);
    }
  }, []);

  const persistGuards = useCallback(
    async (macroId: string, guards: Guard[]) => {
      try {
        await guardSave(macroId, guards);
      } catch (e) {
        pushToast("error", "Guard save failed");
      }
    },
    [pushToast],
  );

  const toggleGuards = useCallback(
    (macroId: string) => {
      if (guardsOpenIdRef.current === macroId) {
        setGuardsOpenId(null);
        setGuardEditor(null);
      } else {
        setGuardsOpenId(macroId);
        setGuardEditor(null);
        loadGuards(macroId);
      }
    },
    [loadGuards],
  );

  const openGuardEditor = useCallback((macroId: string, guard: GuardDraft | null) => {
    const g: GuardDraft = guard || {
      id: "g" + Date.now(),
      name: "",
      method: "color",
      action: "click",
      key: "",
      hsv_low: [0, 0, 0],
      hsv_high: [179, 255, 255],
      region: [0, 0, 100, 100],
      min_area: 40,
      template_path: "",
      threshold: 0.8,
      ocr_text: "",
      resume_delay: 3,
      cooldown: 2,
      enabled: true,
      _isNew: true,
    };
    setGuardEditor({ ...g, macroId, _isNew: !guard, _preview: null });
  }, []);

  const closeGuardEditor = useCallback(() => setGuardEditor(null), []);

  const updateGuardEditor = useCallback((patch: Partial<GuardDraft>) => {
    setGuardEditor((prev) => (prev ? { ...prev, ...patch } : prev));
  }, []);

  const toggleGuard = useCallback(
    async (macroId: string, guardId: string) => {
      const guards = (guardsByMacroRef.current[macroId] || []).map((g) =>
        g.id === guardId ? { ...g, enabled: !g.enabled } : g,
      );
      setGuardsByMacro((prev) => ({ ...prev, [macroId]: guards }));
      await persistGuards(macroId, guards.map(({ _isNew, ...rest }) => rest));
    },
    [persistGuards],
  );

  const deleteGuard = useCallback(
    async (macroId: string, guardId: string) => {
      const guards = (guardsByMacroRef.current[macroId] || []).filter((g) => g.id !== guardId);
      setGuardsByMacro((prev) => ({ ...prev, [macroId]: guards }));
      await persistGuards(macroId, guards.map(({ _isNew, ...rest }) => rest));
      pushToast("info", "Guard removed.");
    },
    [persistGuards, pushToast],
  );

  const saveGuardEditor = useCallback(async () => {
    const ge = guardEditorRef.current;
    if (!ge) return;
    const macroId = ge.macroId as string;
    const clean: GuardDraft = {
      id: ge.id,
      name: ge.name || "Guard",
      method: ge.method || "color",
      action: ge.action,
      key: ge.key || "",
      hsv_low: ge.hsv_low,
      hsv_high: ge.hsv_high,
      region: ge.region,
      min_area: ge.min_area,
      template_path: ge.template_path || "",
      threshold: ge.threshold,
      ocr_text: ge.ocr_text || "",
      click_offset: ge.click_offset || [],
      click_line: ge.click_line || [],
      click_lines: ge.click_lines || [],
      resume_delay: ge.resume_delay,
      cooldown: ge.cooldown,
      enabled: true,
    };
    const existing = guardsByMacroRef.current[macroId] || [];
    const guards = ge._isNew
      ? [...existing, clean]
      : existing.map((g) => (g.id === clean.id ? clean : g));
    setGuardsByMacro((prev) => ({ ...prev, [macroId]: guards }));
    setGuardEditor(null);
    await persistGuards(macroId, guards);
    pushToast("success", 'Guard "' + clean.name + '" saved.');
  }, [persistGuards, pushToast]);

  const testGuard = useCallback(
    async (macroId: string, guard: GuardDraft) => {
      // If testing the guard currently being edited, use the editor's working copy.
      const ge = guardEditorRef.current;
      const target = ge && ge.macroId === macroId && ge.id === guard.id ? ge : guard;
      try {
        const res = await guardTest({
          id: target.id,
          name: target.name || "Guard",
          method: target.method || "color",
          action: target.action,
          key: target.key || "",
          hsv_low: target.hsv_low,
          hsv_high: target.hsv_high,
          region: target.region,
          min_area: target.min_area,
          template_path: target.template_path || "",
          threshold: target.threshold,
          ocr_text: target.ocr_text || "",
          resume_delay: target.resume_delay,
          cooldown: target.cooldown,
          enabled: true,
        });
        if (res && res.ok) {
          pushToast("success", "Match found at (" + res.found_x + ", " + res.found_y + ")");
        } else {
          pushToast("info", (res && res.message) || "No match");
        }
        // Show the preview in the editor if it's open for this macro.
        if (ge && ge.macroId === macroId && res && res.preview) {
          updateGuardEditor({
            _preview: { src: "data:image/jpeg;base64," + res.preview, ok: res.ok, msg: res.message },
          });
        }
      } catch (e) {
        pushToast("error", "Test failed: " + e);
      }
    },
    [pushToast, updateGuardEditor],
  );

  // ── Vision checkpoints ─────────────────────────────────────────────────────

  const loadCheckpoints = useCallback(async (macroId: string) => {
    try {
      const res = await listCheckpoints(macroId);
      if (res && res.ok) {
        setCheckpointsByMacro((prev) => ({ ...prev, [macroId]: res.checkpoints }));
      }
    } catch (e) {
      console.error("list_checkpoints", e);
    }
  }, []);

  const toggleVision = useCallback(
    (macroId: string) => {
      if (visionOpenIdRef.current === macroId) {
        setVisionOpenId(null);
        setVisionDraft(null);
      } else {
        setVisionOpenId(macroId);
        setVisionDraft(freshCheckpointDraft());
        loadCheckpoints(macroId);
      }
    },
    [loadCheckpoints],
  );

  const closeVision = useCallback(() => {
    setVisionOpenId(null);
    setVisionDraft(null);
  }, []);

  const updateVisionDraft = useCallback((patch: Partial<CheckpointDraft>) => {
    setVisionDraft((prev) => (prev ? { ...prev, ...patch } : prev));
  }, []);

  const addCheckpoint = useCallback(async () => {
    const macroId = visionOpenIdRef.current;
    const d = visionDraftRef.current;
    if (!macroId || !d) return;

    // Validate before building the config, matching the Python source.
    if (d.method === "color" && !d.hsv_low) {
      pushToast("error", "Pick a color first");
      return;
    }
    if (d.method === "template" && !d.template) {
      pushToast("error", "Capture an image first");
      return;
    }

    const checkpoint = {
      mode: d.mode,
      method: d.method,
      hsv_low: d.hsv_low,
      hsv_high: d.hsv_high,
      template: d.template,
      click_offset: d.click_offset,
      click_line: d.click_line,
      click_lines: d.click_lines,
      region: d.region || [0, 0, 100, 100],
      threshold: 0.75,
      timeout: parseFloat(String(d.timeout)) || 10,
      action: d.action,
      key: d.key || "",
      button: "left",
      min_area: 40,
      hold_button: "left",
      release_when: d.releaseWhen,
      poll: d.mode === "hold_follow" ? 0.02 : 0.05,
    };

    try {
      const res = await apiAddCheckpoint(macroId, -1, checkpoint);
      if (res && res.ok) {
        pushToast("success", "Checkpoint added");
        setVisionDraft(freshCheckpointDraft());
        await loadCheckpoints(macroId);
      } else {
        pushToast("error", (res && res.error) || "Failed to add checkpoint");
      }
    } catch (e) {
      pushToast("error", "Add failed: " + e);
    }
  }, [loadCheckpoints, pushToast]);

  const deleteCheckpoint = useCallback(
    async (macroId: string, index: number) => {
      try {
        const res = await apiRemoveCheckpoint(macroId, index);
        if (res && res.ok) {
          pushToast("info", "Checkpoint removed");
          await loadCheckpoints(macroId);
        }
      } catch {
        pushToast("error", "Delete failed");
      }
    },
    [loadCheckpoints, pushToast],
  );

  // ── Step editor ────────────────────────────────────────────────────────────

  const closeStepEditor = useCallback(() => {
    setStepsOpenId(null);
    setSteps([]);
    setStepsTestResult(null);
  }, []);

  const toggleStepEditor = useCallback(
    async (macroId: string) => {
      if (stepsOpenIdRef.current === macroId) {
        closeStepEditor();
        return;
      }
      setStepsOpenId(macroId);
      setStepsLoading(true);
      setSteps([]);
      setStepsTestResult(null);
      try {
        const res = await macroToSteps(macroId);
        if (res && res.ok) {
          setSteps(res.steps || []);
          setStepsLoading(false);
          pushToast("success", "Converted to " + res.count + " steps");
        } else {
          pushToast("error", (res && res.error) || "Conversion failed");
          setStepsLoading(false);
        }
      } catch (e) {
        pushToast("error", "Conversion failed: " + e);
        setStepsLoading(false);
      }
    },
    [closeStepEditor, pushToast],
  );

  const saveSteps = useCallback(async () => {
    const macroId = stepsOpenIdRef.current;
    if (!macroId) return;
    try {
      const res = await stepsSave(macroId, stepsRef.current);
      if (res && res.ok) pushToast("success", 'Steps saved as AI macro "' + macroId + '"');
      else pushToast("error", (res && res.error) || "Save failed");
    } catch (e) {
      pushToast("error", "Save failed: " + e);
    }
  }, [pushToast]);

  const runSteps = useCallback(async () => {
    try {
      const res = await stepsRun(stepsRef.current);
      if (res && res.ok) pushToast("info", "Running " + stepsRef.current.length + " steps...");
      else pushToast("error", (res && res.error) || "Run failed");
    } catch (e) {
      pushToast("error", "Run failed: " + e);
    }
  }, [pushToast]);

  const testStepAt = useCallback(async (idx: number) => {
    const step = stepsRef.current[idx];
    if (!step) return;
    setStepsTestResult(null);
    try {
      const res = await stepsTest(step);
      setStepsTestResult({
        idx,
        ok: !!(res && res.ok),
        message: (res && res.message) || "No result",
        preview: res && res.preview ? "data:image/jpeg;base64," + res.preview : null,
      });
    } catch (e) {
      setStepsTestResult({ idx, ok: false, message: "Test failed: " + e, preview: null });
    }
  }, []);

  // Sample a target colour for a detection step (color mode) — the port of the
  // dropped standalone AI view's `ai_pick_color`, folded into the in-card editor.
  // Reuses the guard colour picker; writes the HSV window onto the step so color
  // detection actually has a target (an unpicked step matches every pixel).
  const pickStepColor = useCallback(
    async (idx: number) => {
      try {
        const res = await guardPickColor();
        if (res && res.ok && res.hsv_low && res.hsv_high) {
          const low = res.hsv_low;
          const high = res.hsv_high;
          setSteps((prev) => {
            if (idx < 0 || idx >= prev.length) return prev;
            const next = [...prev];
            next[idx] = { ...next[idx], hsv_low: low, hsv_high: high };
            return next;
          });
          pushToast("success", "Color set: " + (res.hex || ""));
        } else if (res && res.error && res.error !== "cancelled") {
          pushToast("error", res.error);
        }
      } catch {
        pushToast("error", "Color pick failed");
      }
    },
    [pushToast],
  );

  // ── Repeat-dial drag ─────────────────────────────────────────────────────

  const startDrag = useCallback((id: string, e: PointerEvent<HTMLDivElement>) => {
    e.currentTarget.setPointerCapture(e.pointerId);
    dragRectRef.current = e.currentTarget.getBoundingClientRect();
    const macro = macrosRef.current.find((m) => m.id === id);
    const idx = macro ? macro.repeatIndex : 0;
    dragRef.current = { id, index: idx };
    setDragMacroId(id);
    setDragIndex(idx);
  }, []);

  const onDragMove = useCallback((id: string, e: PointerEvent<HTMLDivElement>) => {
    const rect = dragRectRef.current;
    if (!dragRef.current || dragRef.current.id !== id || !rect) return;
    const relX = Math.min(Math.max(e.clientX - rect.left, 0), rect.width);
    const idx = Math.round((relX / rect.width) * 5);
    if (idx !== dragRef.current.index) {
      dragRef.current = { id, index: idx };
      setDragIndex(idx);
    }
  }, []);

  const endDrag = useCallback((id: string) => {
    if (!dragRef.current || dragRef.current.id !== id) return;
    const finalIdx = dragRef.current.index;
    dragRef.current = null;
    dragRectRef.current = null;
    setMacros((prev) => prev.map((m) => (m.id === id ? { ...m, repeatIndex: finalIdx } : m)));
    setDragMacroId(null);
    setDragIndex(null);
    setSpinMacroId(id);
    setSpinTick(0);
    const macro = macrosRef.current.find((m) => m.id === id);
    if (macro) {
      setRepeat(macro.name, repsForIndex(finalIdx)).catch(() => {});
    }
    let n = 0;
    const iv = setInterval(() => {
      n++;
      setSpinTick(n);
      if (n >= 6) {
        clearInterval(iv);
        setSpinMacroId(null);
        setLandedMacroId(id);
        setTimeout(() => setLandedMacroId((cur) => (cur === id ? null : cur)), 650);
      }
    }, 70);
  }, []);

  // ── Derived view state ───────────────────────────────────────────────────

  const recording = mode === "recording" || mode === "paused";
  const recordButtonLabel = recording ? "Stop Recording" : "Record";
  const recordDotColor = recording
    ? mode === "paused"
      ? "var(--color-gold)"
      : "var(--verdict-blocked)"
    : "var(--color-gold)";
  const recordDotPulsing = recording && mode !== "paused";
  const recordDisabled = mode === "playing";
  const recordStatusLine = recording
    ? mode === "paused"
      ? "Paused · " + recordEvents + " actions so far — press Resume to keep going"
      : fmtDur(recordElapsed) + " · " + recordEvents + " actions recorded"
    : "Press Record, do your actions once, and Clawmation plays them back whenever you need.";

  const searchQ = macroSearch.toLowerCase().trim();
  const allCategories = Array.from(
    new Set(macros.map((m) => m.category).filter(Boolean)),
  ).sort();

  const visibleMacros = macros
    .filter(
      (m) =>
        !searchQ ||
        m.name.toLowerCase().includes(searchQ) ||
        (m.notes || "").toLowerCase().includes(searchQ) ||
        (m.category || "").toLowerCase().includes(searchQ),
    )
    .filter((m) => !categoryFilter || m.category === categoryFilter)
    .sort((a, b) => {
      switch (macroSort) {
        case "name":
          return a.name.localeCompare(b.name);
        case "plays":
          return (b.play_count || 0) - (a.play_count || 0);
        case "duration":
          return (b.durationSec || 0) - (a.durationSec || 0);
        case "events":
          return (b.events || 0) - (a.events || 0);
        default:
          return 0;
      }
    });

  const hasMacros = macros.length > 0;
  const noSearchResults = hasMacros && visibleMacros.length === 0;

  return (
    <Page
      title="Your macros."
      subtitle="Record once, play back as many times as you need."
    >
      {/* Record bar */}
      <Card
        pad="18px 22px"
        style={{
          marginBottom: 22,
          display: "flex",
          alignItems: "center",
          gap: 16,
        }}
      >
        <PrimaryButton
          onClick={toggleRecord}
          disabled={recordDisabled}
          style={{ flexShrink: 0 }}
          icon={
            <span
              style={{
                width: 9,
                height: 9,
                borderRadius: "50%",
                background: recordDotColor,
                animation: recordDotPulsing ? "vecRecPulse 1.2s ease-in-out infinite" : undefined,
              }}
            />
          }
        >
          {recordButtonLabel}
        </PrimaryButton>

        {recording && (
          <GhostButton
            onClick={onPauseRecord}
            style={{ flexShrink: 0 }}
            icon={recordPaused ? <Play size={14} /> : <Pause size={14} />}
          >
            {recordPaused ? "Resume" : "Pause"}
          </GhostButton>
        )}

        <GhostButton
          onClick={onImportMacro}
          title="Import a macro from a .json file"
          style={{ flexShrink: 0 }}
          icon={<UploadSimple size={14} />}
        >
          Import
        </GhostButton>

        <GhostButton
          onClick={onImportBundle}
          title="Import a macro bundle (.clawbundle)"
          style={{ flexShrink: 0 }}
          icon={<Package size={14} />}
        >
          Bundle
        </GhostButton>

        <div
          style={{
            fontFamily: "var(--font-sans)",
            fontSize: 12,
            color: "var(--text-50)",
            lineHeight: 1.45,
          }}
        >
          {recordStatusLine}
        </div>
      </Card>

      {hasMacros && (
        <>
          {/* Search / filter / sort */}
          <div style={{ marginBottom: 12, display: "flex", gap: 10, alignItems: "center" }}>
            <input
              value={macroSearch}
              onChange={(e) => setMacroSearch(e.target.value)}
              placeholder="Search name, notes, or category…"
              onFocus={(e) => (e.currentTarget.style.borderColor = "var(--color-gold)")}
              onBlur={(e) => (e.currentTarget.style.borderColor = "var(--border-default)")}
              style={{
                flex: 1,
                boxSizing: "border-box",
                height: 34,
                border: "1px solid var(--border-default)",
                borderRadius: "var(--radius-md)",
                padding: "0 12px",
                fontFamily: "var(--font-sans)",
                fontSize: 12.5,
                color: "var(--color-ink)",
                background: "var(--surface-card)",
                outline: "none",
                transition: "border-color .15s",
              }}
            />
            {allCategories.length > 0 && (
              <select
                value={categoryFilter}
                onChange={(e) => setCategoryFilter(e.target.value)}
                style={{
                  height: 34,
                  border: "1px solid var(--border-default)",
                  borderRadius: "var(--radius-md)",
                  padding: "0 10px",
                  fontFamily: "var(--font-sans)",
                  fontSize: 12,
                  color: "var(--color-ink)",
                  background: "var(--surface-card)",
                  outline: "none",
                  cursor: "pointer",
                  flexShrink: 0,
                }}
              >
                <option value="">All categories</option>
                {allCategories.map((cat) => (
                  <option key={cat} value={cat}>
                    {cat}
                  </option>
                ))}
              </select>
            )}
            <select
              value={macroSort}
              onChange={(e) => setMacroSort(e.target.value)}
              title="Sort macros"
              style={{
                height: 34,
                border: "1px solid var(--border-default)",
                borderRadius: "var(--radius-md)",
                padding: "0 10px",
                fontFamily: "var(--font-sans)",
                fontSize: 12,
                color: "var(--color-ink)",
                background: "var(--surface-card)",
                outline: "none",
                cursor: "pointer",
                flexShrink: 0,
              }}
            >
              <option value="recent">Recent</option>
              <option value="name">Name</option>
              <option value="plays">Most played</option>
              <option value="duration">Duration</option>
              <option value="events">Events</option>
            </select>
          </div>

          {/* Bulk-selection bar */}
          {selectedMacros.length > 0 && (
            <div
              style={{
                marginBottom: 12,
                padding: "10px 14px",
                background: "rgba(194,163,112,0.08)",
                border: "1px solid var(--gold-border)",
                borderRadius: "var(--radius-md)",
                display: "flex",
                alignItems: "center",
                gap: 12,
              }}
            >
              <span
                style={{
                  fontFamily: "var(--font-sans)",
                  fontSize: 12,
                  fontWeight: 600,
                  color: "var(--color-gold)",
                }}
              >
                {selectedMacros.length} selected
              </span>
              <button
                type="button"
                onClick={onBulkExport}
                title="Export selected macros to a folder"
                style={{
                  display: "inline-flex",
                  alignItems: "center",
                  gap: 6,
                  height: 28,
                  padding: "0 12px",
                  borderRadius: "var(--radius-sm)",
                  border: "1px solid var(--border-strong)",
                  background: "transparent",
                  color: "var(--text-78)",
                  fontFamily: "var(--font-sans)",
                  fontSize: 11.5,
                  fontWeight: 600,
                  cursor: "pointer",
                }}
              >
                <DownloadSimple size={12} />
                Export
              </button>
              <button
                type="button"
                onClick={bulkDeleteSelected}
                style={{
                  display: "inline-flex",
                  alignItems: "center",
                  gap: 6,
                  height: 28,
                  padding: "0 12px",
                  borderRadius: "var(--radius-sm)",
                  border: "1px solid var(--verdict-blocked)",
                  background: "transparent",
                  color: "var(--verdict-blocked)",
                  fontFamily: "var(--font-sans)",
                  fontSize: 11.5,
                  fontWeight: 600,
                  cursor: "pointer",
                }}
              >
                <Trash size={12} />
                Delete
              </button>
              <button
                type="button"
                onClick={clearMacroSelection}
                style={{
                  height: 28,
                  padding: "0 12px",
                  borderRadius: "var(--radius-sm)",
                  border: "1px solid var(--border-default)",
                  background: "transparent",
                  color: "var(--text-68)",
                  fontFamily: "var(--font-sans)",
                  fontSize: 11.5,
                  fontWeight: 600,
                  cursor: "pointer",
                }}
              >
                Clear
              </button>
              <button
                type="button"
                onClick={selectAllMacros}
                style={{
                  height: 28,
                  padding: "0 12px",
                  borderRadius: "var(--radius-sm)",
                  border: "1px solid var(--border-default)",
                  background: "transparent",
                  color: "var(--text-68)",
                  fontFamily: "var(--font-sans)",
                  fontSize: 11.5,
                  fontWeight: 600,
                  cursor: "pointer",
                }}
              >
                Select All
              </button>
            </div>
          )}

          {/* Macro list */}
          <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
            {visibleMacros.map((m) => {
              const dragging = dragMacroId === m.id;
              const spinning = spinMacroId === m.id;
              const displayIdx = dragging ? dragIndex ?? m.repeatIndex : m.repeatIndex;
              const shownIdx = spinning
                ? spinSeqFor(m.repeatIndex)[Math.min(spinTick, 5)]
                : displayIdx;
              const label = STOPS[shownIdx];
              const isInf = label === "∞";
              const landed = landedMacroId === m.id;
              const matchesSearch =
                !!searchQ &&
                (m.name.toLowerCase().includes(searchQ) ||
                  (m.notes || "").toLowerCase().includes(searchQ) ||
                  (m.category || "").toLowerCase().includes(searchQ));
              const isPlayingThis = mode === "playing" && playingMacroId === m.id;
              const playDisabled = mode !== "idle";
              const playOpacity = playDisabled && !isPlayingThis ? 0.5 : 1;
              const hasPlays = (m.play_count || 0) > 0;
              const editing = recordNamePromptId === m.id;
              const durationSec = m.durationSec || 1;
              const playProgressPct = isPlayingThis
                ? Math.min(100, ((playElapsed % durationSec) / durationSec) * 100)
                : 0;
              const playRepLabel = isPlayingThis
                ? playTotalReps > 0
                  ? "rep " + playIteration + "/" + playTotalReps
                  : "∞"
                : "";
              // Guards are lazy-loaded, so an unopened card shows a plain
              // "Guards" button with no count — the enabled count / badge only
              // appear once the panel is opened, matching the Python source.
              const loadedGuards = guardsByMacro[m.id] || [];
              const enabledCount = loadedGuards.filter((g) => g.enabled).length;
              const guardsOpen = guardsOpenId === m.id;
              const hasLoadedGuards = loadedGuards.length > 0;
              const isEditingHere =
                guardsOpen && !!guardEditor && guardEditor.macroId === m.id;
              const visionOpen = visionOpenId === m.id;
              const stepsOpen = stepsOpenId === m.id;
              return (
                <Card
                  key={m.id}
                  pad={0}
                  style={{
                    background: matchesSearch ? "rgba(194,163,112,0.05)" : "var(--surface-card)",
                    border:
                      "1px solid " + (matchesSearch ? "var(--color-gold)" : "var(--border-faint)"),
                    borderRadius: "var(--radius-lg)",
                    overflow: "hidden",
                    transition: "border-color .2s, background .2s",
                  }}
                >
                  <div
                    style={{
                      padding: "16px 18px",
                      display: "grid",
                      gridTemplateColumns: "auto minmax(0,1fr) auto auto",
                      gap: 18,
                      alignItems: "center",
                    }}
                  >
                    <input
                      type="checkbox"
                      checked={selectedMacros.includes(m.id)}
                      onChange={() => toggleMacroSelection(m.id)}
                      title="Select for batch operations"
                      style={{ width: 16, height: 16, cursor: "pointer", accentColor: "var(--color-gold)" }}
                    />

                    <div style={{ minWidth: 0 }}>
                      {editing ? (
                        <input
                          ref={focusRenameInput}
                          value={m.name}
                          onChange={(e) => onRenameDraft(m.id, e.target.value)}
                          onKeyDown={(e) => {
                            if (e.key === "Enter") {
                              e.preventDefault();
                              commitRename(m.id, e.currentTarget.value);
                            } else if (e.key === "Escape") {
                              e.preventDefault();
                              setRecordNamePromptId(null);
                              refreshMacros();
                            }
                          }}
                          onBlur={(e) => {
                            if (recordNamePromptRef.current === m.id) commitRename(m.id, e.currentTarget.value);
                          }}
                          onFocus={(e) => {
                            try {
                              e.currentTarget.select();
                            } catch {
                              /* ignore */
                            }
                          }}
                          style={{
                            width: "100%",
                            boxSizing: "border-box",
                            height: 30,
                            border: "1px solid var(--color-gold)",
                            borderRadius: "var(--radius-sm)",
                            padding: "0 8px",
                            fontFamily: "var(--font-sans)",
                            fontSize: 13,
                            fontWeight: 600,
                            color: "var(--color-ink)",
                            background: "var(--surface-card-strong)",
                            outline: "none",
                          }}
                        />
                      ) : (
                        <div
                          onClick={() => setRecordNamePromptId(m.id)}
                          title="Click to rename"
                          onMouseEnter={(e) => (e.currentTarget.style.background = "rgba(194,163,112,0.10)")}
                          onMouseLeave={(e) => (e.currentTarget.style.background = "transparent")}
                          style={{
                            fontFamily: "var(--font-sans)",
                            fontSize: 13.5,
                            fontWeight: 600,
                            color: "var(--color-ink)",
                            whiteSpace: "nowrap",
                            overflow: "hidden",
                            textOverflow: "ellipsis",
                            cursor: "text",
                            borderRadius: "var(--radius-xs)",
                            padding: "1px 4px",
                            margin: "-1px -4px",
                            transition: "background .15s",
                          }}
                        >
                          {m.name}
                        </div>
                      )}
                      <div
                        style={{
                          fontFamily: "var(--font-mono)",
                          fontSize: 10.5,
                          color: "var(--text-40)",
                          marginTop: 4,
                        }}
                      >
                        {m.events} events · {fmtDur(m.durationSec)} · {m.resolution}
                        {hasPlays && (
                          <>
                            {" · "}
                            <span style={{ color: "var(--color-gold)" }}>{m.play_count}×</span>
                            {" · "}
                            {fmtAgo(m.last_played)}
                          </>
                        )}
                      </div>
                      {!!m.category && (
                        <span
                          style={{
                            display: "inline-block",
                            marginTop: 4,
                            padding: "2px 8px",
                            borderRadius: "var(--radius-xs)",
                            background: "rgba(194,163,112,0.10)",
                            border: "1px solid var(--gold-border)",
                            fontFamily: "var(--font-mono)",
                            fontSize: 9.5,
                            fontWeight: 600,
                            color: "var(--color-gold)",
                            letterSpacing: "0.03em",
                          }}
                        >
                          {m.category}
                        </span>
                      )}
                      {!!m.notes && (
                        <div
                          style={{
                            marginTop: 5,
                            fontFamily: "var(--font-sans)",
                            fontSize: 11.5,
                            lineHeight: 1.45,
                            color: "var(--text-50)",
                            fontStyle: "italic",
                            maxWidth: 340,
                          }}
                        >
                          {m.notes}
                        </div>
                      )}
                    </div>

                    {/* Repeat dial */}
                    <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
                      <div
                        onPointerDown={(e) => startDrag(m.id, e)}
                        onPointerMove={(e) => onDragMove(m.id, e)}
                        onPointerUp={() => endDrag(m.id)}
                        style={{
                          position: "relative",
                          width: 132,
                          height: 20,
                          touchAction: "none",
                          cursor: "grab",
                        }}
                      >
                        <div
                          style={{
                            position: "absolute",
                            top: "50%",
                            left: 0,
                            width: "100%",
                            height: 4,
                            transform: "translateY(-50%)",
                            background: "rgba(26,24,22,0.08)",
                            borderRadius: 2,
                          }}
                        />
                        <div
                          style={{
                            position: "absolute",
                            top: "50%",
                            left: 0,
                            height: 4,
                            transform: "translateY(-50%)",
                            borderRadius: 2,
                            background: "var(--color-gold)",
                            width: (displayIdx / 5) * 100 + "%",
                          }}
                        />
                        <div
                          style={{
                            position: "absolute",
                            top: 2,
                            left: 0,
                            width: "100%",
                            display: "flex",
                            justifyContent: "space-between",
                          }}
                        >
                          {[0, 1, 2, 3, 4, 5].map((i) => (
                            <span
                              key={i}
                              style={{
                                width: 3,
                                height: 3,
                                borderRadius: "50%",
                                background: "var(--text-25)",
                              }}
                            />
                          ))}
                        </div>
                        <div
                          style={{
                            position: "absolute",
                            top: "50%",
                            left: (displayIdx / 5) * 100 + "%",
                            transform: "translate(-50%,-50%)",
                            width: 18,
                            height: 18,
                            borderRadius: "50%",
                            background: "var(--color-ink)",
                            border: "2px solid var(--color-gold)",
                            boxShadow: landed
                              ? isInf
                                ? "0 0 0 6px rgba(194,163,112,0.28)"
                                : "0 0 0 4px rgba(194,163,112,0.18)"
                              : "none",
                          }}
                        />
                      </div>
                      <div
                        style={{
                          fontFamily: "var(--font-mono)",
                          fontSize: 14,
                          fontWeight: 600,
                          color: isInf ? "var(--color-gold)" : "var(--color-ink)",
                          textAlign: "center",
                        }}
                      >
                        {label}
                        <span
                          style={{
                            fontSize: 9,
                            color: "var(--text-32)",
                            letterSpacing: "0.08em",
                            textTransform: "uppercase",
                            marginLeft: 4,
                          }}
                        >
                          reps
                        </span>
                      </div>
                    </div>

                    {/* Controls: reps live in the dial to the left; Run is the
                        one primary action — everything else folds into ⋯. */}
                    <div style={{ display: "flex", alignItems: "center", gap: 8, justifySelf: "end" }}>
                      <button
                        type="button"
                        onClick={() => (isPlayingThis ? globalStop() : play(m))}
                        disabled={playDisabled}
                        style={{
                          display: "inline-flex",
                          alignItems: "center",
                          justifyContent: "center",
                          gap: 7,
                          height: 34,
                          minWidth: 94,
                          padding: "0 18px",
                          borderRadius: "var(--radius-sm)",
                          border:
                            "1px solid " +
                            (isPlayingThis ? "var(--verdict-blocked)" : "var(--color-ink)"),
                          background: isPlayingThis ? "var(--verdict-blocked-bg)" : "var(--color-ink)",
                          color: isPlayingThis ? "var(--verdict-blocked)" : "var(--color-gold)",
                          fontFamily: "var(--font-sans)",
                          fontSize: 12.5,
                          fontWeight: 600,
                          cursor: "pointer",
                          opacity: playOpacity,
                          whiteSpace: "nowrap",
                        }}
                      >
                        {isPlayingThis ? <Stop size={13} weight="fill" /> : <Play size={13} weight="fill" />}
                        {isPlayingThis ? "Stop" : "Run"}
                      </button>

                      <OverflowMenu title="More actions">
                        {(close) => (
                          <>
                            <MenuRow
                              icon={<ShieldCheck size={15} />}
                              label="Guards"
                              hint={
                                enabledCount > 0
                                  ? enabledCount + " on"
                                  : hasLoadedGuards
                                    ? m.guardCount
                                    : undefined
                              }
                              active={guardsOpen || enabledCount > 0}
                              onClick={() => {
                                toggleGuards(m.id);
                                close();
                              }}
                            />
                            <MenuRow
                              icon={<List size={15} />}
                              label="Edit steps"
                              active={stepsOpen}
                              onClick={() => {
                                toggleStepEditor(m.id);
                                close();
                              }}
                            />
                            <MenuRow
                              icon={<Eye size={15} />}
                              label="Vision checkpoints"
                              active={visionOpen}
                              onClick={() => {
                                toggleVision(m.id);
                                close();
                              }}
                            />

                            {/* Speed is a value, not an action — chips keep every
                                exact rate without a native <select> fighting the
                                popover's outside-click handler. */}
                            <div style={{ padding: "9px 10px 3px" }}>
                              <div
                                style={{
                                  fontFamily: "var(--font-sans)",
                                  fontSize: 11,
                                  fontWeight: 600,
                                  color: "var(--text-50)",
                                  marginBottom: 7,
                                }}
                              >
                                Playback speed
                              </div>
                              <div style={{ display: "flex", flexWrap: "wrap", gap: 5 }}>
                                {["0.25", "0.5", "1", "1.5", "2", "4"].map((sp) => {
                                  const on = m.speed === sp;
                                  return (
                                    <button
                                      key={sp}
                                      type="button"
                                      disabled={playDisabled}
                                      onClick={() => onSpeedChange(m.id, sp)}
                                      style={{
                                        height: 26,
                                        minWidth: 42,
                                        padding: "0 8px",
                                        borderRadius: "var(--radius-sm)",
                                        border:
                                          "1px solid " +
                                          (on ? "var(--color-gold)" : "var(--border-default)"),
                                        background: on ? "rgba(194,163,112,0.12)" : "transparent",
                                        color: on ? "var(--color-gold)" : "var(--text-68)",
                                        fontFamily: "var(--font-mono)",
                                        fontSize: 11,
                                        fontWeight: 600,
                                        cursor: playDisabled ? "default" : "pointer",
                                        opacity: playDisabled ? 0.5 : 1,
                                      }}
                                    >
                                      {sp}×
                                    </button>
                                  );
                                })}
                              </div>
                            </div>

                            <div style={{ height: 1, background: "var(--border-faint)", margin: "6px" }} />

                            <MenuRow
                              icon={<Copy size={15} />}
                              label="Duplicate"
                              onClick={() => {
                                onDuplicate(m);
                                close();
                              }}
                            />
                            <MenuRow
                              icon={<DownloadSimple size={15} />}
                              label="Export to file"
                              onClick={() => {
                                onExportMacro(m);
                                close();
                              }}
                            />
                            <MenuRow
                              icon={<Package size={15} />}
                              label="Export bundle"
                              onClick={() => {
                                onExportBundle(m);
                                close();
                              }}
                            />
                            <MenuRow
                              icon={<Copy size={15} />}
                              label="Save as template"
                              onClick={() => {
                                onSaveAsTemplate(m);
                                close();
                              }}
                            />
                            <MenuRow
                              icon={<Tag size={15} />}
                              label="Category"
                              hint={m.category || undefined}
                              active={!!m.category}
                              onClick={() => {
                                onSetCategory(m);
                                close();
                              }}
                            />
                            <MenuRow
                              icon={<NotePencil size={15} />}
                              label="Notes"
                              active={!!m.notes}
                              onClick={() => {
                                onSetNotes(m);
                                close();
                              }}
                            />

                            <div style={{ height: 1, background: "var(--border-faint)", margin: "6px" }} />

                            <MenuRow
                              icon={<Trash size={15} />}
                              label={deleteConfirmId === m.id ? "Click again to delete" : "Delete"}
                              danger
                              onClick={() => onDelete(m.id)}
                            />
                          </>
                        )}
                      </OverflowMenu>
                    </div>
                  </div>

                  {/* Playback progress */}
                  {isPlayingThis && (
                    <>
                      <div style={{ position: "relative", height: 3, background: "rgba(26,24,22,0.08)" }}>
                        <div
                          style={{
                            position: "absolute",
                            top: 0,
                            left: 0,
                            height: "100%",
                            width: playProgressPct + "%",
                            background: "var(--verdict-allowed)",
                            transition: "width .4s linear",
                          }}
                        />
                      </div>
                      <div
                        style={{
                          display: "flex",
                          alignItems: "center",
                          justifyContent: "flex-end",
                          padding: "4px 18px 0",
                        }}
                      >
                        <span
                          style={{
                            fontFamily: "var(--font-mono)",
                            fontSize: 10,
                            color: "var(--verdict-allowed)",
                            letterSpacing: "0.04em",
                          }}
                        >
                          {playRepLabel}
                        </span>
                      </div>
                    </>
                  )}

                  {/* Guards panel */}
                  {guardsOpen && (
                    <div
                      style={{
                        borderTop: "1px solid var(--border-faint)",
                        background: "rgba(194,163,112,0.04)",
                        padding: "14px 18px 16px",
                      }}
                    >
                      <div
                        style={{
                          display: "flex",
                          alignItems: "center",
                          justifyContent: "space-between",
                          marginBottom: 10,
                        }}
                      >
                        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                          <span style={LABEL}>Guards</span>
                          <span
                            style={{
                              fontFamily: "var(--font-sans)",
                              fontSize: 11,
                              color: "var(--text-40)",
                            }}
                          >
                            watch the screen while this macro runs — pause it, handle the problem,
                            resume.
                          </span>
                        </div>
                        <AddButton
                          onClick={() => openGuardEditor(m.id, null)}
                          icon={<Plus size={12} />}
                        >
                          Add Guard
                        </AddButton>
                      </div>

                      {hasLoadedGuards && (
                        <div
                          style={{
                            display: "flex",
                            flexDirection: "column",
                            gap: 6,
                            marginBottom: 10,
                          }}
                        >
                          {loadedGuards.map((g) => {
                            const enabled = !!g.enabled;
                            const desc =
                              (g.action === "key" ? 'press "' + (g.key || "?") + '"' : "click it") +
                              " · wait " +
                              (g.resume_delay ?? 3) +
                              "s";
                            return (
                              <div
                                key={g.id}
                                style={{
                                  display: "grid",
                                  gridTemplateColumns: "auto 1fr auto auto auto auto",
                                  gap: 12,
                                  alignItems: "center",
                                  padding: "9px 12px",
                                  borderRadius: "var(--radius-md)",
                                  border: "1px solid var(--border-faint)",
                                  background: "var(--surface-card)",
                                  opacity: enabled ? 1 : 0.45,
                                }}
                              >
                                <span
                                  style={{
                                    width: 16,
                                    height: 16,
                                    borderRadius: 4,
                                    border: "1px solid var(--border-default)",
                                    background: hsvToCss(g.hsv_low, g.hsv_high),
                                    flexShrink: 0,
                                  }}
                                />
                                <div style={{ minWidth: 0 }}>
                                  <div
                                    style={{
                                      fontFamily: "var(--font-sans)",
                                      fontSize: 12,
                                      fontWeight: 600,
                                      color: "var(--color-ink)",
                                      whiteSpace: "nowrap",
                                      overflow: "hidden",
                                      textOverflow: "ellipsis",
                                    }}
                                  >
                                    {g.name || "Guard"}
                                  </div>
                                  <div
                                    style={{
                                      fontFamily: "var(--font-mono)",
                                      fontSize: 10,
                                      color: "var(--text-40)",
                                      marginTop: 2,
                                    }}
                                  >
                                    {desc}
                                  </div>
                                </div>
                                <button
                                  type="button"
                                  onClick={() => openGuardEditor(m.id, g)}
                                  title="Edit"
                                  style={{
                                    display: "inline-flex",
                                    alignItems: "center",
                                    justifyContent: "center",
                                    width: 24,
                                    height: 24,
                                    border: "none",
                                    background: "transparent",
                                    color: "var(--text-50)",
                                    cursor: "pointer",
                                  }}
                                >
                                  <PencilSimple size={13} />
                                </button>
                                <button
                                  type="button"
                                  onClick={() => testGuard(m.id, g)}
                                  title="Test now"
                                  style={{
                                    display: "inline-flex",
                                    alignItems: "center",
                                    justifyContent: "center",
                                    width: 24,
                                    height: 24,
                                    border: "none",
                                    background: "transparent",
                                    color: "var(--text-50)",
                                    cursor: "pointer",
                                  }}
                                >
                                  <Eye size={14} />
                                </button>
                                <button
                                  type="button"
                                  onClick={() => toggleGuard(m.id, g.id)}
                                  style={{
                                    width: 30,
                                    height: 17,
                                    borderRadius: 9,
                                    border: "none",
                                    background: enabled
                                      ? "var(--color-ink)"
                                      : "rgba(26,24,22,0.15)",
                                    position: "relative",
                                    cursor: "pointer",
                                    flexShrink: 0,
                                  }}
                                >
                                  <span
                                    style={{
                                      display: "block",
                                      width: 12,
                                      height: 12,
                                      borderRadius: "50%",
                                      background: "#fff",
                                      position: "absolute",
                                      top: 2.5,
                                      left: enabled ? 15 : 2.5,
                                      transition: "left .18s",
                                    }}
                                  />
                                </button>
                                <button
                                  type="button"
                                  onClick={() => deleteGuard(m.id, g.id)}
                                  title="Delete"
                                  style={{
                                    display: "inline-flex",
                                    alignItems: "center",
                                    justifyContent: "center",
                                    width: 24,
                                    height: 24,
                                    border: "none",
                                    background: "transparent",
                                    color: "var(--text-40)",
                                    cursor: "pointer",
                                  }}
                                >
                                  <Trash size={13} />
                                </button>
                              </div>
                            );
                          })}
                        </div>
                      )}

                      {!hasLoadedGuards && (
                        <div
                          style={{
                            border: "1px dashed var(--border-default)",
                            borderRadius: "var(--radius-md)",
                            padding: 16,
                            textAlign: "center",
                            marginBottom: 10,
                          }}
                        >
                          <span
                            style={{
                              fontFamily: "var(--font-sans)",
                              fontSize: 11.5,
                              color: "var(--text-40)",
                            }}
                          >
                            No guards yet. Add one to auto-click a button (like Reconnect) when it
                            appears.
                          </span>
                        </div>
                      )}

                      {isEditingHere && guardEditor && (
                        <GuardEditor
                          draft={guardEditor}
                          macros={macros.map((mm) => ({ id: mm.id, name: mm.name }))}
                          onChange={updateGuardEditor}
                          onTest={() => testGuard(m.id, guardEditor)}
                          onSave={saveGuardEditor}
                          onCancel={closeGuardEditor}
                          push={pushToast}
                        />
                      )}
                    </div>
                  )}

                  {/* Step editor panel */}
                  {stepsOpen && (
                    <StepEditor
                      steps={steps}
                      loading={stepsLoading}
                      testResult={stepsTestResult}
                      setSteps={setSteps}
                      onSave={saveSteps}
                      onRun={runSteps}
                      onTest={testStepAt}
                      onPickColor={pickStepColor}
                      onClose={closeStepEditor}
                    />
                  )}

                  {/* Vision checkpoints panel */}
                  {visionOpen && visionDraft && (
                    <CheckpointEditor
                      draft={visionDraft}
                      checkpoints={checkpointsByMacro[m.id] || []}
                      onChange={updateVisionDraft}
                      onAdd={addCheckpoint}
                      onDelete={(index) => deleteCheckpoint(m.id, index)}
                      onClose={closeVision}
                      push={pushToast}
                    />
                  )}
                </Card>
              );
            })}
          </div>

          {noSearchResults && (
            <EmptyState
              icon={<MagnifyingGlass size={28} weight="thin" />}
              title="No matches found."
              hint="Try a different search term or clear the filters above."
              style={{ marginTop: 16 }}
            />
          )}
        </>
      )}

      {/* Empty state */}
      {!hasMacros && (
        <div
          style={{
            border: "1px dashed var(--border-default)",
            background: "rgba(255,255,255,0.35)",
            borderRadius: "var(--radius-lg)",
            padding: "48px 28px",
            textAlign: "center",
          }}
        >
          <div
            style={{
              width: 56,
              height: 56,
              borderRadius: "50%",
              background: "rgba(194,163,112,0.10)",
              border: "1px solid var(--gold-border)",
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
              margin: "0 auto 18px",
            }}
          >
            <Record size={26} color="var(--color-gold)" />
          </div>
          <div
            style={{
              fontFamily: "var(--font-serif)",
              fontSize: 22,
              fontWeight: 300,
              color: "var(--color-ink)",
            }}
          >
            No macros yet.
          </div>
          <div className="t-body-sm" style={{ margin: "10px auto 0", maxWidth: "44ch" }}>
            Record your first macro and Clawmation will replay every click, key, and pause exactly as you
            did it.
          </div>
          <div
            style={{
              display: "flex",
              flexDirection: "column",
              gap: 2,
              maxWidth: 420,
              margin: "24px auto 0",
              textAlign: "left",
            }}
          >
            {[
              <>
                Open the app or game you want to automate, then press <strong>Record</strong> above.
              </>,
              <>Do the actions once. Clawmation captures them in real time.</>,
              <>
                Press <strong>Stop</strong> — it's saved automatically, ready to replay on a loop.
              </>,
            ].map((text, i) => (
              <div
                key={i}
                style={{
                  display: "grid",
                  gridTemplateColumns: "24px 1fr",
                  gap: 12,
                  padding: "9px 4px",
                  borderBottom: i < 2 ? "1px solid var(--border-faint)" : undefined,
                }}
              >
                <span style={{ fontFamily: "var(--font-mono)", fontSize: 11, color: "var(--color-gold)" }}>
                  {i + 1}
                </span>
                <span className="t-body-sm">{text}</span>
              </div>
            ))}
          </div>
          <PrimaryButton
            onClick={toggleRecord}
            icon={<Record size={15} />}
            style={{ marginTop: 26, height: 40, padding: "0 22px" }}
          >
            Record your first macro
          </PrimaryButton>
        </div>
      )}

      {/* Templates */}
      {templates.length > 0 && (
        <div style={{ marginTop: 28, paddingTop: 20, borderTop: "1px solid var(--border-faint)" }}>
          <CardHead
            title="Templates"
            action={
              <span
                style={{ fontFamily: "var(--font-mono)", fontSize: 10, color: "var(--text-40)" }}
              >
                {templates.length} saved
              </span>
            }
          />
          <div
            style={{
              display: "grid",
              gridTemplateColumns: "repeat(auto-fill,minmax(220px,1fr))",
              gap: 10,
            }}
          >
            {templates.map((tpl) => (
              <Card key={tpl.name} pad={14} style={{ borderRadius: "var(--radius-md)" }}>
                <div
                  style={{
                    fontFamily: "var(--font-sans)",
                    fontSize: 13,
                    fontWeight: 600,
                    color: "var(--color-ink)",
                    marginBottom: 6,
                  }}
                >
                  {tpl.name}
                </div>
                <div
                  style={{
                    fontFamily: "var(--font-mono)",
                    fontSize: 10,
                    color: "var(--text-40)",
                    marginBottom: 10,
                  }}
                >
                  {tpl.events} events · {fmtDur(tpl.duration)}
                </div>
                <div style={{ display: "flex", gap: 6 }}>
                  <button
                    type="button"
                    onClick={() => onUseTemplate(tpl)}
                    style={{
                      flex: 1,
                      height: 26,
                      borderRadius: "var(--radius-sm)",
                      border: "1px solid var(--color-gold)",
                      background: "rgba(194,163,112,0.08)",
                      color: "var(--color-gold)",
                      fontFamily: "var(--font-sans)",
                      fontSize: 11,
                      fontWeight: 600,
                      cursor: "pointer",
                    }}
                  >
                    Use
                  </button>
                  <button
                    type="button"
                    onClick={() => onDeleteTemplate(tpl.name)}
                    title="Delete template"
                    style={{
                      width: 26,
                      height: 26,
                      borderRadius: "var(--radius-sm)",
                      border: "1px solid var(--border-default)",
                      background: "transparent",
                      color: "var(--text-40)",
                      cursor: "pointer",
                    }}
                  >
                    <Trash size={12} />
                  </button>
                </div>
              </Card>
            ))}
          </div>
        </div>
      )}

      {/* 3-2-1 countdown overlay */}
      {countdown > 0 && (
        <div
          style={{
            position: "fixed",
            inset: 0,
            zIndex: 500,
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            background: "rgba(26,24,22,0.75)",
          }}
        >
          <div style={{ textAlign: "center" }}>
            <div
              style={{
                fontFamily: "var(--font-mono)",
                fontSize: 96,
                fontWeight: 700,
                color: "var(--color-gold)",
                lineHeight: 1,
                animation: "vecPulse .7s ease-in-out infinite",
              }}
            >
              {countdown}
            </div>
            <div
              style={{
                fontFamily: "var(--font-sans)",
                fontSize: 14,
                color: "rgba(250,248,245,0.7)",
                marginTop: 12,
                letterSpacing: "0.02em",
              }}
            >
              Get ready to record…
            </div>
          </div>
        </div>
      )}
    </Page>
  );
}
