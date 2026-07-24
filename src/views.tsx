import { useEffect, useRef, useState, type CSSProperties } from "react";
import { Monitor, Play } from "@phosphor-icons/react";

import {
  getRunHistory,
  listMacros,
  playMacro,
  type HistoryEntry,
  type LogEntry,
  type MacroListItem,
  type Status,
} from "./api";
import { ChainsView } from "./ChainsView";
import { GuideView } from "./GuideView";
import { GuardsView } from "./GuardsView";
import { MacrosView } from "./MacrosView";
import { SettingsView } from "./SettingsView";
import { VisionView } from "./VisionView";
import { useToast } from "./components/Toasts";
import { CARD, Page } from "./components/ui";
import { fmtAgo, fmtDur, repeatToIndex, repsFor } from "./format";
import type { ViewId } from "./nav";

// ─── Dashboard ────────────────────────────────────────────────────────────────
// Faithful port of the Python UI's Dashboard screen. The mode / target facts and
// the activity log read off the shared `get_status` heartbeat (passed in as
// `status`); the recent-run log and recent-macro quick-play poll their own
// commands while this screen is mounted.
//
// The Python dashboard's Record / Play-Last action row is intentionally omitted:
// Record needs the pre-record countdown and optimistic-mode state machine the
// Macros screen owns locally, and a clean second record entry point would mean
// lifting that state to a shared owner — deferred. Play-Last is covered by the
// Recent Macros quick-play below, and the primary record + play controls live one
// click away on the Macros screen.

// Run-history + macro-list refresh cadence, gentler than the 500ms status
// heartbeat: a one-second lag on the recent-runs log and quick-play shelf is fine.
const DASH_POLL_MS = 1000;

const cardBase: CSSProperties = { ...CARD };
const factCard: CSSProperties = { ...cardBase, padding: "18px 20px" };
const sectionCard: CSSProperties = { ...cardBase, padding: "18px 20px" };

const serif: CSSProperties = {
  fontFamily: "var(--font-serif)",
  fontWeight: 400,
  color: "var(--color-ink)",
};
const microLabel: CSSProperties = {
  fontFamily: "var(--font-mono)",
  fontSize: 10,
  color: "var(--text-40)",
  marginTop: 2,
};

interface ModeMeta {
  label: string;
  color: string;
  anim: string;
}

// The dashboard's mode fact. Its default label is "Idle" — distinct from the
// TopBar's "Running" default for the same state — so it stays a local helper.
function dashMode(mode: string): ModeMeta {
  const anim = mode !== "idle" ? "vecPulse 1.4s ease-in-out infinite" : "none";
  switch (mode) {
    case "recording":
      return { label: "Recording", color: "var(--verdict-blocked)", anim };
    case "paused":
      return { label: "Paused", color: "var(--color-gold)", anim };
    case "playing":
      return { label: "Playing", color: "var(--verdict-allowed)", anim };
    default:
      return { label: "Idle", color: "var(--text-32)", anim };
  }
}

function FactCards({
  status,
  lastPlayed,
}: {
  status: Status | null;
  lastPlayed: { name: string; when: string } | null;
}) {
  const mode = dashMode(status?.mode ?? "idle");
  return (
    <div style={{ display: "grid", gridTemplateColumns: "repeat(2, 1fr)", gap: 14 }}>
      <div style={factCard}>
        <div className="t-label">Mode</div>
        <div style={{ display: "flex", alignItems: "center", gap: 7, marginTop: 8 }}>
          <span
            style={{ width: 7, height: 7, borderRadius: "50%", background: mode.color, animation: mode.anim }}
          />
          <span style={{ ...serif, fontSize: 19 }}>{mode.label}</span>
        </div>
      </div>
      <div style={factCard}>
        <div className="t-label">Last Macro Played</div>
        <div style={{ ...serif, fontSize: 16, marginTop: 8 }}>{lastPlayed ? lastPlayed.name : "—"}</div>
        <div style={microLabel}>{lastPlayed ? lastPlayed.when : "Nothing played yet"}</div>
      </div>
    </div>
  );
}

function RecentMacroChip({ name, onPlay }: { name: string; onPlay: () => void }) {
  const [hover, setHover] = useState(false);
  return (
    <button
      type="button"
      onClick={onPlay}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      title={`Play '${name}'`}
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: 7,
        height: 32,
        padding: "0 12px",
        borderRadius: "var(--radius-sm)",
        border: `1px solid ${hover ? "var(--color-gold)" : "var(--border-default)"}`,
        background: hover ? "rgba(194,163,112,0.06)" : "transparent",
        color: "var(--text-68)",
        fontFamily: "var(--font-sans)",
        fontSize: 12,
        fontWeight: 500,
        cursor: "pointer",
        transition: "border-color .15s, background .15s",
      }}
    >
      <Play size={11} color="var(--color-gold)" />
      {name}
    </button>
  );
}

function RecentMacrosCard({
  macros,
  onPlay,
}: {
  macros: MacroListItem[];
  onPlay: (m: MacroListItem) => void;
}) {
  return (
    <div style={sectionCard}>
      <div className="t-label" style={{ marginBottom: 12 }}>
        Recent Macros
      </div>
      <div style={{ display: "flex", flexWrap: "wrap", gap: 8 }}>
        {macros.map((m) => (
          <RecentMacroChip key={m.name} name={m.name} onPlay={() => onPlay(m)} />
        ))}
      </div>
    </div>
  );
}

function RecentRunsCard({ runs }: { runs: HistoryEntry[] }) {
  return (
    <div style={sectionCard}>
      <div className="t-label" style={{ marginBottom: 12 }}>
        Recent Runs
      </div>
      <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
        {runs.slice(0, 8).map((h, i) => {
          const running = h.status === "running";
          return (
            <div
              key={i}
              style={{
                display: "flex",
                alignItems: "center",
                gap: 10,
                padding: "6px 0",
                borderBottom: "1px solid var(--border-faint)",
              }}
            >
              <span
                style={{
                  width: 6,
                  height: 6,
                  borderRadius: "50%",
                  background: running ? "var(--verdict-allowed)" : "var(--text-32)",
                  flexShrink: 0,
                }}
              />
              <span
                style={{
                  flex: 1,
                  fontFamily: "var(--font-sans)",
                  fontSize: 12,
                  fontWeight: 500,
                  color: "var(--color-ink)",
                  whiteSpace: "nowrap",
                  overflow: "hidden",
                  textOverflow: "ellipsis",
                }}
              >
                {h.name || "unknown"}
              </span>
              <span style={{ fontFamily: "var(--font-mono)", fontSize: 10.5, color: "var(--text-40)" }}>
                {h.duration > 0 ? fmtDur(h.duration) : "—"}
              </span>
              <span
                style={{
                  fontFamily: "var(--font-mono)",
                  fontSize: 10.5,
                  color: "var(--text-40)",
                  minWidth: 60,
                  textAlign: "right",
                }}
              >
                {fmtAgo(h.timestamp)}
              </span>
            </div>
          );
        })}
      </div>
    </div>
  );
}

function TargetCard({ status }: { status: Status | null }) {
  const found = !!status?.window.found;
  const app = status?.window.title || "your app";
  const name = found ? app : `Waiting for "${app}"…`;
  return (
    <div style={{ ...sectionCard, display: "flex", alignItems: "center", gap: 16 }}>
      <div
        style={{
          width: 38,
          height: 38,
          borderRadius: "var(--radius-md)",
          background: "rgba(26,24,22,0.05)",
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          flexShrink: 0,
        }}
      >
        <Monitor size={18} color="var(--text-50)" />
      </div>
      <div style={{ flex: 1, minWidth: 0 }}>
        <div
          style={{ fontFamily: "var(--font-sans)", fontSize: 13.5, fontWeight: 600, color: "var(--color-ink)" }}
        >
          {name}
        </div>
        <div className="t-body-sm" style={{ marginTop: 2 }}>
          Clawmation watches for this window to know where to click.
        </div>
      </div>
      <span
        style={{
          display: "inline-flex",
          alignItems: "center",
          gap: 6,
          height: 24,
          padding: "0 10px",
          borderRadius: "var(--radius-xs)",
          fontFamily: "var(--font-mono)",
          fontSize: 10,
          letterSpacing: "0.06em",
          textTransform: "uppercase",
          whiteSpace: "nowrap",
          flexShrink: 0,
          background: found ? "var(--verdict-allowed-bg)" : "var(--verdict-blocked-bg)",
          color: found ? "var(--verdict-allowed)" : "var(--verdict-blocked)",
        }}
      >
        <span style={{ width: 5, height: 5, borderRadius: "50%", background: "currentColor" }} />
        {found ? "Found" : "Not Found"}
      </span>
    </div>
  );
}

function ActivityCard({
  log,
  modeColor,
  modeAnim,
}: {
  log: LogEntry[];
  modeColor: string;
  modeAnim: string;
}) {
  return (
    <div style={{ ...cardBase, overflow: "hidden" }}>
      <div
        style={{
          padding: "12px 18px",
          display: "flex",
          alignItems: "center",
          gap: 10,
          borderBottom: "1px solid var(--border-faint)",
          background: "rgba(255,255,255,0.35)",
        }}
      >
        <span className="t-label" style={{ margin: 0 }}>
          Activity
        </span>
        <span
          style={{ width: 5, height: 5, borderRadius: "50%", background: modeColor, animation: modeAnim }}
        />
      </div>
      {log.length > 0 ? (
        <div style={{ maxHeight: 380, overflowY: "auto", padding: "6px 4px" }}>
          {log.map((entry, i) => (
            <div
              key={i}
              style={{
                display: "grid",
                gridTemplateColumns: "66px 1fr",
                gap: 12,
                padding: "8px 16px",
                borderBottom: "1px solid var(--border-faint)",
              }}
            >
              <span style={{ fontFamily: "var(--font-mono)", fontSize: 10, color: "var(--text-32)" }}>
                {entry.t}
              </span>
              <span style={{ fontFamily: "var(--font-sans)", fontSize: 12.5, color: "var(--text-68)" }}>
                {entry.msg}
              </span>
            </div>
          ))}
        </div>
      ) : (
        <div style={{ padding: "56px 20px", textAlign: "center" }}>
          <div className="t-body-sm">Nothing here yet — press Record to see live events appear.</div>
        </div>
      )}
    </div>
  );
}

function Dashboard({ status }: { status: Status | null }) {
  const { push } = useToast();
  const [runHistory, setRunHistory] = useState<HistoryEntry[]>([]);
  const [macros, setMacros] = useState<MacroListItem[]>([]);
  const busyRef = useRef(false);

  // Mirror Python's `lastMacroPlayed` (index.html onTick): the "when" is a phrase
  // derived from mode transitions, not a timestamp, and only updates while
  // `status.last_macro` is set. `prevModeRef` holds the prior tick's mode, like
  // the Python handler's `prevMode` snapshot taken before the status is applied.
  const [lastPlayed, setLastPlayed] = useState<{ name: string; when: string } | null>(null);
  const prevModeRef = useRef("idle");
  useEffect(() => {
    const mode = status?.mode ?? "idle";
    const prevMode = prevModeRef.current;
    if (status?.last_macro) {
      setLastPlayed((cur) => ({
        name: status.last_macro,
        when:
          mode === "playing"
            ? "playing now"
            : prevMode === "playing" && mode === "idle"
              ? "just now"
              : (cur && cur.when) || "saved",
      }));
    }
    prevModeRef.current = mode;
  }, [status?.mode, status?.last_macro]);

  useEffect(() => {
    let alive = true;
    const load = async () => {
      try {
        const [rh, ml] = await Promise.all([getRunHistory(30), listMacros()]);
        if (!alive) return;
        setRunHistory(rh);
        setMacros(ml);
      } catch {
        // Backend not ready yet — the next tick retries.
      }
    };
    load();
    const id = setInterval(load, DASH_POLL_MS);
    return () => {
      alive = false;
      clearInterval(id);
    };
  }, []);

  // Play a recent macro with its saved repeat and default speed. There is no
  // local record/play state machine here — mode reflects off the status
  // heartbeat — so the guard reads the shared status, and `busyRef` blocks a
  // double-fire during the in-flight call, mirroring the Python `_busyAction`.
  const play = async (m: MacroListItem) => {
    if ((status?.mode ?? "idle") !== "idle") return;
    if (busyRef.current) return;
    busyRef.current = true;
    try {
      const repeat = repsFor(repeatToIndex(m.loop, m.loop_count));
      const res = await playMacro(m.name, repeat, 1.0);
      if (res && res.ok) {
        setLastPlayed({ name: m.name, when: "playing now" });
        push("info", 'Playing "' + m.name + '"');
      } else {
        push("error", (res && res.error) || "Play failed");
      }
    } catch (e) {
      push("error", "Play failed: " + e);
    } finally {
      busyRef.current = false;
    }
  };

  const mode = dashMode(status?.mode ?? "idle");
  const recentMacros = macros
    .filter((m) => m.last_played > 0)
    .sort((a, b) => b.last_played - a.last_played)
    .slice(0, 5);

  return (
    <Page title="Dashboard." subtitle="Everything Clawmation is doing right now, in one place.">
      <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
        <FactCards status={status} lastPlayed={lastPlayed} />

        <div
          style={{
            display: "grid",
            gridTemplateColumns: "minmax(0, 1.7fr) minmax(0, 1fr)",
            gap: 16,
            alignItems: "start",
          }}
        >
          <ActivityCard log={status?.log ?? []} modeColor={mode.color} modeAnim={mode.anim} />
          <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
            <TargetCard status={status} />
            {recentMacros.length > 0 && <RecentMacrosCard macros={recentMacros} onPlay={play} />}
            {runHistory.length > 0 && <RecentRunsCard runs={runHistory} />}
          </div>
        </div>
      </div>
    </Page>
  );
}

export function View({
  view,
  status,
  onNavigate,
}: {
  view: ViewId;
  status: Status | null;
  onNavigate: (view: ViewId) => void;
}) {
  if (view === "dashboard") {
    return <Dashboard status={status} />;
  }
  if (view === "guide") {
    return <GuideView status={status} />;
  }
  if (view === "ai") {
    return <GuardsView onOpenMacros={() => onNavigate("macros")} />;
  }
  if (view === "macros") {
    return <MacrosView status={status} />;
  }
  if (view === "settings") {
    return <SettingsView />;
  }
  if (view === "chains") {
    return <ChainsView />;
  }
  if (view === "vision") {
    return <VisionView />;
  }
  // `view` is `never` here — every ViewId is routed above.
  const unreachable: never = view;
  return unreachable;
}
