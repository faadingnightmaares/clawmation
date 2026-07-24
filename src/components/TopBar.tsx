import type { CSSProperties } from "react";
import { ArrowClockwise, Stop, Tray } from "@phosphor-icons/react";

import { emergencyStop, stopRecord, windowMinimizeToTray } from "../api";
import type { Status } from "../api";
import { useToast } from "./Toasts";

interface TopBarProps {
  status: Status | null;
}

const barStyle: CSSProperties = {
  height: 54,
  flexShrink: 0,
  display: "flex",
  alignItems: "center",
  gap: 12,
  padding: "0 22px",
  borderBottom: "1px solid var(--border-faint)",
  background: "rgba(250,248,245,0.7)",
  position: "relative",
  zIndex: 4,
};

interface ModeMeta {
  label: string;
  color: string;
  border: string;
  bg: string;
}

function modeMeta(mode: string): ModeMeta {
  switch (mode) {
    case "recording":
      return {
        label: "Recording",
        color: "var(--verdict-blocked)",
        border: "rgba(125,42,30,0.25)",
        bg: "var(--verdict-blocked-bg)",
      };
    case "paused":
      return {
        label: "Paused",
        color: "var(--color-gold)",
        border: "rgba(194,163,112,0.25)",
        bg: "rgba(194,163,112,0.08)",
      };
    case "playing":
      return {
        label: "Playing",
        color: "var(--verdict-allowed)",
        border: "rgba(26,102,64,0.25)",
        bg: "var(--verdict-allowed-bg)",
      };
    default:
      return {
        label: "Running",
        color: "var(--text-32)",
        border: "var(--border-default)",
        bg: "rgba(255,255,255,0.4)",
      };
  }
}

function repCounterLabel(iteration: number, totalReps: number): string {
  return totalReps > 0
    ? `Rep ${iteration} / ${totalReps}`
    : `Rep ${iteration} / ∞`;
}

// Shared chrome for the bar's minimize + Stop controls: a transparent 32px
// ghost button that takes its per-button color/opacity through `extra`.
function ghostButtonStyle(extra: CSSProperties): CSSProperties {
  return {
    display: "inline-flex",
    alignItems: "center",
    justifyContent: "center",
    height: 32,
    borderRadius: "var(--radius-md)",
    border: "1px solid var(--border-strong)",
    background: "transparent",
    cursor: "pointer",
    flexShrink: 0,
    ...extra,
  };
}

export function TopBar({ status }: TopBarProps) {
  const { push: pushToast } = useToast();
  const mode = status?.mode ?? "idle";
  const isIdle = mode === "idle";
  const meta = modeMeta(mode);
  const showRepCounter = mode === "playing" && (status?.play_iteration ?? 0) > 0;

  // `Api.globalStop`. While recording, Stop routes through the same stop_record
  // the record button uses, so the macro is saved and toasted identically; it
  // cannot arm MacrosView's inline-rename on the new macro — `recordNamePromptId`
  // is that view's local state, so the record button stays the path that opens
  // it. Otherwise Stop is the global emergency stop.
  const onStop = async () => {
    try {
      if (mode === "recording") {
        const res = await stopRecord();
        if (res && res.ok) {
          pushToast("success", "Saved " + res.name + " — " + res.events + " events");
        } else {
          pushToast("error", (res && res.error) || "Stop failed");
        }
        return;
      }
      const res = await emergencyStop();
      pushToast("info", "Stopped.");
      if (res && !res.ok) console.warn(res);
    } catch {
      pushToast("error", "Stop failed");
    }
  };

  // `Api.minimizeToTray`.
  const onMinimize = async () => {
    try {
      await windowMinimizeToTray();
      pushToast("info", "Minimized to tray — click the tray icon to restore.");
    } catch {
      pushToast("error", "Minimize failed");
    }
  };

  return (
    <div role="banner" style={barStyle}>
      {isIdle ? (
        <div style={{ display: "flex", alignItems: "center", gap: 8, flexShrink: 0 }}>
          <span style={{ width: 7, height: 7, borderRadius: "50%", background: "var(--text-25)" }} />
          <span style={{ fontFamily: "var(--font-sans)", fontSize: 12, color: "var(--text-40)" }}>
            Ready
          </span>
        </div>
      ) : (
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: 8,
            padding: "5px 12px",
            border: `1px solid ${meta.border}`,
            borderRadius: "var(--radius-sm)",
            background: meta.bg,
            flexShrink: 0,
            whiteSpace: "nowrap",
          }}
        >
          <span
            style={{
              width: 7,
              height: 7,
              borderRadius: "50%",
              background: meta.color,
              animation: "vecPulse 1.4s ease-in-out infinite",
              flexShrink: 0,
            }}
          />
          <span
            style={{
              fontFamily: "var(--font-mono)",
              fontSize: 11,
              fontWeight: 600,
              letterSpacing: "0.04em",
              textTransform: "uppercase",
              color: meta.color,
              whiteSpace: "nowrap",
            }}
          >
            {meta.label}
          </span>
        </div>
      )}

      {showRepCounter && status && (
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: 6,
            padding: "5px 10px",
            border: "1px solid var(--gold-border)",
            borderRadius: "var(--radius-sm)",
            background: "rgba(194,163,112,0.06)",
            flexShrink: 0,
            whiteSpace: "nowrap",
          }}
        >
          <ArrowClockwise size={14} color="var(--color-gold)" />
          <span
            style={{
              fontFamily: "var(--font-mono)",
              fontSize: 11,
              fontWeight: 600,
              color: "var(--color-gold)",
              whiteSpace: "nowrap",
            }}
          >
            {repCounterLabel(status.play_iteration, status.play_total_reps)}
          </span>
        </div>
      )}

      <div style={{ flex: 1 }} />

      <button
        type="button"
        title="Minimize to system tray"
        onClick={onMinimize}
        style={ghostButtonStyle({ width: 32, color: "var(--text-78)" })}
      >
        <Tray size={14} />
      </button>
      <button
        type="button"
        onClick={onStop}
        disabled={isIdle}
        style={ghostButtonStyle({
          gap: 7,
          padding: "0 14px",
          color: isIdle ? "var(--text-32)" : "var(--verdict-blocked)",
          fontFamily: "var(--font-sans)",
          fontSize: 12,
          fontWeight: 600,
          opacity: isIdle ? 0.5 : 1,
        })}
      >
        <Stop size={12} />
        Stop
      </button>

      <div
        style={{ width: 1, height: 22, background: "var(--border-default)", flexShrink: 0, margin: "0 4px" }}
      />
      <div style={{ display: "flex", alignItems: "center", gap: 8, flexShrink: 0 }}>
        <img
          src="/Clawmation.svg"
          alt=""
          aria-hidden="true"
          style={{ height: 26, width: "auto", display: "block" }}
        />
        <span
          style={{
            fontFamily: "var(--font-serif)",
            fontSize: 15,
            fontWeight: 500,
            letterSpacing: "0.005em",
            color: "var(--color-ink)",
            whiteSpace: "nowrap",
          }}
        >
          Clawmation
        </span>
      </div>
    </div>
  );
}
