import { useCallback, useEffect, useState, type CSSProperties } from "react";
import {
  CaretDown,
  CaretUp,
  Clock,
  Copy,
  DownloadSimple,
  FlowArrow,
  PencilSimple,
  Play,
  Plus,
  Stop,
  Trash,
  UploadSimple,
  Warning,
  X,
} from "@phosphor-icons/react";

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
} from "./api";
import { fmtDur } from "./format";
import { useToast } from "./components/Toasts";
import {
  AddButton,
  Card,
  CardHead,
  GhostButton,
  IconButton,
  Page,
  PrimaryButton,
} from "./components/ui";

// The Chains view sequences several macros into one ordered run with delays and
// repeats. Faithful port of the Python UI's Chains screen — every toast and
// validation string is preserved character for character.

// The 500ms heartbeat, matching `useStatus` and the Python UI's `setInterval`.
const POLL_MS = 500;

/** The inline editor's working copy of a chain, plus its own dropdown selection. */
interface ChainEditorState {
  id: string | null;
  name: string;
  macro_names: string[];
  delay_between: number;
  repeat: number;
  selectedMacro: string;
}

/** A listed chain enriched with the validation + duration Python computes per row. */
interface EnrichedChain extends Chain {
  missingMacros: string[];
  duration: number | null;
}

const input: CSSProperties = {
  width: "100%",
  boxSizing: "border-box",
  height: 32,
  border: "1px solid var(--border-default)",
  borderRadius: "var(--radius-sm)",
  padding: "0 9px",
  fontFamily: "var(--font-sans)",
  fontSize: 12,
  color: "var(--color-ink)",
  background: "var(--surface-card)",
};

const fieldLabel: CSSProperties = {
  fontFamily: "var(--font-mono)",
  fontSize: 9.5,
  letterSpacing: "0.06em",
  textTransform: "uppercase",
  color: "var(--text-40)",
  marginBottom: 4,
};

/** The empty-state call to action — the one button Python gives a hover style. */
function BuildFirstChainButton({ onClick }: { onClick: () => void }) {
  const [hover, setHover] = useState(false);
  return (
    <button
      onClick={onClick}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      style={{
        marginTop: 26,
        display: "inline-flex",
        alignItems: "center",
        gap: 8,
        height: 40,
        padding: "0 22px",
        borderRadius: "var(--radius-md)",
        border: "none",
        background: hover ? "#2b2825" : "var(--color-ink)",
        color: "var(--color-gold)",
        fontFamily: "var(--font-sans)",
        fontSize: 13,
        fontWeight: 600,
        cursor: "pointer",
        boxShadow: hover ? "0 2px 12px rgba(26,24,22,0.20)" : "none",
      }}
    >
      <Plus size={15} />
      Build your first chain
    </button>
  );
}

export function ChainsView() {
  const { push: pushToast } = useToast();
  const [chains, setChains] = useState<EnrichedChain[]>([]);
  const [macros, setMacros] = useState<MacroListItem[]>([]);
  const [editor, setEditor] = useState<ChainEditorState | null>(null);
  const [runningChain, setRunningChain] = useState<RunningChain | null>(null);

  // Validate each chain and calculate its total duration — Python's loadChains.
  const loadChains = useCallback(async () => {
    try {
      const list = await listChains();
      const validated = await Promise.all(
        (list || []).map(async (c): Promise<EnrichedChain> => {
          try {
            const [v, d] = await Promise.all([
              validateChain(c.id as string),
              getChainDuration(c.id as string),
            ]);
            return {
              ...c,
              missingMacros: (v && v.missing) || [],
              duration: d && d.ok ? d.duration ?? null : null,
            };
          } catch {
            return { ...c, missingMacros: [], duration: null };
          }
        }),
      );
      setChains(validated);
    } catch (e) {
      console.error("list_chains", e);
    }
  }, []);

  // On mount: load chains, and load macros for the editor's picker + durations.
  useEffect(() => {
    let alive = true;
    loadChains();
    (async () => {
      try {
        const list = await listMacros();
        if (alive) setMacros(list);
      } catch {
        // Best-effort — the picker just starts empty until macros arrive.
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
        // Transient poll failure — retry next tick.
      }
    };
    tick();
    const id = setInterval(tick, POLL_MS);
    return () => {
      alive = false;
      clearInterval(id);
    };
  }, []);

  const macroDuration = (name: string): number | undefined =>
    macros.find((m) => m.name === name)?.duration;

  // ─── Editor handlers (mirroring the Python methods 1:1) ───
  const openNewChain = () =>
    setEditor({ id: null, name: "", macro_names: [], delay_between: 1.0, repeat: 1, selectedMacro: "" });

  const openEditChain = (c: EnrichedChain) =>
    setEditor({
      id: c.id ?? null,
      name: c.name ?? "",
      macro_names: [...(c.macro_names || [])],
      delay_between: c.delay_between ?? 0,
      repeat: c.repeat ?? 0,
      selectedMacro: "",
    });

  const closeEditor = () => setEditor(null);

  const patchEditor = (patch: Partial<ChainEditorState>) =>
    setEditor((e) => (e ? { ...e, ...patch } : e));

  const addMacroToEditor = (name: string) => {
    if (!name) return;
    setEditor((e) => (e ? { ...e, macro_names: [...e.macro_names, name] } : e));
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
      pushToast("error", "Chain needs a name");
      return;
    }
    if (!ed.macro_names.length) {
      pushToast("error", "Add at least one macro to the chain");
      return;
    }
    try {
      const res = ed.id
        ? await updateChain(ed.id, ed.name.trim(), ed.macro_names, ed.delay_between, ed.repeat)
        : await addChain(ed.name.trim(), ed.macro_names, ed.delay_between, ed.repeat);
      if (res && res.ok) {
        pushToast("success", 'Chain "' + ed.name.trim() + '" saved');
        setEditor(null);
        await loadChains();
      } else {
        pushToast("error", (res && res.error) || "Chain save failed");
      }
    } catch (e) {
      pushToast("error", "Chain save failed: " + e);
    }
  };

  const handleRun = async (chainId: string) => {
    try {
      const res = await runChain(chainId);
      if (res && (res as { ok?: boolean }).ok) {
        pushToast("success", "Chain started");
      } else {
        pushToast("error", "Chain could not start (busy or not found)");
      }
    } catch (e) {
      pushToast("error", "Chain run failed: " + e);
    }
  };

  const handleDelete = async (chainId: string) => {
    try {
      await removeChain(chainId);
      pushToast("info", "Chain removed");
      await loadChains();
    } catch {
      pushToast("error", "Chain removal failed");
    }
  };

  const handleDuplicate = async (chainId: string) => {
    try {
      const res = await duplicateChain(chainId);
      if (res && res.ok) {
        pushToast("success", 'Duplicated chain "' + res.name + '"');
        await loadChains();
      } else {
        pushToast("error", (res && res.error) || "Duplicate failed");
      }
    } catch (e) {
      pushToast("error", "Chain duplicate failed: " + e);
    }
  };

  const handleExport = async (chainId: string) => {
    try {
      const res = await exportChain(chainId);
      if (res && res.ok) {
        pushToast("success", "Chain exported");
      } else if (res && res.error !== "cancelled") {
        pushToast("error", res.error || "Export failed");
      }
    } catch (e) {
      pushToast("error", "Chain export failed: " + e);
    }
  };

  const handleImport = async () => {
    try {
      const res = await importChain();
      if (res && res.ok) {
        pushToast("success", 'Imported chain "' + res.name + '"');
        await loadChains();
      } else if (res && res.error !== "cancelled") {
        pushToast("error", res.error || "Import failed");
      }
    } catch (e) {
      pushToast("error", "Chain import failed: " + e);
    }
  };

  const handleSchedule = async (chainId: string) => {
    const kind = window.prompt("Schedule type (interval or once):", "interval");
    if (!kind || (kind !== "interval" && kind !== "once")) return;
    let intervalMin = 30;
    let atTime = "";
    if (kind === "interval") {
      const mins = window.prompt("Interval in minutes:", "30");
      intervalMin = parseFloat(mins ?? "") || 30;
    } else {
      atTime = window.prompt("Time to run (HH:MM, 24h format):", "09:00") ?? "";
      if (!atTime) return;
    }
    try {
      const res = await scheduleChain(chainId, kind, intervalMin, atTime);
      if (res && res.ok) {
        pushToast("success", "Chain scheduled");
      } else {
        pushToast("error", (res && res.error) || "Failed to schedule chain");
      }
    } catch (e) {
      pushToast("error", "Failed to schedule chain: " + e);
    }
  };

  const handleStop = async () => {
    try {
      const res = await stopChain();
      if (res && res.ok) {
        pushToast("info", "Chain stop requested");
        setRunningChain(null);
      }
    } catch (e) {
      pushToast("error", "Failed to stop chain: " + e);
    }
  };

  const chainRunning = !!(runningChain && runningChain.running);
  const runStep = (runningChain?.current_index ?? 0) + 1;
  const runTotal = runningChain?.total_macros ?? 0;
  const runHasIterations = !!(runningChain?.total_iterations && runningChain.total_iterations > 1);
  const runProgressPct = runTotal ? Math.round((runStep / runTotal) * 100) : 0;

  // Editor-derived values (Python's `ce` view-model).
  const editorTotalDur = (() => {
    if (!editor) return "";
    const names = editor.macro_names;
    const total = names.reduce((sum, name) => sum + (macroDuration(name) || 0), 0);
    const delayTotal = (names.length > 1 ? names.length - 1 : 0) * (editor.delay_between || 0);
    return names.length ? fmtDur(total + delayTotal) : "";
  })();
  const availableMacros = editor
    ? macros.map((m) => m.name).filter((n) => !editor.macro_names.includes(n))
    : [];

  return (
    <Page
      title="Macro Chains."
      subtitle="Run multiple macros in sequence. Build a chain, add macros in order, and run them one after another automatically."
      action={
        <>
          <PrimaryButton onClick={openNewChain} icon={<Plus size={14} />}>
            New Chain
          </PrimaryButton>
          <GhostButton onClick={handleImport} icon={<UploadSimple size={14} />}>
            Import
          </GhostButton>
        </>
      }
    >

      {chainRunning && (
        <Card
          pad="16px 18px"
          style={{
            marginBottom: 18,
            background: "rgba(95,174,95,0.08)",
            border: "1px solid var(--verdict-allowed)",
          }}
        >
          <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
            <span
              style={{
                width: 8,
                height: 8,
                borderRadius: "50%",
                background: "var(--verdict-allowed)",
                animation: "vecPulse 1.4s ease-in-out infinite",
                flexShrink: 0,
              }}
            />
            <div style={{ flex: 1, minWidth: 0 }}>
              <div
                style={{
                  fontFamily: "var(--font-sans)",
                  fontSize: 13,
                  fontWeight: 600,
                  color: "var(--color-ink)",
                }}
              >
                Running: {runningChain?.name || ""}
              </div>
              <div
                style={{
                  fontFamily: "var(--font-mono)",
                  fontSize: 10.5,
                  color: "var(--text-50)",
                  marginTop: 2,
                }}
              >
                Step {runStep} / {runTotal} · {runningChain?.current_macro || ""}
                {runHasIterations && (
                  <> · round {runningChain?.iteration || 1} / {runningChain?.total_iterations || 1}</>
                )}
              </div>
            </div>
            <button
              onClick={handleStop}
              style={{
                display: "inline-flex",
                alignItems: "center",
                gap: 6,
                height: 30,
                padding: "0 14px",
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
              <Stop size={12} />
              Stop
            </button>
          </div>
          <div
            style={{
              marginTop: 10,
              height: 4,
              borderRadius: 2,
              background: "rgba(26,24,22,0.08)",
              overflow: "hidden",
            }}
          >
            <div
              style={{
                height: "100%",
                width: runProgressPct + "%",
                background: "var(--verdict-allowed)",
                borderRadius: 2,
                transition: "width .3s ease",
              }}
            />
          </div>
        </Card>
      )}

      {editor && (
        <Card
          pad={20}
          style={{
            marginBottom: 18,
            border: "1px solid var(--color-gold)",
            background: "var(--surface-card-strong)",
          }}
        >
          <div
            style={{
              display: "flex",
              alignItems: "center",
              justifyContent: "space-between",
              marginBottom: 14,
            }}
          >
            <span
              style={{
                fontFamily: "var(--font-mono)",
                fontSize: 11,
                letterSpacing: "0.08em",
                textTransform: "uppercase",
                color: "var(--color-gold)",
              }}
            >
              {editor.id ? "Edit Chain" : "New Chain"}
            </span>
            <button
              onClick={closeEditor}
              style={{
                border: "none",
                background: "transparent",
                color: "var(--text-40)",
                cursor: "pointer",
                display: "flex",
                alignItems: "center",
              }}
            >
              <X size={14} />
            </button>
          </div>

          <div
            style={{
              display: "grid",
              gridTemplateColumns: "1fr 1fr 1fr",
              gap: 12,
              marginBottom: 14,
            }}
          >
            <div>
              <div style={fieldLabel}>Chain name</div>
              <input
                value={editor.name}
                onChange={(e) => patchEditor({ name: e.target.value })}
                placeholder="e.g. Morning routine"
                style={input}
              />
            </div>
            <div>
              <div style={fieldLabel}>Delay between (s)</div>
              <input
                type="number"
                min="0"
                step="0.5"
                value={editor.delay_between}
                onChange={(e) => patchEditor({ delay_between: parseFloat(e.target.value) || 0 })}
                style={input}
              />
            </div>
            <div>
              <div style={fieldLabel}>Repeat (0 = ∞)</div>
              <input
                type="number"
                min="0"
                value={editor.repeat}
                onChange={(e) => patchEditor({ repeat: parseInt(e.target.value, 10) || 0 })}
                style={input}
              />
            </div>
          </div>

          <div style={{ marginBottom: 14 }}>
            <div style={{ ...fieldLabel, marginBottom: 6 }}>Macros in order</div>
            {editor.macro_names.length > 0 ? (
              <>
                <div style={{ display: "flex", flexDirection: "column", gap: 6, marginBottom: 10 }}>
                  {editor.macro_names.map((name, idx) => {
                    const durLabel = (() => {
                      const d = macroDuration(name);
                      return d !== undefined ? fmtDur(d) : "";
                    })();
                    const upDisabled = idx === 0;
                    const downDisabled = idx === editor.macro_names.length - 1;
                    return (
                      <div
                        key={name + idx}
                        style={{
                          display: "flex",
                          alignItems: "center",
                          gap: 8,
                          padding: "8px 10px",
                          border: "1px solid var(--border-default)",
                          borderRadius: "var(--radius-sm)",
                          background: "var(--surface-card)",
                        }}
                      >
                        <span
                          style={{
                            fontFamily: "var(--font-mono)",
                            fontSize: 11,
                            color: "var(--text-40)",
                            width: 20,
                          }}
                        >
                          {idx + 1}
                        </span>
                        <span
                          style={{
                            flex: 1,
                            fontFamily: "var(--font-sans)",
                            fontSize: 12,
                            fontWeight: 500,
                            color: "var(--color-ink)",
                          }}
                        >
                          {name}
                        </span>
                        {durLabel && (
                          <span
                            style={{
                              fontFamily: "var(--font-mono)",
                              fontSize: 10.5,
                              color: "var(--text-40)",
                            }}
                          >
                            {durLabel}
                          </span>
                        )}
                        <button
                          onClick={() => moveMacroInEditor(idx, -1)}
                          disabled={upDisabled}
                          title="Move up"
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
                            opacity: upDisabled ? 0.3 : 1,
                          }}
                        >
                          <CaretUp size={13} />
                        </button>
                        <button
                          onClick={() => moveMacroInEditor(idx, 1)}
                          disabled={downDisabled}
                          title="Move down"
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
                            opacity: downDisabled ? 0.3 : 1,
                          }}
                        >
                          <CaretDown size={13} />
                        </button>
                        <button
                          onClick={() => removeMacroFromEditor(idx)}
                          title="Remove"
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
                          <X size={12} />
                        </button>
                      </div>
                    );
                  })}
                </div>
                {editorTotalDur && (
                  <div
                    style={{
                      display: "flex",
                      alignItems: "center",
                      justifyContent: "flex-end",
                      gap: 6,
                      marginBottom: 10,
                    }}
                  >
                    <span
                      style={{
                        fontFamily: "var(--font-mono)",
                        fontSize: 10,
                        letterSpacing: "0.04em",
                        textTransform: "uppercase",
                        color: "var(--text-40)",
                      }}
                    >
                      Total run time
                    </span>
                    <span
                      style={{
                        fontFamily: "var(--font-mono)",
                        fontSize: 11.5,
                        fontWeight: 600,
                        color: "var(--color-gold)",
                      }}
                    >
                      {editorTotalDur}
                    </span>
                  </div>
                )}
              </>
            ) : (
              <div
                style={{
                  border: "1px dashed var(--border-default)",
                  borderRadius: "var(--radius-sm)",
                  padding: 14,
                  textAlign: "center",
                  marginBottom: 10,
                }}
              >
                <span
                  style={{ fontFamily: "var(--font-sans)", fontSize: 11.5, color: "var(--text-40)" }}
                >
                  No macros added yet. Pick one below.
                </span>
              </div>
            )}
            <div style={{ display: "flex", gap: 8 }}>
              <select
                value={editor.selectedMacro}
                onChange={(e) => patchEditor({ selectedMacro: e.target.value })}
                style={{
                  flex: 1,
                  height: 32,
                  border: "1px solid var(--border-default)",
                  borderRadius: "var(--radius-sm)",
                  padding: "0 9px",
                  fontFamily: "var(--font-sans)",
                  fontSize: 12,
                  color: "var(--color-ink)",
                  background: "var(--surface-card)",
                  cursor: "pointer",
                }}
              >
                <option value="">Select a macro to add…</option>
                {availableMacros.map((am) => (
                  <option key={am} value={am}>
                    {am}
                  </option>
                ))}
              </select>
              <AddButton
                onClick={() => {
                  const sel = editor.selectedMacro;
                  if (sel) {
                    addMacroToEditor(sel);
                    patchEditor({ selectedMacro: "" });
                  }
                }}
                icon={<Plus size={12} />}
              >
                Add
              </AddButton>
            </div>
          </div>

          <div style={{ display: "flex", justifyContent: "flex-end", gap: 8 }}>
            <GhostButton onClick={closeEditor}>Cancel</GhostButton>
            <PrimaryButton onClick={saveEditor}>Save Chain</PrimaryButton>
          </div>
        </Card>
      )}

      {chains.length > 0 && (
        <div
          style={{
            display: "grid",
            gridTemplateColumns: "repeat(2, minmax(0, 1fr))",
            gap: 16,
            alignItems: "start",
          }}
        >
          {chains.map((chain) => {
            const macroNames = chain.macro_names || [];
            const repeatLabel = chain.repeat === 0 ? "∞" : String(chain.repeat);
            const cid = chain.id as string;
            return (
              <Card key={cid} pad={20}>
                <CardHead
                  title={chain.name}
                  action={
                    <div style={{ display: "flex", gap: 8, flexShrink: 0 }}>
                      <button
                        onClick={() => handleRun(cid)}
                        style={{
                          display: "inline-flex",
                          alignItems: "center",
                          gap: 6,
                          height: 28,
                          padding: "0 12px",
                          borderRadius: "var(--radius-sm)",
                          border: "1px solid var(--verdict-allowed)",
                          background: "transparent",
                          color: "var(--verdict-allowed)",
                          fontFamily: "var(--font-sans)",
                          fontSize: 11.5,
                          fontWeight: 600,
                          cursor: "pointer",
                        }}
                      >
                        <Play size={12} />
                        Run
                      </button>
                      <IconButton
                        title="Schedule this chain to run automatically"
                        onClick={() => handleSchedule(cid)}
                      >
                        <Clock size={13} />
                      </IconButton>
                      <IconButton title="Edit chain" onClick={() => openEditChain(chain)}>
                        <PencilSimple size={13} />
                      </IconButton>
                      <IconButton title="Duplicate chain" onClick={() => handleDuplicate(cid)}>
                        <Copy size={13} />
                      </IconButton>
                      <IconButton title="Export chain as JSON" onClick={() => handleExport(cid)}>
                        <DownloadSimple size={13} />
                      </IconButton>
                      <IconButton title="Delete chain" onClick={() => handleDelete(cid)}>
                        <Trash size={13} />
                      </IconButton>
                    </div>
                  }
                />
                <div
                  style={{
                    fontFamily: "var(--font-mono)",
                    fontSize: 11,
                    color: "var(--text-50)",
                    marginBottom: 8,
                  }}
                >
                  {macroNames.length} macros · {chain.delay_between}s delay · repeat {repeatLabel}
                  {chain.duration != null && <> · ~{fmtDur(chain.duration)}</>}
                </div>
                {chain.missingMacros.length > 0 && (
                  <div
                    style={{
                      marginBottom: 8,
                      padding: "8px 10px",
                      background: "rgba(220,80,60,0.08)",
                      border: "1px solid rgba(220,80,60,0.3)",
                      borderRadius: "var(--radius-sm)",
                      display: "flex",
                      alignItems: "center",
                      gap: 8,
                    }}
                  >
                    <Warning size={14} color="var(--verdict-blocked)" style={{ flexShrink: 0 }} />
                    <span
                      style={{
                        fontFamily: "var(--font-sans)",
                        fontSize: 11,
                        color: "var(--verdict-blocked)",
                      }}
                    >
                      {chain.missingMacros.length} macro(s) missing — chain will skip them
                    </span>
                  </div>
                )}
                <div style={{ display: "flex", flexWrap: "wrap", gap: 6 }}>
                  {macroNames.map((mname, i) => (
                    <span
                      key={mname + i}
                      style={{
                        display: "inline-block",
                        padding: "3px 8px",
                        borderRadius: "var(--radius-xs)",
                        background: "rgba(194,163,112,0.08)",
                        border: "1px solid var(--gold-border)",
                        fontFamily: "var(--font-mono)",
                        fontSize: 10,
                        color: "var(--color-gold)",
                      }}
                    >
                      {mname}
                    </span>
                  ))}
                </div>
              </Card>
            );
          })}
        </div>
      )}

      {chains.length === 0 && (
        <div
          style={{
            border: "1px dashed var(--border-default)",
            background: "var(--surface-inset)",
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
            <FlowArrow size={26} color="var(--color-gold)" />
          </div>
          <div
            style={{
              fontFamily: "var(--font-serif)",
              fontSize: 22,
              fontWeight: 300,
              color: "var(--color-ink)",
            }}
          >
            No chains yet.
          </div>
          <div className="t-body-sm" style={{ margin: "10px auto 0", maxWidth: "46ch" }}>
            A chain strings several macros together and runs them back-to-back — perfect for a full
            routine you only want to kick off once.
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
            <div
              style={{
                display: "grid",
                gridTemplateColumns: "24px 1fr",
                gap: 12,
                padding: "9px 4px",
                borderBottom: "1px solid var(--border-faint)",
              }}
            >
              <span style={{ fontFamily: "var(--font-mono)", fontSize: 11, color: "var(--color-gold)" }}>
                1
              </span>
              <span className="t-body-sm">Pick the macros you want, in the order they should run.</span>
            </div>
            <div
              style={{
                display: "grid",
                gridTemplateColumns: "24px 1fr",
                gap: 12,
                padding: "9px 4px",
                borderBottom: "1px solid var(--border-faint)",
              }}
            >
              <span style={{ fontFamily: "var(--font-mono)", fontSize: 11, color: "var(--color-gold)" }}>
                2
              </span>
              <span className="t-body-sm">
                Set a delay between steps so each one settles before the next.
              </span>
            </div>
            <div
              style={{
                display: "grid",
                gridTemplateColumns: "24px 1fr",
                gap: 12,
                padding: "9px 4px",
              }}
            >
              <span style={{ fontFamily: "var(--font-mono)", fontSize: 11, color: "var(--color-gold)" }}>
                3
              </span>
              <span className="t-body-sm">Run it once, loop it, or schedule it to fire on its own.</span>
            </div>
          </div>
          <BuildFirstChainButton onClick={openNewChain} />
        </div>
      )}
    </Page>
  );
}
