import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Loader2, Search } from "lucide-react";

import {
  listChains,
  listMacros,
  playMacro,
  runChain,
  visionLoad,
  visionSave,
  type Guard,
  type MacroListItem,
} from "@/api";
import { describeTrigger, draftFromGuard } from "@/lib/triggers";
import { cn } from "@/lib/utils";

/**
 * The macro launcher: a Raycast/PowerToys-style palette the play hotkey opens.
 * Lists every macro, every saved workflow chain, AND every Watch trigger — macros
 * first (most-recently-played on top), then chains by name, then watch triggers.
 * Each macro row shows how long one run takes and its category, plus cumulative
 * time played and play count; each chain row shows how many macros it runs and
 * its repeat; each watch row shows what it looks for and whether it is on.
 * Filter by typing, move with ↑/↓, Enter acts on the highlighted item — a macro
 * plays, a chain runs, a watch trigger is switched on/off — Esc, or just clicking
 * back into the game, dismisses: the window hides itself on blur, so this
 * component only handles picking.
 */

/** One selectable row: a macro, a chain, or a watch trigger, normalized for the
 *  flat list. */
type Entry = {
  kind: "macro" | "chain" | "watch";
  /** Stable React key; also the value handed to play/run. */
  id: string;
  name: string;
  /** Secondary line under the name (run length + category for macros, size for
   *  chains, what-it-watches-for for watch triggers). */
  sub: string;
  /** Right-aligned stats (time · count for macros, repeat for chains, on/off for
   *  watch triggers). */
  right: string;
  /** Watch triggers only: the full guard (to flip its `enabled`) and its current
   *  on/off state, so the row can be toggled and drawn accordingly. */
  guard?: Guard;
  enabled?: boolean;
};

export function Launcher() {
  const [macros, setMacros] = useState<MacroListItem[]>([]);
  const [chains, setChains] = useState<{ id: string; name: string; macroCount: number; repeat: number }[]>([]);
  const [watches, setWatches] = useState<Guard[]>([]);
  const [loading, setLoading] = useState(true);
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  const load = useCallback(async () => {
    try {
      const [macroList, chainList, watchList] = await Promise.all([
        listMacros(),
        listChains(),
        visionLoad().catch(() => ({ ok: false, triggers: [] as Guard[] })),
      ]);
      setMacros(macroList);
      setChains(
        chainList
          .filter((c): c is typeof c & { id: string; name: string } => Boolean(c.id && c.name))
          .map((c) => ({
            id: c.id as string,
            name: c.name as string,
            macroCount: c.macro_names?.length ?? 0,
            repeat: c.repeat ?? 1,
          })),
      );
      setWatches(watchList.ok ? (watchList.triggers ?? []) : []);
    } finally {
      setLoading(false);
    }
  }, []);

  // Load on mount, then refresh whenever the window regains focus — the hotkey
  // shows (focuses) the window, so the list is re-read and the query cleared on
  // every summon. What you see is always the current macro/chain/watch set.
  useEffect(() => {
    void load();
    const win = getCurrentWindow();
    const unlisten = win.onFocusChanged(({ payload: focused }) => {
      if (focused) {
        setQuery("");
        setSelected(0);
        void load();
        inputRef.current?.focus();
      }
    });
    return () => {
      void unlisten.then((off) => off());
    };
  }, [load]);

  // Case-insensitive substring filter. Macros first, most-recently-played on top
  // (never-played, `last_played` 0, fall to the bottom of the macro block), then
  // chains alphabetically, then watch triggers alphabetically — one flat list so
  // the arrow keys flow straight through.
  const entries = useMemo<Entry[]>(() => {
    const q = query.trim().toLowerCase();
    const match = (name: string) => !q || name.toLowerCase().includes(q);
    const macroEntries: Entry[] = macros
      .filter((m) => match(m.name))
      .sort((a, b) => b.last_played - a.last_played || a.name.localeCompare(b.name))
      .map((m) => ({
        kind: "macro",
        id: m.name,
        name: m.name,
        sub: [m.category, fmtDuration(m.duration)].filter(Boolean).join(" · "),
        right: `${fmtPlayed(m.played)} · ×${m.play_count}`,
      }));
    const chainEntries: Entry[] = chains
      .filter((c) => match(c.name))
      .sort((a, b) => a.name.localeCompare(b.name))
      .map((c) => ({
        kind: "chain",
        id: c.id,
        name: c.name,
        sub: `${c.macroCount} macro${c.macroCount === 1 ? "" : "s"}`,
        right: c.repeat > 1 ? `×${c.repeat}` : "chain",
      }));
    const watchEntries: Entry[] = watches
      .filter((g) => match(String(g.name) || "Untitled watch"))
      .sort((a, b) => (String(a.name) || "").localeCompare(String(b.name) || ""))
      .map((g) => {
        const enabled = g.enabled !== false;
        return {
          kind: "watch",
          id: String(g.id),
          name: String(g.name) || "Untitled watch",
          sub: describeTrigger(draftFromGuard(g)),
          right: enabled ? "on" : "off",
          guard: g,
          enabled,
        };
      });
    return [...macroEntries, ...chainEntries, ...watchEntries];
  }, [macros, chains, watches, query]);

  // A filter change can leave the selection past the end; snap it back to the top.
  useEffect(() => {
    setSelected(0);
  }, [query]);

  // Flip a watch trigger's `enabled` and persist the whole set (the backend, like
  // the Watch view, stores the file as the unit of truth). Unlike a played macro
  // or a run chain, this does NOT dismiss the palette — switching triggers on/off
  // is a settings-style action you do several of in a row, so the list stays open
  // and updates in place.
  const toggleWatch = useCallback(async (guard: Guard) => {
    const enabled = guard.enabled === false; // flip: off→on, on→off
    const next = watches.map((g) => (String(g.id) === String(guard.id) ? { ...g, enabled } : g));
    setWatches(next);
    await visionSave(next);
  }, [watches]);

  const run = useCallback(
    async (entry: Entry) => {
      if (entry.kind === "watch") {
        if (entry.guard) await toggleWatch(entry.guard);
        return; // stay open: toggling is not a screen-takeover like playing a macro
      }
      if (entry.kind === "macro") {
        await playMacro(entry.id);
      } else {
        await runChain(entry.id);
      }
      await getCurrentWindow().hide();
    },
    [toggleWatch],
  );

  // Arrows move the selection, Enter acts on it, Esc dismisses. Bound on the
  // document so they work whether the input or a row has focus.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setSelected((s) => Math.min(s + 1, entries.length - 1));
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        setSelected((s) => Math.max(s - 1, 0));
      } else if (e.key === "Enter") {
        e.preventDefault();
        const target = entries[selected];
        if (target) void run(target);
      } else if (e.key === "Escape") {
        e.preventDefault();
        void getCurrentWindow().hide();
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [entries, selected, run]);

  // Keep the highlighted row visible as the selection moves off-screen.
  useEffect(() => {
    document.getElementById(`launcher-row-${selected}`)?.scrollIntoView({ block: "nearest" });
  }, [selected]);

  const isEmpty = macros.length === 0 && chains.length === 0 && watches.length === 0;

  return (
    <div className="flex h-screen flex-col overflow-hidden rounded-lg border border-border bg-background text-foreground">
      {/* Search bar. Doubles as the drag region so the palette can be moved by
          its header; the input itself still takes typing and clicks. */}
      <div
        data-tauri-drag-region
        className="flex shrink-0 items-center gap-2.5 border-b border-border px-4 py-3"
      >
        <Search className="h-4 w-4 shrink-0 text-muted-foreground" />
        <input
          ref={inputRef}
          autoFocus
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search macros, chains and watch…"
          spellCheck={false}
          className="w-full bg-transparent text-sm outline-none placeholder:text-muted-foreground"
        />
      </div>

      {/* Macro + chain + watch list. */}
      <div className="flex-1 overflow-y-auto py-1">
        {loading ? (
          <div className="flex items-center justify-center gap-2 py-12 text-sm text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin" />
            Loading…
          </div>
        ) : entries.length === 0 ? (
          <div className="px-4 py-12 text-center text-sm text-muted-foreground">
            {isEmpty ? "No macros, chains or watch triggers yet." : "Nothing matches that search."}
          </div>
        ) : (
          entries.map((entry, i) => (
            <button
              key={`${entry.kind}:${entry.id}`}
              id={`launcher-row-${i}`}
              type="button"
              onMouseEnter={() => setSelected(i)}
              onClick={() => void run(entry)}
              className={cn(
                "flex w-full items-center gap-3 px-4 py-2 text-left transition-colors",
                i === selected ? "bg-muted" : "hover:bg-muted/50",
              )}
            >
              {/* Type badge: a macro plays outright, a chain runs its macros in
                  sequence, a watch trigger toggles on/off. Kept tiny so the name
                  still reads first. */}
              <span
                className={cn(
                  "w-12 shrink-0 rounded px-1.5 py-0.5 text-center text-[10px] font-semibold uppercase tracking-wide",
                  entry.kind === "chain"
                    ? "bg-accent text-accent-foreground"
                    : entry.kind === "watch"
                      ? "bg-primary/15 text-primary"
                      : "bg-muted text-muted-foreground",
                )}
              >
                {entry.kind}
              </span>
              <div className="min-w-0 flex-1">
                <p className="truncate text-sm font-medium">{entry.name}</p>
                <p className="truncate text-xs text-muted-foreground">{entry.sub}</p>
              </div>
              <span
                className={cn(
                  "shrink-0 text-xs tabular-nums",
                  entry.kind === "watch"
                    ? entry.enabled
                      ? "font-semibold text-primary"
                      : "text-muted-foreground/60"
                    : "text-muted-foreground",
                )}
              >
                {entry.right}
              </span>
            </button>
          ))
        )}
      </div>

      {/* Hint bar. */}
      <div className="flex shrink-0 items-center gap-3 border-t border-border px-4 py-2 text-[11px] text-muted-foreground">
        <span>↑↓ navigate</span>
        <span>↵ run / toggle</span>
        <span>esc dismiss</span>
      </div>
    </div>
  );
}

/** How long one run of a macro takes: `<1s`, `8s`, `2m 30s`, `1h 5m`. Unlike the
 *  cumulative `fmtPlayed`, a never-banked / sub-second macro still shows a real
 *  length (`<1s`) rather than a dash — it is the recording's own duration. */
function fmtDuration(secs: number): string {
  const s = Math.max(0, Math.round(secs));
  if (s < 1) return "<1s";
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ${s % 60}s`.trim();
  const h = Math.floor(m / 60);
  return `${h}h ${m % 60}m`.trim();
}

/** Human-friendly cumulative play time: `45s`, `12m 5s`, `3h 20m`, or `—` when a
 *  macro has never completed a run. */
function fmtPlayed(secs: number): string {
  if (!secs || secs <= 0) return "—";
  const s = Math.round(secs);
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ${s % 60}s`.trim();
  const h = Math.floor(m / 60);
  return `${h}h ${m % 60}m`.trim();
}
