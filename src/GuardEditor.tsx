import type { CSSProperties } from "react";
import { X, Crosshair, Selection, Plus, Trash } from "@phosphor-icons/react";

import {
  guardPickColor,
  guardPickRegion,
  captureTemplate,
  addTemplateImage,
  surgicalCapture,
} from "./api";
import { hsvToCss } from "./format";
import type { ToastKind } from "./components/Toasts";

// The shared guard / trigger editor — a faithful port of the inline editor the
// Python UI renders inside each macro card (`item.ge.*`, index.html:578-758) and
// reuses for Vision triggers via the `__vision__` macro id. One editor, two
// hosts: MacrosView owns per-macro guards, VisionView owns triggers; both keep
// the working copy in their own state and pass it here as `draft`, along with a
// `macroId`-aware `onSave`. The Detection method picks color / image / OCR; the
// native "Pick from screen" / "Capture" / "Add image" / "Change area" flows call
// the picker commands directly and merge their results back through `onChange`.
// Vision triggers turn on two extras the per-macro editor omits (matching the
// Python `visionEditor` template): `showSurgical` renders the Surgical-capture
// button + click-offset chip, and `saveLabel` relabels the save button.

/** One queued macro in the post-trigger orchestration list. `name` holds a macro
 * id (the select's option values are ids), matching the Python source. */
export interface SeqStep {
  name?: string;
  repeat?: number;
}

/** The Test button's rendered result: a base64 JPEG plus its verdict. */
export interface GuardPreview {
  src: string;
  ok: boolean;
  msg?: string;
}

/** The editor's working copy of a guard. Superset of the persisted fields plus
 * the transient underscore-prefixed ones (`_preview`, `_templateThumb`) that the
 * host strips before saving. Mirrors the Python `guardEditor` object. */
export interface GuardDraft {
  /** Unknown backend keys pass through untyped, mirroring the dynamic Python
   * guard dict (and keeping this assignable to the api-layer `Guard`). */
  [key: string]: unknown;
  id: string;
  name?: string;
  method?: string;
  action?: string;
  key?: string;
  hsv_low?: number[];
  hsv_high?: number[];
  region?: number[];
  min_area?: number;
  template_path?: string;
  threshold?: number;
  ocr_text?: string;
  resume_delay?: number;
  cooldown?: number;
  enabled?: boolean;
  click_offset?: number[];
  click_line?: number[];
  click_lines?: number[][];
  macro_sequence?: SeqStep[];
  macroId?: string;
  _isNew?: boolean;
  _preview?: GuardPreview | null;
  _templateThumb?: string;
}

export interface GuardEditorProps {
  draft: GuardDraft;
  /** Macros offered in the orchestration sequence dropdown (`{ id, name }`). */
  macros: { id: string; name: string }[];
  /** Merge a partial patch into the draft — the `updateGuardEditor` analogue. */
  onChange: (patch: Partial<GuardDraft>) => void;
  onTest: () => void;
  onSave: () => void;
  onCancel: () => void;
  push: (kind: ToastKind, text: string) => void;
  /** Vision-trigger extras: the Surgical-capture button + click-offset chip.
   * Off for per-macro guards (whose editor omits them), on for Vision triggers. */
  showSurgical?: boolean;
  /** Save-button label — "Save Guard" for macros, "Save Trigger" for Vision. */
  saveLabel?: string;
}

const fieldLabel: CSSProperties = {
  fontFamily: "var(--font-mono)",
  fontSize: 9.5,
  letterSpacing: "0.06em",
  textTransform: "uppercase",
  color: "var(--text-40)",
};

const textInput: CSSProperties = {
  width: "100%",
  boxSizing: "border-box",
  height: 30,
  border: "1px solid var(--border-default)",
  borderRadius: "var(--radius-sm)",
  padding: "0 9px",
  fontFamily: "var(--font-sans)",
  fontSize: 12,
  color: "var(--color-ink)",
  background: "var(--surface-card)",
};

const monoInput: CSSProperties = { ...textInput, fontFamily: "var(--font-mono)" };

export function GuardEditor({
  draft,
  macros,
  onChange,
  onTest,
  onSave,
  onCancel,
  push,
  showSurgical = false,
  saveLabel = "Save Guard",
}: GuardEditorProps) {
  const method = draft.method || "color";
  const isColor = method === "color";
  const isTemplate = method === "template";
  const isOcr = method === "ocr";
  const isKey = draft.action === "key";

  const region = draft.region || [0, 0, 100, 100];
  const regionIsFull =
    region[0] <= 0.5 && region[1] <= 0.5 && region[2] >= 99.5 && region[3] >= 99.5;
  const regionLabel = regionIsFull
    ? "Whole screen"
    : Math.round(region[0]) +
      "," +
      Math.round(region[1]) +
      " → " +
      Math.round(region[2]) +
      "," +
      Math.round(region[3]) +
      "%";
  const regionBtnLabel = regionIsFull ? "Limit area" : "Change area";

  const threshold = draft.threshold ?? 0.8;
  const preview = draft._preview;

  // The surgical-capture "hit target" chip (Vision only). A multi-stroke capture
  // reads as "N strokes"; a single 4-element line reads as a point or an arrow
  // between its endpoints; a bare offset reads as a point.
  const clickLines = draft.click_lines;
  const clickLine = draft.click_line;
  const clickOffset = draft.click_offset;
  const hasClickOffset = !!(
    (clickLines && clickLines.length) ||
    (clickLine && clickLine.length === 4) ||
    (clickOffset && clickOffset.length === 2)
  );
  const clickOffsetLabel =
    clickLines && clickLines.length > 1
      ? clickLines.length + " strokes"
      : clickLine && clickLine.length === 4
        ? clickLine[0] === clickLine[2] && clickLine[1] === clickLine[3]
          ? "(" + clickLine[0] + ", " + clickLine[1] + ")"
          : "(" + clickLine[0] + ", " + clickLine[1] + ")→(" + clickLine[2] + ", " + clickLine[3] + ")"
        : clickOffset
          ? "(" + clickOffset[0] + ", " + clickOffset[1] + ")"
          : "";

  // ── Native capture flows — merge each result back through onChange ──
  const onPickColor = async () => {
    try {
      const res = await guardPickColor();
      if (res && res.ok) {
        onChange({ hsv_low: res.hsv_low, hsv_high: res.hsv_high });
        push("success", "Color set: " + res.hex);
      }
    } catch {
      push("error", "Color pick failed");
    }
  };

  const onPickRegion = async () => {
    try {
      const res = await guardPickRegion();
      if (res && res.ok) {
        // Convert pixels → percentage of screen (2560x1440).
        const sw = 2560;
        const sh = 1440;
        const next = [
          ((res.x ?? 0) / sw) * 100,
          ((res.y ?? 0) / sh) * 100,
          ((res.x ?? 0) + (res.w ?? 0)) / sw * 100,
          ((res.y ?? 0) + (res.h ?? 0)) / sh * 100,
        ];
        onChange({ region: next });
        push("success", "Watch area set: " + res.w + "×" + res.h);
      }
    } catch {
      push("error", "Region pick failed");
    }
  };

  const onCaptureTemplate = async () => {
    try {
      const res = await captureTemplate();
      if (res && res.ok) {
        onChange({
          template_path: res.path,
          _templateThumb: "data:image/png;base64," + res.thumb,
        });
        push("success", "Button captured (" + res.w + "×" + res.h + ")");
      }
    } catch (e) {
      push("error", "Capture failed: " + e);
    }
  };

  const onSurgical = async () => {
    try {
      const res = await surgicalCapture();
      if (res && res.ok) {
        onChange({
          method: "template",
          template_path: res.path,
          _templateThumb: "data:image/png;base64," + res.thumb,
          click_offset: res.offset,
          click_line: res.click_line,
          click_lines: res.click_lines,
        });
        const n = (res.click_lines || []).length;
        push(
          "success",
          "Surgical " + res.w + "×" + res.h + ": " + n + " stroke" + (n === 1 ? "" : "s") + " drawn",
        );
      }
    } catch {
      push("error", "Surgical capture failed");
    }
  };

  const onAddImage = async () => {
    try {
      const res = await addTemplateImage();
      if (res && res.ok) {
        onChange({
          template_path: res.path,
          _templateThumb: "data:image/png;base64," + res.thumb,
        });
        push("success", "Image added (" + res.w + "×" + res.h + ")");
      } else if (res && res.error && res.error !== "cancelled") {
        push("error", res.error);
      }
    } catch (e) {
      push("error", "Add image failed: " + e);
    }
  };

  // ── Orchestration sequence (macros run after the trigger fires) ──
  const seq = draft.macro_sequence || [];
  const onAddSeqStep = () => onChange({ macro_sequence: [...seq, { name: "", repeat: 1 }] });
  const updateSeqStep = (idx: number, patch: Partial<SeqStep>) =>
    onChange({ macro_sequence: seq.map((s, i) => (i === idx ? { ...s, ...patch } : s)) });
  const deleteSeqStep = (idx: number) =>
    onChange({ macro_sequence: seq.filter((_, i) => i !== idx) });

  return (
    <div
      style={{
        border: "1px solid var(--color-gold)",
        borderRadius: "var(--radius-md)",
        background: "var(--surface-card-strong)",
        padding: "14px 16px",
      }}
    >
      <div
        style={{
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
          marginBottom: 12,
        }}
      >
        <span
          style={{
            fontFamily: "var(--font-mono)",
            fontSize: 10,
            letterSpacing: "0.08em",
            textTransform: "uppercase",
            color: "var(--color-gold)",
          }}
        >
          {draft._isNew ? "New Guard" : "Edit Guard"}
        </span>
        <button
          type="button"
          onClick={onCancel}
          style={{
            border: "none",
            background: "transparent",
            color: "var(--text-40)",
            cursor: "pointer",
            display: "flex",
            alignItems: "center",
          }}
        >
          <X size={13} />
        </button>
      </div>

      <div
        style={{
          display: "grid",
          gridTemplateColumns: "1fr 1fr",
          gap: 12,
          marginBottom: 12,
        }}
      >
        <div>
          <div style={{ ...fieldLabel, marginBottom: 4 }}>Name</div>
          <input
            value={draft.name || ""}
            onChange={(e) => onChange({ name: e.target.value })}
            placeholder="e.g. Reconnect"
            style={textInput}
          />
        </div>
        <div>
          <div style={{ ...fieldLabel, marginBottom: 4 }}>When found</div>
          <div style={{ display: "flex", gap: 6 }}>
            <button
              type="button"
              onClick={() => onChange({ action: "click" })}
              style={{
                flex: 1,
                height: 30,
                borderRadius: "var(--radius-sm)",
                border: "1px solid var(--border-default)",
                background: !isKey ? "var(--color-ink)" : "transparent",
                color: !isKey ? "var(--color-gold)" : "var(--text-68)",
                fontFamily: "var(--font-sans)",
                fontSize: 11,
                fontWeight: 600,
                cursor: "pointer",
              }}
            >
              Click it
            </button>
            <button
              type="button"
              onClick={() => onChange({ action: "key" })}
              style={{
                flex: 1,
                height: 30,
                borderRadius: "var(--radius-sm)",
                border: "1px solid var(--border-default)",
                background: isKey ? "var(--color-ink)" : "transparent",
                color: isKey ? "var(--color-gold)" : "var(--text-68)",
                fontFamily: "var(--font-sans)",
                fontSize: 11,
                fontWeight: 600,
                cursor: "pointer",
              }}
            >
              Press key
            </button>
          </div>
        </div>
      </div>

      {isKey && (
        <div style={{ marginBottom: 12 }}>
          <div style={{ ...fieldLabel, marginBottom: 4 }}>Key to press</div>
          <input
            value={draft.key || ""}
            onChange={(e) => onChange({ key: e.target.value })}
            placeholder="e.g. Enter"
            style={{
              width: 140,
              height: 30,
              border: "1px solid var(--border-default)",
              borderRadius: "var(--radius-sm)",
              padding: "0 9px",
              fontFamily: "var(--font-mono)",
              fontSize: 12,
              color: "var(--color-ink)",
              background: "var(--surface-card)",
            }}
          />
        </div>
      )}

      <div style={{ marginBottom: 12 }}>
        <div style={{ ...fieldLabel, marginBottom: 5 }}>Detection method</div>
        <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr 1fr", gap: 6 }}>
          <MethodButton
            label="Color"
            hint="fastest · basic"
            active={isColor}
            onClick={() => onChange({ method: "color" })}
          />
          <MethodButton
            label="Image"
            hint="fast · accurate"
            active={isTemplate}
            onClick={() => onChange({ method: "template" })}
          />
          <MethodButton
            label="Text (OCR)"
            hint="slowest · robust"
            active={isOcr}
            onClick={() => onChange({ method: "ocr" })}
          />
        </div>
      </div>

      {isColor && (
        <div style={{ marginBottom: 12 }}>
          <div style={{ ...fieldLabel, marginBottom: 4 }}>Trigger color</div>
          <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
            <span
              style={{
                width: 26,
                height: 26,
                borderRadius: 6,
                border: "1px solid var(--border-default)",
                background: hsvToCss(draft.hsv_low, draft.hsv_high),
                flexShrink: 0,
              }}
            />
            <button
              type="button"
              onClick={onPickColor}
              style={{
                flex: 1,
                height: 30,
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
              <span
                style={{
                  display: "inline-flex",
                  alignItems: "center",
                  gap: 6,
                  justifyContent: "center",
                }}
              >
                <Crosshair size={12} />
                Pick from screen
              </span>
            </button>
          </div>
        </div>
      )}

      {isTemplate && (
        <div style={{ marginBottom: 12 }}>
          <div style={{ ...fieldLabel, marginBottom: 4 }}>Button image</div>
          <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
            {draft.template_path && (
              <img
                src={draft._templateThumb || ""}
                style={{
                  height: 30,
                  maxWidth: 90,
                  objectFit: "contain",
                  borderRadius: "var(--radius-sm)",
                  border: "1px solid var(--border-default)",
                  background: "#000",
                  flexShrink: 0,
                }}
              />
            )}
            <button
              type="button"
              onClick={onCaptureTemplate}
              style={{
                flex: 1,
                height: 30,
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
              <span
                style={{
                  display: "inline-flex",
                  alignItems: "center",
                  gap: 6,
                  justifyContent: "center",
                }}
              >
                <Selection size={12} />
                {draft.template_path ? "Re-capture" : "Capture from screen"}
              </span>
            </button>
            {showSurgical && (
              <button
                type="button"
                onClick={onSurgical}
                title="Drag a tight box, then DRAW the exact pixels to hit (drag a line across the element)"
                style={{
                  flex: 1,
                  height: 30,
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
                <span
                  style={{
                    display: "inline-flex",
                    alignItems: "center",
                    gap: 6,
                    justifyContent: "center",
                  }}
                >
                  <Crosshair size={12} />
                  Surgical
                </span>
              </button>
            )}
            <button
              type="button"
              onClick={onAddImage}
              style={{
                flex: 1,
                height: 30,
                borderRadius: "var(--radius-sm)",
                border: "1px solid var(--border-default)",
                background: "var(--surface-card)",
                color: "var(--text-78)",
                fontFamily: "var(--font-sans)",
                fontSize: 11,
                fontWeight: 600,
                cursor: "pointer",
              }}
            >
              <span
                style={{
                  display: "inline-flex",
                  alignItems: "center",
                  gap: 6,
                  justifyContent: "center",
                }}
              >
                <Plus size={12} />
                Add image
              </span>
            </button>
          </div>
          {showSurgical && hasClickOffset && (
            <div style={{ display: "flex", alignItems: "center", gap: 6, marginTop: 8 }}>
              <span
                style={{
                  display: "inline-flex",
                  alignItems: "center",
                  gap: 5,
                  height: 26,
                  padding: "0 10px",
                  borderRadius: "var(--radius-sm)",
                  border: "1px solid var(--color-gold)",
                  background: "rgba(194,163,112,0.06)",
                  fontFamily: "var(--font-mono)",
                  fontSize: 10.5,
                  color: "var(--color-gold)",
                }}
              >
                <Crosshair size={12} />
                {"hit @ " + clickOffsetLabel}
              </span>
            </div>
          )}
          <div style={{ display: "flex", alignItems: "center", gap: 8, marginTop: 8 }}>
            <span style={{ ...fieldLabel, whiteSpace: "nowrap" }}>Match ≥</span>
            <input
              type="range"
              min="0.5"
              max="0.99"
              step="0.01"
              value={threshold}
              onChange={(e) => onChange({ threshold: parseFloat(e.target.value) || 0.8 })}
              style={{ flex: 1, accentColor: "var(--color-gold)" }}
            />
            <span
              style={{
                fontFamily: "var(--font-mono)",
                fontSize: 11,
                color: "var(--text-68)",
                width: 38,
                textAlign: "right",
              }}
            >
              {Math.round(threshold * 100) + "%"}
            </span>
          </div>
        </div>
      )}

      {isOcr && (
        <div style={{ marginBottom: 12 }}>
          <div style={{ ...fieldLabel, marginBottom: 4 }}>Text to find</div>
          <input
            value={draft.ocr_text || ""}
            onChange={(e) => onChange({ ocr_text: e.target.value })}
            placeholder="e.g. Reconnect"
            style={textInput}
          />
          <div
            style={{
              fontFamily: "var(--font-sans)",
              fontSize: 10.5,
              color: "var(--text-40)",
              marginTop: 5,
            }}
          >
            Reads on-screen text and clicks where it appears. Survives button redesigns and color
            changes.
          </div>
        </div>
      )}

      <div style={{ marginBottom: 12 }}>
        <div
          style={{
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            marginBottom: 4,
          }}
        >
          <div style={fieldLabel}>Watch area</div>
          <span style={{ fontFamily: "var(--font-mono)", fontSize: 10, color: "var(--text-50)" }}>
            {regionLabel}
          </span>
        </div>
        <button
          type="button"
          onClick={onPickRegion}
          style={{
            width: "100%",
            height: 30,
            borderRadius: "var(--radius-sm)",
            border: "1px solid var(--border-default)",
            background: "var(--surface-card)",
            color: "var(--text-78)",
            fontFamily: "var(--font-sans)",
            fontSize: 11,
            fontWeight: 600,
            cursor: "pointer",
          }}
        >
          <span
            style={{
              display: "inline-flex",
              alignItems: "center",
              gap: 6,
              justifyContent: "center",
            }}
          >
            <Selection size={12} />
            {regionBtnLabel}
          </span>
        </button>
        <div
          style={{
            fontFamily: "var(--font-sans)",
            fontSize: 10.5,
            color: "var(--text-32)",
            marginTop: 5,
          }}
        >
          Scans the whole screen by default. Limit the area only to speed up detection.
        </div>
      </div>

      <div
        style={{
          display: "grid",
          gridTemplateColumns: "1fr 1fr auto",
          gap: 12,
          alignItems: "end",
          marginBottom: 12,
        }}
      >
        <div>
          <div style={{ ...fieldLabel, marginBottom: 4 }}>Wait after (sec)</div>
          <input
            type="number"
            step="0.5"
            value={draft.resume_delay ?? 3}
            onChange={(e) => onChange({ resume_delay: parseFloat(e.target.value) || 0 })}
            style={monoInput}
          />
        </div>
        {isColor ? (
          <div>
            <div style={{ ...fieldLabel, marginBottom: 4 }}>Min blob size</div>
            <input
              type="number"
              value={draft.min_area ?? 40}
              onChange={(e) => onChange({ min_area: parseInt(e.target.value) || 0 })}
              style={monoInput}
            />
          </div>
        ) : (
          <div />
        )}
        <button
          type="button"
          onClick={onTest}
          style={{
            height: 30,
            padding: "0 14px",
            borderRadius: "var(--radius-sm)",
            border: "1px solid var(--border-strong)",
            background: "transparent",
            color: "var(--text-78)",
            fontFamily: "var(--font-sans)",
            fontSize: 11.5,
            fontWeight: 600,
            cursor: "pointer",
            whiteSpace: "nowrap",
          }}
        >
          Test
        </button>
      </div>

      {preview && preview.src && (
        <div style={{ marginBottom: 12 }}>
          <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 6 }}>
            <span
              style={{
                width: 8,
                height: 8,
                borderRadius: "50%",
                background: preview.ok ? "var(--verdict-allowed)" : "var(--verdict-blocked)",
                flexShrink: 0,
              }}
            />
            <span
              style={{
                fontFamily: "var(--font-mono)",
                fontSize: 10.5,
                fontWeight: 600,
                color: preview.ok ? "var(--verdict-allowed)" : "var(--verdict-blocked)",
              }}
            >
              {preview.msg}
            </span>
          </div>
          <img
            src={preview.src}
            style={{
              width: "100%",
              maxHeight: 180,
              objectFit: "contain",
              borderRadius: "var(--radius-sm)",
              border: "1px solid var(--border-faint)",
              background: "#000",
            }}
          />
        </div>
      )}

      <div style={{ marginBottom: 12 }}>
        <div
          style={{
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            marginBottom: 5,
          }}
        >
          <div style={fieldLabel}>Run macros after (orchestration)</div>
          <button
            type="button"
            onClick={onAddSeqStep}
            style={{
              display: "inline-flex",
              alignItems: "center",
              gap: 5,
              height: 22,
              padding: "0 8px",
              borderRadius: "var(--radius-sm)",
              border: "1px solid var(--color-gold)",
              background: "rgba(194,163,112,0.08)",
              color: "var(--color-gold)",
              fontFamily: "var(--font-sans)",
              fontSize: 10,
              fontWeight: 600,
              cursor: "pointer",
              whiteSpace: "nowrap",
            }}
          >
            <Plus size={11} />
            Add Step
          </button>
        </div>
        {seq.length > 0 ? (
          <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
            {seq.map((step, idx) => (
              <div
                key={idx}
                style={{
                  display: "grid",
                  gridTemplateColumns: "1fr auto auto",
                  gap: 8,
                  alignItems: "center",
                  padding: "7px 10px",
                  borderRadius: "var(--radius-sm)",
                  border: "1px solid var(--border-faint)",
                  background: "var(--surface-card-strong)",
                }}
              >
                <select
                  value={step.name || ""}
                  onChange={(e) => updateSeqStep(idx, { name: e.target.value })}
                  style={{
                    width: "100%",
                    height: 28,
                    border: "1px solid var(--border-default)",
                    borderRadius: "var(--radius-sm)",
                    padding: "0 8px",
                    fontFamily: "var(--font-sans)",
                    fontSize: 11.5,
                    color: "var(--color-ink)",
                    background: "var(--surface-card)",
                    cursor: "pointer",
                  }}
                >
                  <option value="">Select macro…</option>
                  {macros.map((m) => (
                    <option key={m.id} value={m.id}>
                      {m.name}
                    </option>
                  ))}
                </select>
                <select
                  value={String(step.repeat ?? 1)}
                  onChange={(e) => updateSeqStep(idx, { repeat: parseInt(e.target.value) || 1 })}
                  style={{
                    width: 70,
                    height: 28,
                    border: "1px solid var(--border-default)",
                    borderRadius: "var(--radius-sm)",
                    padding: "0 6px",
                    fontFamily: "var(--font-mono)",
                    fontSize: 11,
                    color: "var(--color-ink)",
                    background: "var(--surface-card)",
                    cursor: "pointer",
                  }}
                >
                  <option value="1">×1</option>
                  <option value="2">×2</option>
                  <option value="3">×3</option>
                  <option value="5">×5</option>
                  <option value="10">×10</option>
                  <option value="0">×∞</option>
                </select>
                <button
                  type="button"
                  onClick={() => deleteSeqStep(idx)}
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
                  <Trash size={12} />
                </button>
              </div>
            ))}
          </div>
        ) : (
          <div
            style={{
              fontFamily: "var(--font-sans)",
              fontSize: 10.5,
              color: "var(--text-32)",
              padding: "6px 0",
            }}
          >
            No macros queued. Add steps to run them in order after this trigger fires.
          </div>
        )}
      </div>

      <div style={{ display: "flex", justifyContent: "flex-end", gap: 8 }}>
        <button
          type="button"
          onClick={onCancel}
          style={{
            height: 30,
            padding: "0 14px",
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
          Cancel
        </button>
        <button
          type="button"
          onClick={onSave}
          onMouseEnter={(e) => (e.currentTarget.style.background = "#2b2825")}
          onMouseLeave={(e) => (e.currentTarget.style.background = "var(--color-ink)")}
          style={{
            height: 30,
            padding: "0 16px",
            borderRadius: "var(--radius-sm)",
            border: "none",
            background: "var(--color-ink)",
            color: "var(--color-gold)",
            fontFamily: "var(--font-sans)",
            fontSize: 11.5,
            fontWeight: 600,
            cursor: "pointer",
          }}
        >
          {saveLabel}
        </button>
      </div>
    </div>
  );
}

function MethodButton({
  label,
  hint,
  active,
  onClick,
}: {
  label: string;
  hint: string;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      style={{
        display: "flex",
        flexDirection: "column",
        alignItems: "flex-start",
        gap: 2,
        padding: "8px 10px",
        borderRadius: "var(--radius-sm)",
        border: "1px solid " + (active ? "var(--color-gold)" : "var(--border-default)"),
        background: active ? "rgba(194,163,112,0.10)" : "transparent",
        cursor: "pointer",
        textAlign: "left",
      }}
    >
      <span
        style={{
          fontFamily: "var(--font-sans)",
          fontSize: 11.5,
          fontWeight: 600,
          color: active ? "var(--color-gold)" : "var(--text-68)",
        }}
      >
        {label}
      </span>
      <span style={{ fontFamily: "var(--font-mono)", fontSize: 9, color: "var(--text-40)" }}>
        {hint}
      </span>
    </button>
  );
}
