import { Fragment, useState, type CSSProperties, type Dispatch, type SetStateAction } from "react";
import { Eye, Eyedropper, Trash, X } from "@phosphor-icons/react";

import type { Step } from "./api";
import { hsvToCss } from "./format";

// The step editor — a faithful port of the panel the Python UI renders inside
// each macro card (`item.se.*`, index.html:819-942) plus its pure edit logic
// (`updateStepAt` / `deleteStepAt` / `moveStepAt` / `insertStepAt` /
// `bulkEditDelays` / `buildStepEditorVals`, index.html:3234-3404). It edits a
// recording as a flat list of actions and lets detection steps be inserted
// between them. The host (MacrosView) owns the async seam — converting the
// recording, and the Test / Save / Run / Close operations that toast — so those
// arrive as callbacks; this component owns only the in-memory list transforms
// (through `setSteps`) and the bulk-delay input's transient value.

/** The host's rendered outcome of a Test click. Carries the step index it ran
 * for so the matching row can highlight (`stepEditorTestResult`, index.html:3329). */
export interface StepEditorTestResult {
  idx: number;
  ok: boolean;
  message: string;
  preview: string | null;
}

export interface StepEditorProps {
  steps: Step[];
  loading: boolean;
  testResult: StepEditorTestResult | null;
  setSteps: Dispatch<SetStateAction<Step[]>>;
  onSave: () => void;
  onRun: () => void;
  onTest: (idx: number) => void;
  /** Sample a target colour from the screen for the detection step at `idx`
   * (host owns the native picker + toast, like the other async seams here). */
  onPickColor: (idx: number) => void;
  onClose: () => void;
}

const TYPE_LABELS: Record<string, string> = {
  click: "Click",
  key: "Key",
  type: "Type",
  scroll: "Scroll",
  delay: "Delay",
  find_click: "Find & Click",
  wait_for: "Wait For",
};

const TYPE_COLORS: Record<string, string> = {
  click: "var(--text-68)",
  key: "var(--text-68)",
  type: "var(--text-68)",
  scroll: "var(--text-68)",
  delay: "var(--text-50)",
  find_click: "var(--color-gold)",
  wait_for: "var(--color-gold)",
};

/** A freshly inserted step. Unlike the Python source's partial dicts, this
 * builds a *complete* `Step`: the list `macroToSteps` returns is already
 * complete objects, so inserts must be too to keep the array uniformly typed.
 * `base` mirrors the Rust `Step` defaults (src-tauri/src/models/step.rs) — keep
 * the two in lockstep. It's invisible either way at the persistence boundary
 * (the backend serde-fills any omitted field to these same values), and only
 * the type-specific fields ever render. The detection types default to the
 * `template` mode the monolith does — which never actually matches, since AI
 * step templates are never loaded (see sidecar `ai_detect`); reproduced, not new. */
function makeStep(type: string): Step {
  const base: Step = {
    id: "s" + Date.now(),
    type,
    enabled: true,
    label: "",
    x: 0,
    y: 0,
    key: "",
    text: "",
    delay: 0,
    scroll_amount: 0,
    detect_mode: "color",
    hsv_low: [0, 0, 0],
    hsv_high: [179, 255, 255],
    template: "",
    region: [0, 0, 100, 100],
    min_area: 40,
    timeout: 10,
    confidence: 0.8,
  };
  const overlays: Record<string, Partial<Step>> = {
    click: { x: 0, y: 0, label: "Click (0, 0)" },
    key: { key: "", label: "Press key" },
    type: { text: "", label: "Type text" },
    scroll: { scroll_amount: 3, x: 0, y: 0, label: "Scroll +3" },
    delay: { delay: 1.0, label: "Wait 1s" },
    find_click: { detect_mode: "template", template: "", region: [0, 0, 100, 100], confidence: 0.8, label: "Find & Click" },
    wait_for: { detect_mode: "template", template: "", region: [0, 0, 100, 100], confidence: 0.8, timeout: 10, label: "Wait For" },
  };
  return { ...base, ...(overlays[type] || {}) };
}

const panelWrap: CSSProperties = {
  borderTop: "1px solid var(--border-faint)",
  background: "rgba(194,163,112,0.03)",
  padding: "14px 18px 16px",
};

const monoLabel: CSSProperties = {
  fontFamily: "var(--font-mono)",
  fontSize: 10,
  letterSpacing: "0.08em",
  textTransform: "uppercase",
  color: "var(--text-50)",
};

const runBtn: CSSProperties = {
  height: 28,
  padding: "0 12px",
  borderRadius: "var(--radius-sm)",
  border: "1px solid var(--border-strong)",
  background: "transparent",
  color: "var(--text-68)",
  fontFamily: "var(--font-sans)",
  fontSize: 11,
  fontWeight: 600,
  cursor: "pointer",
};

const saveBtn: CSSProperties = {
  height: 28,
  padding: "0 12px",
  borderRadius: "var(--radius-sm)",
  border: "none",
  background: "var(--color-ink)",
  color: "var(--color-gold)",
  fontFamily: "var(--font-sans)",
  fontSize: 11,
  fontWeight: 600,
  cursor: "pointer",
};

const closeBtn: CSSProperties = {
  width: 28,
  height: 28,
  border: "none",
  background: "transparent",
  color: "var(--text-40)",
  cursor: "pointer",
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
};

const bulkBtn: CSSProperties = {
  height: 24,
  padding: "0 9px",
  borderRadius: "var(--radius-xs)",
  border: "1px solid var(--border-default)",
  background: "transparent",
  color: "var(--text-68)",
  fontFamily: "var(--font-sans)",
  fontSize: 10,
  fontWeight: 600,
  cursor: "pointer",
};

const iconBtn: CSSProperties = {
  border: "none",
  background: "transparent",
  cursor: "pointer",
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
};

const insertBase: CSSProperties = {
  height: 20,
  padding: "0 8px",
  borderRadius: "var(--radius-xs)",
  fontFamily: "var(--font-sans)",
  fontSize: 9.5,
  fontWeight: 600,
  cursor: "pointer",
};
const insertGold: CSSProperties = {
  ...insertBase,
  border: "1px solid var(--gold-border)",
  background: "rgba(194,163,112,0.06)",
  color: "var(--color-gold)",
};
const insertPlain: CSSProperties = {
  ...insertBase,
  border: "1px solid var(--border-default)",
  background: "transparent",
  color: "var(--text-50)",
};

/** A mono text field of the given width. */
function field(width: number): CSSProperties {
  return {
    width,
    height: 26,
    border: "1px solid var(--border-faint)",
    borderRadius: "var(--radius-sm)",
    padding: "0 6px",
    fontFamily: "var(--font-mono)",
    fontSize: 11,
    color: "var(--color-ink)",
    background: "var(--surface-card)",
  };
}

const fieldUnit: CSSProperties = {
  fontFamily: "var(--font-mono)",
  fontSize: 10,
  color: "var(--text-32)",
};

const pickColorBtn: CSSProperties = {
  height: 26,
  padding: "0 9px",
  borderRadius: "var(--radius-sm)",
  border: "1px solid var(--gold-border)",
  background: "rgba(194,163,112,0.06)",
  color: "var(--color-gold)",
  fontFamily: "var(--font-sans)",
  fontSize: 10.5,
  fontWeight: 600,
  cursor: "pointer",
  display: "flex",
  alignItems: "center",
  gap: 4,
  whiteSpace: "nowrap",
};

/** A colour step still on its full-range default — no colour picked, so it would
 * match every pixel. Drives the "any" hint that tells the user to pick one. */
function hsvIsUnset(low: number[], high: number[]): boolean {
  return (
    low[0] === 0 &&
    low[1] === 0 &&
    low[2] === 0 &&
    high[0] === 179 &&
    high[1] === 255 &&
    high[2] === 255
  );
}

export function StepEditor({
  steps,
  loading,
  testResult,
  setSteps,
  onSave,
  onRun,
  onTest,
  onPickColor,
  onClose,
}: StepEditorProps) {
  // The bulk-delay input's working value — editor-local, transient. Starts null
  // and displays "0.5"; the handlers read the raw value with the same "?? '0.5'"
  // / "?? '1'" fallbacks as the Python source, preserving the quirk that a bare
  // "× factor" click (no typing) multiplies by 1 while the field reads 0.5.
  const [bulkDelayValue, setBulkDelayValue] = useState<string | null>(null);

  const updateStepAt = (idx: number, patch: Partial<Step>) => {
    setSteps((prev) => {
      const next = [...prev];
      next[idx] = { ...next[idx], ...patch };
      return next;
    });
  };

  const deleteStepAt = (idx: number) => {
    setSteps((prev) => prev.filter((_, i) => i !== idx));
  };

  const moveStepAt = (idx: number, dir: number) => {
    setSteps((prev) => {
      const j = idx + dir;
      if (j < 0 || j >= prev.length) return prev;
      const next = [...prev];
      [next[idx], next[j]] = [next[j], next[idx]];
      return next;
    });
  };

  const insertStepAt = (idx: number, type: string) => {
    setSteps((prev) => {
      const next = [...prev];
      next.splice(idx + 1, 0, makeStep(type));
      return next;
    });
  };

  // Bulk-edit every delay step at once — the single most common post-recording
  // cleanup (recorded delays are noisy from human hesitation / system lag).
  const bulkEditDelays = (mode: "set" | "multiply" | "cap", value: number) => {
    if (!(value > 0) && mode !== "set") return;
    if (mode === "set" && value < 0) return;
    setSteps((prev) =>
      prev.map((step) => {
        if (step.type !== "delay") return step;
        const cur = step.delay || 0;
        let next = cur;
        if (mode === "set") next = value;
        else if (mode === "multiply") next = Math.round(cur * value * 100) / 100;
        else if (mode === "cap") next = Math.min(cur, value);
        return { ...step, delay: next, label: "Wait " + next + "s" };
      }),
    );
  };

  const delayCount = steps.filter((s) => s.type === "delay").length;
  const delayCountLabel = delayCount + (delayCount === 1 ? " delay" : " delays");
  const bulkDisplay = bulkDelayValue ?? "0.5";

  return (
    <div style={panelWrap}>
      <div
        style={{
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
          marginBottom: 12,
        }}
      >
        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <span style={monoLabel}>Steps</span>
          <span style={{ fontFamily: "var(--font-sans)", fontSize: 11, color: "var(--text-40)" }}>
            edit the recording as individual actions — insert detection steps between them.
          </span>
        </div>
        <div style={{ display: "flex", gap: 6 }}>
          <button type="button" onClick={onRun} style={runBtn}>
            Run
          </button>
          <button
            type="button"
            onClick={onSave}
            onMouseEnter={(e) => (e.currentTarget.style.background = "#2b2825")}
            onMouseLeave={(e) => (e.currentTarget.style.background = "var(--color-ink)")}
            style={saveBtn}
          >
            Save
          </button>
          <button type="button" onClick={onClose} title="Close step editor" style={closeBtn}>
            <X size={13} />
          </button>
        </div>
      </div>

      {delayCount > 0 && (
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: 8,
            flexWrap: "wrap",
            padding: "8px 10px",
            marginBottom: 10,
            border: "1px solid var(--gold-border)",
            borderRadius: "var(--radius-sm)",
            background: "rgba(194,163,112,0.05)",
          }}
        >
          <span
            style={{
              fontFamily: "var(--font-mono)",
              fontSize: 9.5,
              letterSpacing: "0.04em",
              textTransform: "uppercase",
              color: "var(--color-gold)",
            }}
          >
            Bulk delays
          </span>
          <span style={{ fontFamily: "var(--font-sans)", fontSize: 10.5, color: "var(--text-40)" }}>
            {delayCountLabel}
          </span>
          <input
            value={bulkDisplay}
            onChange={(e) => setBulkDelayValue(e.target.value)}
            placeholder="sec"
            style={{
              width: 56,
              height: 24,
              border: "1px solid var(--border-faint)",
              borderRadius: "var(--radius-xs)",
              padding: "0 6px",
              fontFamily: "var(--font-mono)",
              fontSize: 11,
              color: "var(--color-ink)",
              background: "var(--surface-card)",
            }}
          />
          <button
            type="button"
            onClick={() => bulkEditDelays("set", parseFloat(bulkDelayValue ?? "0.5"))}
            title="Set every delay to this value"
            style={bulkBtn}
          >
            Set all
          </button>
          <button
            type="button"
            onClick={() => bulkEditDelays("multiply", parseFloat(bulkDelayValue ?? "1"))}
            title="Multiply every delay by this factor"
            style={bulkBtn}
          >
            × factor
          </button>
          <button
            type="button"
            onClick={() => bulkEditDelays("cap", parseFloat(bulkDelayValue ?? "0.5"))}
            title="Cap every delay at this value"
            style={bulkBtn}
          >
            Cap at
          </button>
        </div>
      )}

      {loading && (
        <div
          style={{
            padding: 20,
            textAlign: "center",
            fontFamily: "var(--font-sans)",
            fontSize: 12,
            color: "var(--text-40)",
          }}
        >
          Converting recording to steps...
        </div>
      )}

      {!loading && (
        <>
          <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
            {steps.map((step, i) => {
              const isClick = step.type === "click";
              const isKey = step.type === "key";
              const isType = step.type === "type";
              const isScroll = step.type === "scroll";
              const isDelay = step.type === "delay";
              const isDetection = step.type === "find_click" || step.type === "wait_for";
              const isWaitFor = step.type === "wait_for";
              const typeLabel = TYPE_LABELS[step.type] ?? step.type;
              const typeColor = TYPE_COLORS[step.type] ?? "var(--text-68)";
              const tested = !!testResult && testResult.idx === i;
              const rowBorder = tested
                ? testResult!.ok
                  ? "var(--verdict-allowed)"
                  : "var(--verdict-blocked)"
                : "var(--border-faint)";
              const rowBg = tested
                ? testResult!.ok
                  ? "rgba(95,174,95,0.06)"
                  : "rgba(194,95,95,0.06)"
                : "var(--surface-card)";
              return (
                <Fragment key={step.id}>
                  <div
                    style={{
                      display: "grid",
                      gridTemplateColumns: "auto 1fr auto",
                      gap: 10,
                      alignItems: "center",
                      padding: "8px 12px",
                      borderRadius: "var(--radius-md)",
                      border: "1px solid " + rowBorder,
                      background: rowBg,
                      opacity: step.enabled ? 1 : 0.45,
                    }}
                  >
                    {/* Left: type badge + enable toggle */}
                    <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                      <span
                        style={{
                          fontFamily: "var(--font-mono)",
                          fontSize: 9.5,
                          letterSpacing: "0.04em",
                          textTransform: "uppercase",
                          color: typeColor,
                          minWidth: 64,
                        }}
                      >
                        {typeLabel}
                      </span>
                      <button
                        type="button"
                        onClick={() => updateStepAt(i, { enabled: !step.enabled })}
                        style={{
                          width: 30,
                          height: 17,
                          borderRadius: 9,
                          border: "none",
                          background: step.enabled ? "var(--color-ink)" : "rgba(26,24,22,0.15)",
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
                            left: step.enabled ? 15 : 2.5,
                            transition: "left .18s",
                          }}
                        />
                      </button>
                    </div>

                    {/* Middle: label + type-specific fields */}
                    <div style={{ display: "flex", alignItems: "center", gap: 8, minWidth: 0 }}>
                      <input
                        value={step.label}
                        onChange={(e) => updateStepAt(i, { label: e.target.value })}
                        placeholder="Label"
                        style={{
                          flex: 1,
                          minWidth: 80,
                          height: 26,
                          border: "1px solid var(--border-faint)",
                          borderRadius: "var(--radius-sm)",
                          padding: "0 8px",
                          fontFamily: "var(--font-sans)",
                          fontSize: 11.5,
                          color: "var(--color-ink)",
                          background: "var(--surface-card)",
                        }}
                      />

                      {isClick && (
                        <>
                          <input
                            value={step.x}
                            onChange={(e) => updateStepAt(i, { x: parseInt(e.target.value) || 0 })}
                            placeholder="X"
                            style={field(52)}
                          />
                          <input
                            value={step.y}
                            onChange={(e) => updateStepAt(i, { y: parseInt(e.target.value) || 0 })}
                            placeholder="Y"
                            style={field(52)}
                          />
                        </>
                      )}

                      {isKey && (
                        <input
                          value={step.key}
                          onChange={(e) => updateStepAt(i, { key: e.target.value })}
                          placeholder="Key"
                          style={field(70)}
                        />
                      )}

                      {isType && (
                        <input
                          value={step.text}
                          onChange={(e) => updateStepAt(i, { text: e.target.value })}
                          placeholder="Text"
                          style={field(120)}
                        />
                      )}

                      {isScroll && (
                        <input
                          value={step.scroll_amount}
                          onChange={(e) =>
                            updateStepAt(i, { scroll_amount: parseInt(e.target.value) || 0 })
                          }
                          placeholder="+/-"
                          style={field(52)}
                        />
                      )}

                      {isDelay && (
                        <>
                          <input
                            value={step.delay}
                            onChange={(e) =>
                              updateStepAt(i, { delay: parseFloat(e.target.value) || 0 })
                            }
                            placeholder="sec"
                            style={field(52)}
                          />
                          <span style={fieldUnit}>sec</span>
                        </>
                      )}

                      {isDetection && (
                        <>
                          <select
                            value={step.detect_mode}
                            onChange={(e) => updateStepAt(i, { detect_mode: e.target.value })}
                            style={{
                              height: 26,
                              border: "1px solid var(--border-faint)",
                              borderRadius: "var(--radius-sm)",
                              padding: "0 6px",
                              fontFamily: "var(--font-mono)",
                              fontSize: 10.5,
                              color: "var(--color-ink)",
                              background: "var(--surface-card)",
                              cursor: "pointer",
                            }}
                          >
                            <option value="color">Color</option>
                            <option value="template">Template</option>
                            <option value="features">Features</option>
                          </select>
                          <input
                            value={step.template}
                            onChange={(e) => updateStepAt(i, { template: e.target.value })}
                            placeholder="template"
                            style={field(90)}
                          />
                          <input
                            value={step.confidence}
                            onChange={(e) =>
                              updateStepAt(i, { confidence: parseFloat(e.target.value) || 0.8 })
                            }
                            placeholder="conf"
                            style={field(44)}
                          />
                          {step.detect_mode === "color" && (
                            <>
                              <button
                                type="button"
                                onClick={() => onPickColor(i)}
                                title="Sample the target colour from the screen"
                                style={pickColorBtn}
                              >
                                <Eyedropper size={12} weight="bold" /> Pick
                              </button>
                              {hsvIsUnset(step.hsv_low, step.hsv_high) ? (
                                <span
                                  title="No colour picked — matches anything. Click Pick."
                                  style={{ ...fieldUnit, color: "var(--text-40)", cursor: "default" }}
                                >
                                  any
                                </span>
                              ) : (
                                <span
                                  title={
                                    "H " + step.hsv_low[0] + "–" + step.hsv_high[0] +
                                    "   S " + step.hsv_low[1] + "–" + step.hsv_high[1] +
                                    "   V " + step.hsv_low[2] + "–" + step.hsv_high[2]
                                  }
                                  style={{
                                    width: 18,
                                    height: 18,
                                    borderRadius: 4,
                                    border: "1px solid var(--border-default)",
                                    background: hsvToCss(step.hsv_low, step.hsv_high),
                                    flexShrink: 0,
                                  }}
                                />
                              )}
                            </>
                          )}
                        </>
                      )}

                      {isWaitFor && (
                        <>
                          <input
                            value={step.timeout}
                            onChange={(e) =>
                              updateStepAt(i, { timeout: parseFloat(e.target.value) || 10 })
                            }
                            placeholder="timeout"
                            style={field(44)}
                          />
                          <span style={fieldUnit}>s</span>
                        </>
                      )}
                    </div>

                    {/* Right: per-step actions. Up/down are never disabled — the
                        Python source declared `upDisabled`/`downDisabled` for step
                        rows but never set them, so both always rendered enabled. */}
                    <div style={{ display: "flex", alignItems: "center", gap: 2 }}>
                      <button
                        type="button"
                        onClick={() => onTest(i)}
                        title="Test this step"
                        style={{ ...iconBtn, width: 24, height: 24, color: "var(--text-50)" }}
                      >
                        <Eye size={13} />
                      </button>
                      <button
                        type="button"
                        onClick={() => moveStepAt(i, -1)}
                        title="Move up"
                        style={{
                          ...iconBtn,
                          width: 22,
                          height: 24,
                          color: "var(--text-40)",
                          fontSize: 11,
                        }}
                      >
                        ▲
                      </button>
                      <button
                        type="button"
                        onClick={() => moveStepAt(i, 1)}
                        title="Move down"
                        style={{
                          ...iconBtn,
                          width: 22,
                          height: 24,
                          color: "var(--text-40)",
                          fontSize: 11,
                        }}
                      >
                        ▼
                      </button>
                      <button
                        type="button"
                        onClick={() => deleteStepAt(i)}
                        title="Delete step"
                        style={{ ...iconBtn, width: 24, height: 24, color: "var(--text-40)" }}
                      >
                        <Trash size={12} />
                      </button>
                    </div>
                  </div>

                  {/* Insert bar after each step */}
                  <div style={{ display: "flex", alignItems: "center", gap: 4, padding: "2px 12px" }}>
                    <span
                      style={{
                        fontFamily: "var(--font-mono)",
                        fontSize: 9,
                        color: "var(--text-25)",
                        marginRight: 4,
                      }}
                    >
                      insert:
                    </span>
                    <button type="button" onClick={() => insertStepAt(i, "find_click")} style={insertGold}>
                      Find &amp; Click
                    </button>
                    <button type="button" onClick={() => insertStepAt(i, "wait_for")} style={insertGold}>
                      Wait For
                    </button>
                    <button type="button" onClick={() => insertStepAt(i, "delay")} style={insertPlain}>
                      Delay
                    </button>
                    <button type="button" onClick={() => insertStepAt(i, "click")} style={insertPlain}>
                      Click
                    </button>
                    <button type="button" onClick={() => insertStepAt(i, "key")} style={insertPlain}>
                      Key
                    </button>
                  </div>
                </Fragment>
              );
            })}
          </div>

          {testResult && (
            <div
              style={{
                marginTop: 10,
                padding: "10px 12px",
                borderRadius: "var(--radius-md)",
                border:
                  "1px solid " +
                  (testResult.ok ? "var(--verdict-allowed)" : "var(--verdict-blocked)"),
                background: testResult.ok ? "rgba(95,174,95,0.06)" : "rgba(194,95,95,0.06)",
              }}
            >
              <div
                style={{
                  fontFamily: "var(--font-sans)",
                  fontSize: 11.5,
                  fontWeight: 600,
                  color: testResult.ok ? "var(--verdict-allowed)" : "var(--verdict-blocked)",
                }}
              >
                {testResult.ok ? "Match" : "No match"}
              </div>
              <div
                style={{
                  fontFamily: "var(--font-mono)",
                  fontSize: 10.5,
                  color: "var(--text-50)",
                  marginTop: 3,
                }}
              >
                {testResult.message}
              </div>
              {testResult.preview && (
                <img
                  src={testResult.preview}
                  style={{
                    marginTop: 8,
                    maxWidth: "100%",
                    borderRadius: "var(--radius-sm)",
                    border: "1px solid var(--border-faint)",
                  }}
                />
              )}
            </div>
          )}

          {steps.length === 0 && (
            <div
              style={{
                border: "1px dashed var(--border-default)",
                borderRadius: "var(--radius-md)",
                padding: 16,
                textAlign: "center",
              }}
            >
              <span style={{ fontFamily: "var(--font-sans)", fontSize: 11.5, color: "var(--text-40)" }}>
                No steps. The recording had no recognizable actions, or it hasn't been converted yet.
              </span>
            </div>
          )}
        </>
      )}
    </div>
  );
}
