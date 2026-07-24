import type { CSSProperties } from "react";
import { Camera, Crop, Crosshair, Eye, Eyedropper, HandGrabbing, Plus, Trash, X } from "@phosphor-icons/react";

import { captureTemplate, guardPickColor, guardPickRegion, surgicalCapture, type CheckpointItem } from "./api";
import { hsvToCss } from "./format";
import type { ToastKind } from "./components/Toasts";

// The vision-checkpoint editor — a faithful port of the panel the Python UI
// renders inside each macro card (`item.ve.*`, index.html:944-1039) plus its
// driving logic (`_freshVisionDraft` / `visionPick*` / `buildVisionEditorVals`,
// index.html:3417-3644). A checkpoint pauses playback mid-macro to wait for (or
// hold-and-follow) an image or colour, then clicks / presses a key. Distinct
// from the guard editor: mode wait_for / hold_follow, release-when, no OCR, no
// orchestration sequence. The native Pick / Capture / Surgical / Watch-area
// flows call the picker commands directly and merge their results back through
// `onChange`, exactly as the guard editor does — only the toast strings differ
// (the vision "Color: " vs the guard's "Color set: ").

/** The editor's working copy of the checkpoint being composed. Mirrors the
 * Python `visionDraft` (`_freshVisionDraft`, index.html:3417). */
export interface CheckpointDraft {
  mode: string; // 'wait_for' | 'hold_follow'
  method: string; // 'color' | 'template'
  hsv_low: number[] | null;
  hsv_high: number[] | null;
  colorHex: string | null;
  template: string | null;
  templateThumb: string | null;
  click_offset: number[] | null; // [x, y] exact pixel to hit (from surgical capture)
  click_line: number[] | null; // [sx,sy,ex,ey] drawn sweep line
  click_lines: number[][] | null; // [[sx,sy,ex,ey], …] all drawn strokes
  region: number[] | null; // [x1,y1,x2,y2] percentages
  action: string; // 'click' | 'key' | 'none'
  key: string;
  releaseWhen: string; // hold_follow: 'lost' | 'found'
  timeout: number | string;
}

/** The typed view of a stored checkpoint's config for the list display. Every
 * checkpoint `add_checkpoint` writes carries these fields, so they read as
 * required here (matching the Python source's unguarded `cp.checkpoint.*`);
 * extra keys pass through untyped. */
interface CheckpointConfig {
  mode: string;
  method: string;
  action: string;
  key: string;
  timeout: number;
  [key: string]: unknown;
}

export interface CheckpointEditorProps {
  draft: CheckpointDraft;
  checkpoints: CheckpointItem[];
  /** Merge a partial patch into the draft — the `updateVisionDraft` analogue. */
  onChange: (patch: Partial<CheckpointDraft>) => void;
  /** Validate + persist the draft as a new checkpoint (host-owned). */
  onAdd: () => void;
  onDelete: (index: number) => void;
  onClose: () => void;
  push: (kind: ToastKind, text: string) => void;
}

/** A fresh, empty checkpoint draft. Mirrors Python `_freshVisionDraft`. */
export function freshCheckpointDraft(): CheckpointDraft {
  return {
    mode: "wait_for",
    method: "color",
    hsv_low: null,
    hsv_high: null,
    colorHex: null,
    template: null,
    templateThumb: null,
    click_offset: null,
    click_line: null,
    click_lines: null,
    region: null,
    action: "click",
    key: "",
    releaseWhen: "lost",
    timeout: 10,
  };
}

const monoMeta: CSSProperties = {
  fontFamily: "var(--font-mono)",
  fontSize: 10,
  color: "var(--text-40)",
};

/** The border / background / foreground triple for a segmented toggle button,
 * gold when active. */
function seg(active: boolean): CSSProperties {
  return {
    border: "1px solid " + (active ? "var(--color-gold)" : "var(--border-default)"),
    background: active ? "rgba(194,163,112,0.08)" : "transparent",
    color: active ? "var(--color-gold)" : "var(--text-50)",
  };
}

const pickerBtn: CSSProperties = {
  height: 30,
  padding: "0 12px",
  borderRadius: "var(--radius-sm)",
  border: "1px solid var(--border-strong)",
  background: "transparent",
  color: "var(--text-68)",
  fontFamily: "var(--font-sans)",
  fontSize: 11,
  fontWeight: 600,
  cursor: "pointer",
  display: "flex",
  alignItems: "center",
  gap: 6,
};

const smallSelect: CSSProperties = {
  height: 30,
  border: "1px solid var(--border-faint)",
  borderRadius: "var(--radius-sm)",
  padding: "0 8px",
  fontFamily: "var(--font-sans)",
  fontSize: 11,
  color: "var(--color-ink)",
  background: "var(--surface-card)",
  cursor: "pointer",
};

export function CheckpointEditor({
  draft,
  checkpoints,
  onChange,
  onAdd,
  onDelete,
  onClose,
  push,
}: CheckpointEditorProps) {
  const isHold = draft.mode === "hold_follow";
  const isColor = draft.method === "color";
  const isTemplate = draft.method === "template";
  const isActionKey = draft.action === "key";

  const hasColor = !!draft.hsv_low;
  const colorSwatch = hsvToCss(draft.hsv_low ?? undefined, draft.hsv_high ?? undefined);
  const hasTemplate = !!draft.template;

  // The surgical-capture "hit target" chip. A multi-stroke capture reads as "N
  // strokes"; a single 4-element line reads as a point or an arrow between its
  // endpoints; a bare offset reads as a point.
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
        onChange({ hsv_low: res.hsv_low, hsv_high: res.hsv_high, colorHex: res.hex });
        push("success", "Color: " + res.hex);
      }
    } catch {
      push("error", "Color pick failed");
    }
  };

  const onCaptureTemplate = async () => {
    try {
      const res = await captureTemplate();
      if (res && res.ok) {
        onChange({ template: res.path, templateThumb: "data:image/png;base64," + res.thumb });
        push("success", "Template captured (" + res.w + "×" + res.h + ")");
      }
    } catch {
      push("error", "Capture failed");
    }
  };

  const onSurgical = async () => {
    try {
      const res = await surgicalCapture();
      if (res && res.ok) {
        onChange({
          method: "template",
          template: res.path,
          templateThumb: "data:image/png;base64," + res.thumb,
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

  const onPickRegion = async () => {
    try {
      const res = await guardPickRegion();
      if (res && res.ok) {
        // Convert pixels → percentage of screen (2560x1440).
        const sw = 2560;
        const sh = 1440;
        const region = [
          ((res.x ?? 0) / sw) * 100,
          ((res.y ?? 0) / sh) * 100,
          (((res.x ?? 0) + (res.w ?? 0)) / sw) * 100,
          (((res.y ?? 0) + (res.h ?? 0)) / sh) * 100,
        ];
        onChange({ region });
        push("success", "Watch area: " + res.w + "×" + res.h);
      }
    } catch {
      push("error", "Region pick failed");
    }
  };

  return (
    <div
      style={{
        borderTop: "1px solid var(--border-faint)",
        background: "rgba(194,163,112,0.03)",
        padding: "14px 18px 16px",
      }}
    >
      {/* Header */}
      <div
        style={{
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
          marginBottom: 12,
        }}
      >
        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <span
            style={{
              fontFamily: "var(--font-mono)",
              fontSize: 10,
              letterSpacing: "0.08em",
              textTransform: "uppercase",
              color: "var(--text-50)",
            }}
          >
            Vision
          </span>
          <span style={{ fontFamily: "var(--font-sans)", fontSize: 11, color: "var(--text-40)" }}>
            detect an image or color mid-macro, then act. Perfect for timing bars, popups, and
            moving targets.
          </span>
        </div>
        <button
          type="button"
          onClick={onClose}
          title="Close vision editor"
          style={{
            width: 28,
            height: 28,
            border: "none",
            background: "transparent",
            color: "var(--text-40)",
            cursor: "pointer",
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
          }}
        >
          <X size={13} />
        </button>
      </div>

      {/* Existing checkpoints */}
      {checkpoints.length > 0 && (
        <div style={{ display: "flex", flexDirection: "column", gap: 4, marginBottom: 14 }}>
          {checkpoints.map((cp) => {
            const c = (cp.checkpoint || {}) as CheckpointConfig;
            const isHoldRow = c.mode === "hold_follow";
            const label = isHoldRow ? "Hold & Follow" : "Wait & Act";
            const detail =
              (c.method === "color" ? "color" : "image") +
              " · " +
              (c.action === "click"
                ? "click"
                : c.action === "key"
                  ? "press " + c.key
                  : "no action") +
              " · timeout " +
              c.timeout +
              "s";
            return (
              <div
                key={cp.index}
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: 10,
                  padding: "8px 12px",
                  borderRadius: "var(--radius-md)",
                  border: "1px solid var(--border-faint)",
                  background: "var(--surface-card)",
                }}
              >
                {isHoldRow ? (
                  <HandGrabbing size={15} color="var(--color-gold)" style={{ flexShrink: 0 }} />
                ) : (
                  <Eye size={15} color="var(--color-gold)" style={{ flexShrink: 0 }} />
                )}
                <div style={{ flex: 1, minWidth: 0 }}>
                  <div
                    style={{
                      fontFamily: "var(--font-sans)",
                      fontSize: 12,
                      fontWeight: 600,
                      color: "var(--color-ink)",
                    }}
                  >
                    {label}
                  </div>
                  <div style={{ fontFamily: "var(--font-mono)", fontSize: 10, color: "var(--text-40)" }}>
                    {detail}
                  </div>
                </div>
                <button
                  type="button"
                  onClick={() => onDelete(cp.index)}
                  title="Remove checkpoint"
                  style={{
                    width: 24,
                    height: 24,
                    border: "none",
                    background: "transparent",
                    color: "var(--text-40)",
                    cursor: "pointer",
                    display: "flex",
                    alignItems: "center",
                    justifyContent: "center",
                  }}
                >
                  <Trash size={12} />
                </button>
              </div>
            );
          })}
        </div>
      )}

      {/* Add new checkpoint form */}
      <div
        style={{
          border: "1px solid var(--border-faint)",
          borderRadius: "var(--radius-lg)",
          padding: 14,
          background: "var(--surface-card)",
        }}
      >
        <div
          style={{
            fontFamily: "var(--font-mono)",
            fontSize: 10,
            letterSpacing: "0.08em",
            textTransform: "uppercase",
            color: "var(--text-50)",
            marginBottom: 10,
          }}
        >
          Add checkpoint
        </div>

        {/* Mode */}
        <div style={{ display: "flex", gap: 8, marginBottom: 12 }}>
          <button
            type="button"
            onClick={() => onChange({ mode: "wait_for" })}
            style={{
              flex: 1,
              height: 34,
              borderRadius: "var(--radius-sm)",
              fontFamily: "var(--font-sans)",
              fontSize: 11.5,
              fontWeight: 600,
              cursor: "pointer",
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
              gap: 6,
              ...seg(!isHold),
            }}
          >
            <Eye size={14} /> Wait &amp; Act
          </button>
          <button
            type="button"
            onClick={() => onChange({ mode: "hold_follow" })}
            style={{
              flex: 1,
              height: 34,
              borderRadius: "var(--radius-sm)",
              fontFamily: "var(--font-sans)",
              fontSize: 11.5,
              fontWeight: 600,
              cursor: "pointer",
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
              gap: 6,
              ...seg(isHold),
            }}
          >
            <HandGrabbing size={14} /> Hold &amp; Follow
          </button>
        </div>

        {/* Method */}
        <div style={{ display: "flex", gap: 8, marginBottom: 12 }}>
          <button
            type="button"
            onClick={() => onChange({ method: "color" })}
            style={{
              flex: 1,
              height: 30,
              borderRadius: "var(--radius-sm)",
              fontFamily: "var(--font-sans)",
              fontSize: 11,
              fontWeight: 600,
              cursor: "pointer",
              ...seg(isColor),
            }}
          >
            Color
          </button>
          <button
            type="button"
            onClick={() => onChange({ method: "template" })}
            style={{
              flex: 1,
              height: 30,
              borderRadius: "var(--radius-sm)",
              fontFamily: "var(--font-sans)",
              fontSize: 11,
              fontWeight: 600,
              cursor: "pointer",
              ...seg(isTemplate),
            }}
          >
            Image
          </button>
        </div>

        {/* Pickers row */}
        <div style={{ display: "flex", gap: 8, marginBottom: 12, flexWrap: "wrap" }}>
          {isColor && (
            <>
              <button type="button" onClick={onPickColor} style={pickerBtn}>
                <Eyedropper size={13} /> Pick Color
              </button>
              {hasColor && (
                <span
                  style={{
                    display: "inline-flex",
                    alignItems: "center",
                    gap: 6,
                    height: 30,
                    padding: "0 10px",
                    borderRadius: "var(--radius-sm)",
                    border: "1px solid var(--border-faint)",
                    fontFamily: "var(--font-mono)",
                    fontSize: 10.5,
                    color: "var(--text-68)",
                  }}
                >
                  <span
                    style={{
                      width: 14,
                      height: 14,
                      borderRadius: 3,
                      background: colorSwatch,
                      border: "1px solid var(--border-default)",
                    }}
                  />
                  {draft.colorHex || ""}
                </span>
              )}
            </>
          )}
          {isTemplate && (
            <>
              <button type="button" onClick={onCaptureTemplate} style={pickerBtn}>
                <Camera size={13} /> Capture Image
              </button>
              <button
                type="button"
                onClick={onSurgical}
                title="Drag a tight box, then DRAW the exact pixels to hit (drag a line across the element)"
                style={{
                  ...pickerBtn,
                  border: "1px solid var(--color-gold)",
                  background: "rgba(194,163,112,0.08)",
                  color: "var(--color-gold)",
                }}
              >
                <Crosshair size={13} /> Surgical
              </button>
              {hasTemplate && draft.templateThumb && (
                <img
                  src={draft.templateThumb}
                  style={{
                    height: 30,
                    borderRadius: "var(--radius-sm)",
                    border: "1px solid var(--border-faint)",
                  }}
                />
              )}
              {hasClickOffset && (
                <span
                  style={{
                    display: "inline-flex",
                    alignItems: "center",
                    gap: 5,
                    height: 30,
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
                  hit @ {clickOffsetLabel}
                </span>
              )}
            </>
          )}
          <button type="button" onClick={onPickRegion} style={pickerBtn}>
            <Crop size={13} /> Watch Area
          </button>
        </div>

        {/* Action + hold options */}
        <div
          style={{
            display: "flex",
            gap: 8,
            marginBottom: 12,
            alignItems: "center",
            flexWrap: "wrap",
          }}
        >
          <span style={monoMeta}>then</span>
          <select
            value={draft.action}
            onChange={(e) => onChange({ action: e.target.value })}
            style={smallSelect}
          >
            <option value="click">Click it</option>
            <option value="key">Press key</option>
            <option value="none">Do nothing</option>
          </select>
          {isActionKey && (
            <input
              value={draft.key}
              onChange={(e) => onChange({ key: e.target.value })}
              placeholder="key"
              style={{
                width: 60,
                height: 30,
                border: "1px solid var(--border-faint)",
                borderRadius: "var(--radius-sm)",
                padding: "0 8px",
                fontFamily: "var(--font-mono)",
                fontSize: 11,
                color: "var(--color-ink)",
                background: "var(--surface-card)",
              }}
            />
          )}
          {isHold && (
            <>
              <span style={monoMeta}>release when</span>
              <select
                value={draft.releaseWhen}
                onChange={(e) => onChange({ releaseWhen: e.target.value })}
                style={smallSelect}
              >
                <option value="lost">target disappears</option>
                <option value="found">target appears</option>
              </select>
            </>
          )}
        </div>

        {/* Timeout + add */}
        <div
          style={{
            display: "flex",
            gap: 8,
            alignItems: "center",
            justifyContent: "space-between",
          }}
        >
          <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
            <span style={monoMeta}>timeout</span>
            <input
              value={draft.timeout}
              onChange={(e) => onChange({ timeout: e.target.value })}
              style={{
                width: 48,
                height: 30,
                border: "1px solid var(--border-faint)",
                borderRadius: "var(--radius-sm)",
                padding: "0 8px",
                fontFamily: "var(--font-mono)",
                fontSize: 11,
                color: "var(--color-ink)",
                background: "var(--surface-card)",
              }}
            />
            <span style={{ fontFamily: "var(--font-mono)", fontSize: 10, color: "var(--text-32)" }}>
              sec
            </span>
          </div>
          <button
            type="button"
            onClick={onAdd}
            style={{
              height: 32,
              padding: "0 18px",
              borderRadius: "var(--radius-sm)",
              border: "none",
              background: "var(--color-ink)",
              color: "var(--color-gold)",
              fontFamily: "var(--font-sans)",
              fontSize: 12,
              fontWeight: 600,
              cursor: "pointer",
              display: "flex",
              alignItems: "center",
              gap: 6,
            }}
          >
            <Plus size={14} /> Add Checkpoint
          </button>
        </div>
      </div>
    </div>
  );
}
