import { Fragment, useCallback, useEffect, useState, type CSSProperties } from "react";
import { ArrowClockwise, CaretDown } from "@phosphor-icons/react";

import {
  checkUpdate,
  getConfig,
  getDataPaths,
  getVersion,
  openDataFolder,
  updateConfig,
  type DataPaths,
} from "./api";
import { useToast } from "./components/Toasts";
import { AddButton, Card, CardHead, Page, Toggle } from "./components/ui";

// Faithful port of the Python UI's Settings screen (index.html "isSettings"
// block). Every label, toast, and hotkey-capture flow mirrors the original
// character for character. Like the other views, this one loads its
// own config/version/data-paths on mount rather than hoisting to app state.

type Backend = "dxgi" | "gdi";
type ListenTarget = "record" | "play" | "stop";

// config.capture_backend speaks the engine's names; the UI speaks friendly ones.
const CONFIG_TO_BACKEND: Record<string, Backend> = { dxcam: "dxgi", mss: "gdi" };
const BACKEND_TO_CONFIG: Record<Backend, string> = { dxgi: "dxcam", gdi: "mss" };

function fmtHotkey(k: string): string {
  return String(k)
    .split("+")
    .map((p) => (p.length === 1 ? p.toUpperCase() : p[0].toUpperCase() + p.slice(1).toLowerCase()))
    .join("+");
}

const rowTitle: CSSProperties = {
  fontFamily: "var(--font-sans)",
  fontSize: 13.5,
  fontWeight: 600,
  color: "var(--color-ink)",
};

const KBD: CSSProperties = {
  fontFamily: "var(--font-mono)",
  fontSize: 11,
  color: "var(--color-ink)",
  border: "1px solid var(--border-default)",
  background: "rgba(255,255,255,0.5)",
  padding: "3px 8px",
  borderRadius: 4,
  justifySelf: "end",
};

// The static shortcut reference — verbatim from the Python template.
const SHORTCUTS: Array<[string, string]> = [
  ["Record / Stop recording", "Ctrl+R"],
  ["Play last macro", "Ctrl+P"],
  ["Stop playback", "Ctrl+S"],
  ["Jump to Dashboard / Macros / Guards / Vision / Chains / Guide / Settings", "Alt+1 … Alt+7"],
  ["Toggle this shortcuts panel", "?"],
  ["Surgical: toggle pixel grid", "G"],
  ["Surgical: zoom in / out", "Scroll"],
  ["Surgical: undo last stroke", "Right-click / Backspace"],
  ["Surgical: confirm strokes", "Enter / Done button"],
  ["Cancel overlay", "Esc"],
];

// The hotkey "Change / Cancel" button — Python's `data-ghost`, whose only styled
// state is a faint hover fill over the strong-bordered transparent base.
function GhostButton({ label, onClick }: { label: string; onClick: () => void }) {
  const [hover, setHover] = useState(false);
  return (
    <button
      onClick={onClick}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      style={{
        height: 28,
        padding: "0 12px",
        borderRadius: "var(--radius-sm)",
        border: "1px solid var(--border-strong)",
        background: hover ? "rgba(26,24,22,0.05)" : "transparent",
        color: "var(--text-78)",
        fontFamily: "var(--font-sans)",
        fontSize: 11.5,
        fontWeight: 600,
        cursor: "pointer",
        whiteSpace: "nowrap",
      }}
    >
      {label}
    </button>
  );
}

function BackendButton({
  selected,
  title,
  desc,
  onClick,
}: {
  selected: boolean;
  title: string;
  desc: string;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      style={{
        display: "flex",
        flexDirection: "column",
        alignItems: "flex-start",
        gap: 3,
        padding: "12px 14px",
        borderRadius: "var(--radius-md)",
        border: `1px solid ${selected ? "var(--color-gold)" : "var(--border-default)"}`,
        background: selected ? "rgba(194,163,112,0.06)" : "var(--surface-card-strong)",
        cursor: "pointer",
        textAlign: "left",
        transition: "border-color .15s, background .15s",
      }}
    >
      <span
        style={{
          fontFamily: "var(--font-sans)",
          fontSize: 13,
          fontWeight: 600,
          color: "var(--color-ink)",
          whiteSpace: "nowrap",
        }}
      >
        {title}
      </span>
      <span className="t-body-sm" style={{ margin: 0 }}>
        {desc}
      </span>
    </button>
  );
}

function DataFolderRow({
  label,
  count,
  dir,
  onOpen,
}: {
  label: string;
  count: number;
  dir: string;
  onOpen: () => void;
}) {
  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        justifyContent: "space-between",
        gap: 12,
        padding: "9px 12px",
        border: "1px solid var(--border-faint)",
        borderRadius: "var(--radius-md)",
        background: "var(--surface-card-strong)",
      }}
    >
      <div style={{ minWidth: 0 }}>
        <div style={{ fontFamily: "var(--font-sans)", fontSize: 12, fontWeight: 600, color: "var(--color-ink)" }}>
          {label}{" "}
          <span style={{ fontFamily: "var(--font-mono)", fontSize: 10, color: "var(--text-40)", fontWeight: 400 }}>
            ({count})
          </span>
        </div>
        <div
          style={{
            fontFamily: "var(--font-mono)",
            fontSize: 10,
            color: "var(--text-40)",
            marginTop: 2,
            whiteSpace: "nowrap",
            overflow: "hidden",
            textOverflow: "ellipsis",
          }}
        >
          {dir}
        </div>
      </div>
      <button
        onClick={onOpen}
        style={{
          height: 26,
          padding: "0 12px",
          borderRadius: "var(--radius-sm)",
          border: "1px solid var(--border-strong)",
          background: "transparent",
          color: "var(--text-78)",
          fontFamily: "var(--font-sans)",
          fontSize: 11,
          fontWeight: 600,
          cursor: "pointer",
          whiteSpace: "nowrap",
          flexShrink: 0,
        }}
      >
        Open
      </button>
    </div>
  );
}

export function SettingsView() {
  const { push: pushToast } = useToast();

  const [captureBackend, setCaptureBackend] = useState<Backend>("dxgi");
  const [alwaysOnTop, setAlwaysOnTop] = useState(false);
  const [humanizeClicks, setHumanizeClicks] = useState(false);
  const [notifySchedule, setNotifySchedule] = useState(true);
  const [notifyComplete, setNotifyComplete] = useState(true);
  const [hotkeys, setHotkeys] = useState({
    record: "Ctrl+Shift+R",
    play: "Ctrl+Shift+P",
    stop: "Ctrl+Shift+S",
  });
  const [listeningFor, setListeningFor] = useState<ListenTarget | null>(null);
  const [appVersion, setAppVersion] = useState("1.0.0");
  const [updateStatus, setUpdateStatus] = useState("You are on the latest version.");
  const [dataPaths, setDataPaths] = useState<DataPaths | null>(null);
  const [showAdvanced, setShowAdvanced] = useState(false);

  // Load persisted config, version, and data paths on mount. Each fetch is
  // best-effort — Python's bootstrap silently keeps defaults if a call fails.
  useEffect(() => {
    let alive = true;
    (async () => {
      try {
        const cfg = await getConfig();
        if (!alive) return;
        setCaptureBackend(CONFIG_TO_BACKEND[cfg.capture_backend] || (cfg.capture_backend as Backend) || "dxgi");
        setAlwaysOnTop(!!cfg.indicator_on_top);
        setHumanizeClicks(!!cfg.humanize_clicks);
        setNotifySchedule(cfg.notify_on_schedule !== false);
        setNotifyComplete(cfg.notify_on_complete !== false);
        setHotkeys({
          record: fmtHotkey(cfg.hotkey_record || "f6"),
          play: fmtHotkey(cfg.hotkey_play || "f4"),
          stop: fmtHotkey(cfg.hotkey_stop || "f3"),
        });
      } catch {
        /* keep defaults */
      }
      try {
        const ver = await getVersion();
        if (alive) setAppVersion(ver.version || "1.0.0");
      } catch {
        /* keep default */
      }
      try {
        const dp = await getDataPaths();
        if (alive && dp.ok) setDataPaths(dp);
      } catch {
        /* keep null */
      }
    })();
    return () => {
      alive = false;
    };
  }, []);

  // While listening for a hotkey, the next non-modifier keypress becomes it.
  // A window-level listener matches Python; the future app-shortcut handler
  // must bail while `listeningFor` is set to avoid a double-fire on the key.
  useEffect(() => {
    if (!listeningFor) return;
    const which = listeningFor;
    const onKey = async (e: KeyboardEvent) => {
      e.preventDefault();
      let key = e.key;
      if (key === "Control" || key === "Shift" || key === "Alt" || key === "Meta") return;
      key = key.toLowerCase();
      const field =
        which === "record" ? "hotkey_record" : which === "play" ? "hotkey_play" : "hotkey_stop";
      setHotkeys((prev) => ({ ...prev, [which]: fmtHotkey(key) }));
      setListeningFor(null);
      try {
        await updateConfig({ [field]: key });
        pushToast("success", which[0].toUpperCase() + which.slice(1) + " hotkey set to " + fmtHotkey(key));
      } catch {
        pushToast("error", "Failed to save hotkey");
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [listeningFor, pushToast]);

  const setBackend = useCallback(
    async (b: Backend) => {
      setCaptureBackend(b);
      try {
        await updateConfig({ capture_backend: BACKEND_TO_CONFIG[b] || b });
        pushToast("success", "Capture backend saved");
      } catch {
        pushToast("error", "Failed to save backend");
      }
    },
    [pushToast],
  );

  const toggleAlwaysOnTop = useCallback(async () => {
    const next = !alwaysOnTop;
    setAlwaysOnTop(next);
    try {
      await updateConfig({ indicator_on_top: next });
    } catch {
      /* Python ignores save failures on this toggle */
    }
  }, [alwaysOnTop]);

  const toggleHumanize = useCallback(async () => {
    const next = !humanizeClicks;
    setHumanizeClicks(next);
    pushToast(
      "info",
      next
        ? "Humanized clicks ON — autonomous clicks curve naturally"
        : "Humanized clicks OFF — exact teleport clicks",
    );
    try {
      await updateConfig({ humanize_clicks: next });
    } catch {
      /* toast already reflects the optimistic toggle */
    }
  }, [humanizeClicks, pushToast]);

  const toggleNotifySchedule = useCallback(async () => {
    const next = !notifySchedule;
    setNotifySchedule(next);
    try {
      await updateConfig({ notify_on_schedule: next });
    } catch {
      /* Python ignores save failures on this toggle */
    }
  }, [notifySchedule]);

  const toggleNotifyComplete = useCallback(async () => {
    const next = !notifyComplete;
    setNotifyComplete(next);
    try {
      await updateConfig({ notify_on_complete: next });
    } catch {
      /* Python ignores save failures on this toggle */
    }
  }, [notifyComplete]);

  const onListen = useCallback((which: ListenTarget) => {
    setListeningFor((prev) => (prev === which ? null : which));
  }, []);

  const onCheckUpdate = useCallback(async () => {
    setUpdateStatus("Checking for updates...");
    try {
      const res = await checkUpdate();
      if (res.update_available) {
        setUpdateStatus("Update available: v" + res.latest + " (you have v" + res.current + ")");
        pushToast("info", "Update available: v" + res.latest);
      } else {
        setUpdateStatus("You are on the latest version.");
        pushToast("success", "Already up to date");
      }
    } catch {
      setUpdateStatus("Could not check for updates.");
      pushToast("error", "Update check failed");
    }
  }, [pushToast]);

  const openFolder = useCallback(
    async (kind: string) => {
      try {
        const res = await openDataFolder(kind);
        if (!res.ok) pushToast("error", res.error || "Could not open folder");
      } catch (e) {
        pushToast("error", "Could not open folder: " + e);
      }
    },
    [pushToast],
  );

  const macroCount = (dataPaths && dataPaths.macro_count) || 0;
  const templateCount = (dataPaths && dataPaths.template_count) || 0;
  const snapshotCount = (dataPaths && dataPaths.snapshot_count) || 0;
  const macrosDir = (dataPaths && dataPaths.macros_dir) || "—";
  const templatesDir = (dataPaths && dataPaths.templates_dir) || "—";
  const snapshotsDir = (dataPaths && dataPaths.snapshots_dir) || "—";

  const hotkeyRow = (label: string, which: ListenTarget, last?: boolean) => (
    <div
      style={{
        display: "grid",
        gridTemplateColumns: "120px 1fr auto",
        gap: 14,
        alignItems: "center",
        padding: "10px 0",
        borderBottom: last ? "none" : "1px solid var(--border-faint)",
      }}
    >
      <span style={{ fontFamily: "var(--font-sans)", fontSize: 12.5, fontWeight: 600, color: "var(--color-ink)" }}>
        {label}
      </span>
      <span
        style={{
          fontFamily: "var(--font-mono)",
          fontSize: 12,
          color: listeningFor === which ? "var(--color-gold)" : "var(--color-ink)",
        }}
      >
        {listeningFor === which ? "Press a key…" : hotkeys[which]}
      </span>
      <GhostButton label={listeningFor === which ? "Cancel" : "Change"} onClick={() => onListen(which)} />
    </div>
  );

  // A single boolean setting inside a grouped card: title + help on the left,
  // the shared Toggle on the right, hairline-separated from the next row.
  const toggleRow = (
    title: string,
    desc: string,
    on: boolean,
    onToggle: () => void,
    last?: boolean,
  ) => (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        justifyContent: "space-between",
        gap: 16,
        padding: "13px 0",
        borderBottom: last ? "none" : "1px solid var(--border-faint)",
      }}
    >
      <div style={{ minWidth: 0 }}>
        <div style={rowTitle}>{title}</div>
        <div className="t-body-sm" style={{ marginTop: 2 }}>
          {desc}
        </div>
      </div>
      <Toggle on={on} onClick={onToggle} />
    </div>
  );

  return (
    <Page title="Make Clawmation yours.">
      <div
        style={{
          display: "grid",
          gridTemplateColumns: "minmax(0, 1fr) minmax(0, 1fr)",
          gap: 16,
          alignItems: "start",
        }}
      >
        {/* Left column — preference toggles */}
        <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
          <Card>
            <CardHead label="Behavior" />
            {toggleRow(
              "Always on top",
              "Keep Clawmation's window above everything else.",
              alwaysOnTop,
              toggleAlwaysOnTop,
            )}
            {toggleRow(
              "Humanize autonomous clicks",
              "Guard & vision clicks travel a natural curved path instead of teleporting. Recorded-macro playback is always exact.",
              humanizeClicks,
              toggleHumanize,
              true,
            )}
          </Card>

          <Card>
            <CardHead label="Notifications" />
            {toggleRow(
              "Notify on schedule",
              "Show a balloon when a scheduled macro or chain starts.",
              notifySchedule,
              toggleNotifySchedule,
            )}
            {toggleRow(
              "Notify on complete",
              "Show a balloon when macro playback finishes.",
              notifyComplete,
              toggleNotifyComplete,
              true,
            )}
          </Card>
        </div>

        {/* Right column — keyboard: editable hotkeys + the static reference */}
        <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
          <Card>
            <CardHead label="Hotkeys" />
            {hotkeyRow("Record", "record")}
            {hotkeyRow("Play", "play")}
            {hotkeyRow("Stop", "stop", true)}
          </Card>

          <Card>
            <CardHead label="Keyboard shortcuts" />
            <div style={{ display: "grid", gridTemplateColumns: "1fr auto", gap: "8px 16px" }}>
              {SHORTCUTS.map(([label, keys]) => (
                <Fragment key={label}>
                  <span className="t-body-sm" style={{ margin: 0, alignSelf: "center" }}>
                    {label}
                  </span>
                  <span style={KBD}>{keys}</span>
                </Fragment>
              ))}
            </div>
          </Card>
        </div>
      </div>

      {/* Data + about — full-width bands below the two-column preference grid */}
      <div style={{ display: "flex", flexDirection: "column", gap: 16, marginTop: 16 }}>
        <Card>
          <CardHead
            label="Your data"
            desc="Macros, templates, and snapshots live in plain files you can back up or move."
          />
          <div
            style={{
              display: "grid",
              gridTemplateColumns: "repeat(auto-fit, minmax(280px, 1fr))",
              gap: 8,
            }}
          >
            <DataFolderRow label="Macros" count={macroCount} dir={macrosDir} onOpen={() => openFolder("macros")} />
            <DataFolderRow
              label="Templates"
              count={templateCount}
              dir={templatesDir}
              onOpen={() => openFolder("templates")}
            />
            <DataFolderRow
              label="Snapshots"
              count={snapshotCount}
              dir={snapshotsDir}
              onOpen={() => openFolder("snapshots")}
            />
          </div>
        </Card>

        <Card
          style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 16 }}
        >
          <div>
            <div style={rowTitle}>
              Clawmation{" "}
              <span style={{ fontFamily: "var(--font-mono)", fontSize: 12, color: "var(--text-40)", fontWeight: 400 }}>
                v{appVersion}
              </span>
            </div>
            <div className="t-body-sm" style={{ marginTop: 2 }}>
              {updateStatus}
            </div>
          </div>
          <AddButton icon={<ArrowClockwise size={12} />} onClick={onCheckUpdate}>
            Check for updates
          </AddButton>
        </Card>

        {/* Advanced — the capture backend, folded away by default. Screen capture
            "just works" on the recommended default; this only matters when it
            doesn't, so it stays out of the main flow until asked for. */}
        <Card>
          <button
            type="button"
            onClick={() => setShowAdvanced((v) => !v)}
            style={{
              display: "flex",
              alignItems: "center",
              justifyContent: "space-between",
              gap: 16,
              width: "100%",
              padding: 0,
              border: "none",
              background: "transparent",
              cursor: "pointer",
              textAlign: "left",
            }}
          >
            <div style={{ minWidth: 0 }}>
              <div style={rowTitle}>Advanced</div>
              <div className="t-body-sm" style={{ marginTop: 2 }}>
                Most people never need this. Open it only if Clawmation is having trouble seeing the screen.
              </div>
            </div>
            <CaretDown
              size={16}
              color="var(--text-50)"
              style={{
                flexShrink: 0,
                transition: "transform .18s",
                transform: showAdvanced ? "rotate(180deg)" : "none",
              }}
            />
          </button>
          {showAdvanced && (
            <div style={{ marginTop: 16 }}>
              <div style={rowTitle}>How Clawmation reads the screen</div>
              <div className="t-body-sm" style={{ marginTop: 2, marginBottom: 10 }}>
                Leave this on the recommended option unless detection is unreliable.
              </div>
              <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
                <BackendButton
                  selected={captureBackend === "dxgi"}
                  title="Recommended"
                  desc="Fastest, and best for games and smooth detection."
                  onClick={() => setBackend("dxgi")}
                />
                <BackendButton
                  selected={captureBackend === "gdi"}
                  title="Compatibility mode"
                  desc="Slower, but works on almost anything. Use this if the recommended option fails."
                  onClick={() => setBackend("gdi")}
                />
              </div>
            </div>
          )}
        </Card>
      </div>
    </Page>
  );
}
