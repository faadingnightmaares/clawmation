import { useState } from "react";
import {
  IconArrowsMove,
  IconCheck,
  IconColorPicker,
  IconCrop,
  IconDeviceFloppy,
  IconFlask,
  IconFocus2,
  IconKeyboard,
  IconLetterT,
  IconLoader2,
  IconPointer,
  type Icon,
} from "@tabler/icons-react";

import {
  addTemplateImage,
  guardPickColor,
  guardPickRegion,
  guardTest,
  saveTemplateUpload,
  surgicalCapture,
  type Guard,
} from "@/api";
import { hsvToCss } from "@/format";
import { pxRectToPct } from "@/lib/screen";
import { notify } from "@/lib/toast";
import { cn } from "@/lib/utils";
import {
  appendImageCandidate,
  imageCandidates,
  removeImageCandidate,
  splitImageCandidates,
} from "@/lib/visionImages";
import {
  REPEAT_STOPS,
  guardFromDraft,
  isAnywhere,
  isTriggerReady,
  lookOf,
  methodFor,
  repeatStopOf,
  type LookKind,
  type TriggerDraft,
} from "@/lib/triggers";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { ImageCandidateGallery } from "@/components/vision/ImageCandidateGallery";

const CANCELLED = "cancelled";
const asDataUri = (thumb: string, mime = "image/png") =>
  thumb.startsWith("data:") ? thumb : `data:${mime};base64,${thumb}`;

interface WatchTriggerEditorProps {
  initial: TriggerDraft;
  saveLabel?: string;
  onSave: (guard: Guard) => void | Promise<void>;
  onCancel: () => void;
}

export function WatchTriggerEditor({
  initial,
  saveLabel = "Save trigger",
  onSave,
  onCancel,
}: WatchTriggerEditorProps) {
  const [draft, setDraft] = useState(initial);
  const [namedByHand, setNamedByHand] = useState(Boolean(initial.name));
  const [thumbs, setThumbs] = useState<(string | null)[]>([]);
  const [imageDragOver, setImageDragOver] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  const [testing, setTesting] = useState(false);
  const [saving, setSaving] = useState(false);

  const patch = (next: Partial<TriggerDraft>) =>
    setDraft((current) => ({ ...current, ...next }));
  const look = lookOf(draft.method);
  const ready = isTriggerReady(draft);
  const preview = draft._preview;
  const candidates = imageCandidates(draft.template_path, draft.template_paths);

  const run = async (tag: string, action: () => Promise<void>) => {
    setBusy(tag);
    try {
      await action();
    } catch (error) {
      notify("error", String(error));
    } finally {
      setBusy(null);
    }
  };

  const appendImage = (
    path: string,
    thumb: string | undefined,
    clickGeometry?: {
      offset?: number[];
      click_line?: number[];
      click_lines?: number[][];
    },
  ) => {
    const appended = appendImageCandidate(candidates, path);
    if (appended.full) {
      notify("warning", "A Watch trigger can use up to 8 images.");
      return false;
    }
    if (!appended.added) {
      notify("info", "That image is already included.");
      return false;
    }
    const split = splitImageCandidates(appended.candidates);
    patch({
      method: methodFor("image"),
      template_path: split.primary,
      template_paths: split.alternatives,
      ...(candidates.length === 0 && clickGeometry
        ? {
            click_offset: clickGeometry.offset ?? [],
            click_line: clickGeometry.click_line ?? [],
            click_lines: clickGeometry.click_lines ?? [],
          }
        : {}),
      _preview: null,
    });
    setThumbs((current) => [
      ...candidates.map((_, index) => current[index] ?? null),
      thumb ? asDataUri(thumb) : null,
    ]);
    return true;
  };

  const captureImage = () =>
    run("image", async () => {
      const result = await surgicalCapture();
      if (result.error === CANCELLED) return;
      if (result.ok && result.path) {
        const added = appendImage(result.path, result.thumb, {
          offset: (result as { offset?: number[] }).offset,
          click_line: result.click_line,
          click_lines: result.click_lines,
        });
        if (added) {
          notify(
            "success",
            candidates.length === 0
              ? "Picture and click point saved."
              : "Another visual state was added.",
          );
        }
      } else if (result.error) {
        notify("error", result.error);
      }
    });

  const chooseImage = () =>
    run("file", async () => {
      const result = await addTemplateImage();
      if (result.error === CANCELLED) return;
      if (result.ok && result.path) {
        if (appendImage(result.path, result.thumb)) {
          notify("success", "Image state added.");
        }
      } else if (result.error) {
        notify("error", result.error);
      }
    });

  const uploadDroppedImage = (file: File) =>
    run("drop", async () => {
      if (!file.type.startsWith("image/")) {
        notify("error", "Drop a PNG, JPG, BMP, or WebP image.");
        return;
      }
      if (file.size > 20 * 1024 * 1024) {
        notify("error", "The image must be 20 MB or smaller.");
        return;
      }
      const dataUrl = await new Promise<string>((resolve, reject) => {
        const reader = new FileReader();
        reader.onload = () => resolve(String(reader.result));
        reader.onerror = () => reject(reader.error || new Error("Could not read image"));
        reader.readAsDataURL(file);
      });
      const result = await saveTemplateUpload(
        dataUrl.slice(dataUrl.indexOf(",") + 1),
      );
      if (result.ok && result.path) {
        if (appendImage(result.path, result.thumb)) {
          notify("success", "Dropped image state added.");
        }
      } else if (result.error) {
        notify("error", result.error);
      }
    });

  const removeImage = (index: number) => {
    const remaining = removeImageCandidate(candidates, index);
    const split = splitImageCandidates(remaining);
    patch({
      template_path: split.primary,
      template_paths: split.alternatives,
      _preview: null,
    });
    setThumbs((current) =>
      current.filter((_, candidateIndex) => candidateIndex !== index),
    );
  };

  const pickColor = () =>
    run("color", async () => {
      const result = await guardPickColor();
      if (result.ok && result.hsv_low && result.hsv_high) {
        patch({
          method: methodFor("color"),
          hsv_low: result.hsv_low,
          hsv_high: result.hsv_high,
          _preview: null,
        });
        notify("success", "Colour picked.");
      } else if (result.error && result.error !== CANCELLED) {
        notify("error", result.error);
      }
    });

  const pickRegion = () =>
    run("region", async () => {
      const result = await guardPickRegion();
      if (result.ok) {
        patch({
          region:
            result.region ??
            pxRectToPct(
              result.x ?? 0,
              result.y ?? 0,
              result.w ?? 0,
              result.h ?? 0,
            ),
          _preview: null,
        });
        notify("success", "Watch area set.");
      } else if (result.error && result.error !== CANCELLED) {
        notify("error", result.error);
      }
    });

  const test = async () => {
    setTesting(true);
    try {
      const result = await guardTest(
        guardFromDraft(draft, { forTest: true }),
      );
      patch({
        _preview: {
          src: result.preview
            ? `data:${result.preview_mime || "image/png"};base64,${result.preview}`
            : "",
          ok: Boolean(result.ok),
          msg: result.message,
        },
      });
      notify(
        result.ok ? "success" : "info",
        result.ok
          ? "Found it on screen."
          : result.message || "Couldn’t see it right now.",
      );
    } catch (error) {
      notify("error", `Test failed: ${String(error)}`);
    } finally {
      setTesting(false);
    }
  };

  const save = async () => {
    setSaving(true);
    try {
      await onSave(guardFromDraft(draft));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="grid min-w-0 items-start gap-5 lg:grid-cols-[minmax(0,1fr)_320px]">
      <div className="min-w-0 overflow-hidden rounded-xl border border-border bg-card">
        <Preview
          draft={draft}
          look={look}
          thumb={thumbs[0] ?? null}
          preview={preview}
        />

        <div className="divide-y divide-border/80 px-5">
          <EditorStep number={1} title="Trigger name">
            <Input
              value={draft.name}
              onChange={(event) => {
                setNamedByHand(true);
                patch({ name: event.target.value });
              }}
              placeholder="Name this trigger"
              aria-label="Trigger name"
              autoComplete="off"
              className="h-10"
            />
          </EditorStep>

          <EditorStep
            number={2}
            title="Detect"
            hint="What should Clawmation look for?"
          >
            <div className="grid gap-2">
              <DetectionChoice
                icon={IconFocus2}
                title={
                  candidates.length > 0
                    ? "Add another visual state"
                    : "Snap it and mark where to click"
                }
                description={
                  candidates.length > 0
                    ? "Capture its hovered, highlighted, or changed appearance."
                    : "Draw around the button, then mark the exact spot to press."
                }
                selected={look === "image"}
                busy={busy === "image"}
                onClick={captureImage}
              />
              <DetectionChoice
                icon={IconColorPicker}
                title="Pick its colour"
                description="Click the thing on screen and learn its colour."
                selected={look === "color"}
                busy={busy === "color"}
                swatch={
                  ready && look === "color"
                    ? hsvToCss(draft.hsv_low, draft.hsv_high)
                    : undefined
                }
                onClick={pickColor}
              />
              <DetectionChoice
                icon={IconLetterT}
                title="Watch for words"
                description="Type what shows up, like Reconnect."
                selected={look === "text"}
                onClick={() =>
                  patch({ method: methodFor("text"), _preview: null })
                }
              />
            </div>

            {look === "text" && (
              <Input
                value={draft.ocr_text}
                onChange={(event) => {
                  const ocr_text = event.target.value;
                  patch(
                    namedByHand
                      ? { ocr_text, _preview: null }
                      : {
                          ocr_text,
                          name: ocr_text,
                          _preview: null,
                        },
                  );
                }}
                placeholder="Words to watch for"
                autoComplete="off"
                className="mt-2 h-10"
              />
            )}

            {look === "image" && (
              <div className="mt-3">
                <ImageCandidateGallery
                  candidates={candidates}
                  thumbnails={thumbs}
                  busy={busy === "image" || busy === "file" || busy === "drop"}
                  dragOver={imageDragOver}
                  magicLabel={
                    candidates.length > 0
                      ? "Capture another state"
                      : "Capture from screen"
                  }
                  onDragOverChange={setImageDragOver}
                  onChoose={chooseImage}
                  onMagicSelect={captureImage}
                  onRemove={removeImage}
                  onDrop={(file) => void uploadDroppedImage(file)}
                />
              </div>
            )}
          </EditorStep>

          <EditorStep
            number={3}
            title="Action"
            hint="What should happen next?"
          >
            <div className="grid gap-2 sm:grid-cols-3">
              <ActionChoice
                icon={IconPointer}
                label="Click it"
                description="Left mouse click"
                active={draft.action === "click"}
                onClick={() => patch({ action: "click" })}
              />
              <ActionChoice
                icon={IconKeyboard}
                label="Press a key"
                description="Type a keyboard key"
                active={draft.action === "key"}
                onClick={() => patch({ action: "key" })}
              />
              <ActionChoice
                icon={IconArrowsMove}
                label="Nudge the mouse"
                description="Move the mouse slightly"
                active={draft.action === "nudge"}
                onClick={() => patch({ action: "nudge" })}
              />
            </div>
            {draft.action === "key" && (
              <Input
                value={draft.key}
                onChange={(event) => patch({ key: event.target.value })}
                placeholder="space, enter, f"
                aria-label="Key to press"
                autoComplete="off"
                className="mt-2 h-10 font-mono"
              />
            )}
          </EditorStep>

          <EditorStep
            number={4}
            title="Fine tuning"
            hint="Adjust how it runs."
          >
            <div className="grid gap-3">
              <div className="grid items-center gap-2 sm:grid-cols-[150px_1fr]">
                <p className="text-sm font-medium text-foreground">Where to watch</p>
                <div className="grid grid-cols-2 overflow-hidden rounded-lg border border-border">
                  <button
                    type="button"
                    onClick={() =>
                      patch({
                        region: [0, 0, 100, 100],
                        _preview: null,
                      })
                    }
                    className={cn(
                      "flex h-10 items-center justify-center gap-2 border-r border-border px-3 text-xs font-medium transition-colors",
                      isAnywhere(draft.region)
                        ? "bg-primary/10 text-primary"
                        : "text-muted-foreground hover:text-foreground",
                    )}
                  >
                    Anywhere
                  </button>
                  <button
                    type="button"
                    onClick={pickRegion}
                    disabled={busy === "region"}
                    className={cn(
                      "flex h-10 items-center justify-center gap-2 px-3 text-xs font-medium transition-colors disabled:opacity-60",
                      !isAnywhere(draft.region)
                        ? "bg-primary/10 text-primary"
                        : "text-muted-foreground hover:text-foreground",
                    )}
                  >
                    {busy === "region" ? (
                      <IconLoader2 className="size-4 animate-spin" />
                    ) : (
                      <IconCrop className="size-4" />
                    )}
                    One area
                  </button>
                </div>
              </div>

              <div className="grid items-center gap-2 sm:grid-cols-[150px_1fr]">
                <p className="text-sm font-medium text-foreground">Act again</p>
                <Select
                  value={String(repeatStopOf(draft.cooldown))}
                  onValueChange={(value) =>
                    patch({ cooldown: Number(value) })
                  }
                >
                  <SelectTrigger className="h-10 w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {REPEAT_STOPS.map((stop) => (
                      <SelectItem
                        key={stop.seconds}
                        value={String(stop.seconds)}
                      >
                        {stop.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            </div>
          </EditorStep>
        </div>
      </div>

      <aside className="grid min-w-0 gap-4 lg:sticky lg:top-5">
        <section className="rounded-xl border border-border bg-card p-4">
          <h2 className="px-1 text-base font-semibold text-foreground">
            Actions
          </h2>
          <div className="mt-4 grid gap-2">
            <Button
              variant="outline"
              onClick={test}
              disabled={testing || !ready}
              aria-label="Test trigger"
              className="h-14 justify-start rounded-lg px-4"
            >
              {testing ? (
                <IconLoader2 className="size-5 animate-spin" />
              ) : (
                <IconFlask className="size-5" />
              )}
              <span className="text-left">
                <span className="block text-sm font-semibold">Test trigger</span>
                <span className="block text-[11px] font-normal text-muted-foreground">
                  Run a quick test
                </span>
              </span>
            </Button>
            <Button
              onClick={save}
              disabled={!ready || saving}
              aria-label={saveLabel}
              className="h-14 justify-start rounded-lg px-4"
            >
              {saving ? (
                <IconLoader2 className="size-5 animate-spin" />
              ) : (
                <IconDeviceFloppy className="size-5" />
              )}
              <span className="text-left">
                <span className="block text-sm font-semibold">{saveLabel}</span>
                <span className="block text-[11px] font-normal opacity-80">
                  Save and return to Watch
                </span>
              </span>
            </Button>
            <Button
              variant="ghost"
              size="sm"
              onClick={onCancel}
              disabled={saving}
              className="justify-start text-muted-foreground"
            >
              Cancel
            </Button>
          </div>
        </section>

      </aside>
    </div>
  );
}

function Preview({
  draft,
  look,
  thumb,
  preview,
}: {
  draft: TriggerDraft;
  look: LookKind;
  thumb: string | null;
  preview: TriggerDraft["_preview"];
}) {
  const image = preview?.src || thumb;
  const status = preview
    ? preview.ok
      ? "Visible now"
      : "Not visible"
    : "Live preview";

  return (
    <section className="border-b border-border bg-muted/15 p-5">
      <div className="mb-3 flex items-center justify-between gap-3">
        <h2 className="text-sm font-semibold text-foreground">Preview</h2>
        <div className="flex items-center gap-2 text-xs text-muted-foreground">
          <span
            className={cn(
              "size-2 rounded-full",
              preview ? (preview.ok ? "bg-success" : "bg-destructive") : "bg-success",
            )}
          />
          {status}
        </div>
      </div>

      <div className="relative mx-auto flex min-h-[180px] max-w-[660px] items-center justify-center overflow-hidden rounded-xl border border-border bg-background shadow-[0_16px_38px_rgba(50,35,18,0.08)]">
        {image ? (
          <img
            src={image}
            alt="Trigger preview"
            className="max-h-[260px] w-full object-contain"
          />
        ) : look === "color" && isTriggerReady(draft) ? (
          <div className="grid place-items-center gap-3 text-center">
            <span
              className="size-20 rounded-2xl border border-border shadow-inner"
              style={{
                background: hsvToCss(draft.hsv_low, draft.hsv_high),
              }}
            />
            <p className="text-sm font-medium text-foreground">
              Watching for this colour
            </p>
          </div>
        ) : look === "text" && draft.ocr_text.trim() ? (
          <div className="max-w-md rounded-lg border border-dashed border-primary/60 bg-primary/5 px-8 py-5 text-center">
            <p className="text-xl font-semibold text-foreground">
              {draft.ocr_text}
            </p>
            <p className="mt-2 text-xs text-muted-foreground">
              Clawmation will watch for these words.
            </p>
          </div>
        ) : (
          <div className="w-[76%]">
            <div className="mb-5 grid grid-cols-[1fr_0.7fr] gap-5">
              <div className="space-y-3">
                <div className="h-3 w-4/5 rounded bg-muted" />
                <div className="h-3 w-3/5 rounded bg-muted/80" />
                <div className="h-3 w-2/5 rounded bg-muted/60" />
              </div>
              <div className="h-20 rounded-lg bg-muted/60" />
            </div>
            <div className="relative h-14 w-2/5 rounded-lg border border-dashed border-primary bg-primary/5">
              <span className="absolute -bottom-3 -right-3 grid size-7 place-items-center rounded-full border border-primary bg-background text-primary">
                <IconFocus2 className="size-4" />
              </span>
            </div>
          </div>
        )}
      </div>
      <p className="mt-3 text-center text-xs text-muted-foreground">
        Clawmation will watch for the thing you mark here.
      </p>
    </section>
  );
}

function EditorStep({
  number,
  title,
  hint,
  children,
}: {
  number: number;
  title: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <section className="grid gap-4 py-4 md:grid-cols-[190px_minmax(0,1fr)] xl:grid-cols-[220px_minmax(0,1fr)]">
      <div className="flex items-start gap-3">
        <span className="grid size-7 shrink-0 place-items-center rounded-full bg-primary/10 text-xs font-semibold text-primary">
          {number}
        </span>
        <div>
          <h2 className="text-sm font-semibold text-foreground">{title}</h2>
          {hint && (
            <p className="mt-1 text-xs leading-4 text-muted-foreground">{hint}</p>
          )}
        </div>
      </div>
      <div className="min-w-0">{children}</div>
    </section>
  );
}

function DetectionChoice({
  icon: IconComponent,
  title,
  description,
  selected,
  busy,
  swatch,
  onClick,
}: {
  icon: Icon;
  title: string;
  description: string;
  selected: boolean;
  busy?: boolean;
  swatch?: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={busy}
      className={cn(
        "flex min-h-12 w-full items-center gap-3 rounded-lg border px-3 py-2 text-left transition-colors disabled:opacity-60",
        selected
          ? "border-primary bg-primary/[0.045]"
          : "border-border hover:border-primary/45",
      )}
    >
      <span className="grid size-9 shrink-0 place-items-center rounded-lg bg-muted text-muted-foreground">
        {busy ? (
          <IconLoader2 className="size-5 animate-spin" />
        ) : swatch ? (
          <span
            className="size-5 rounded-md border border-border"
            style={{ background: swatch }}
          />
        ) : (
          <IconComponent className="size-5" strokeWidth={1.7} />
        )}
      </span>
      <span className="min-w-0">
        <span className="block text-sm font-semibold text-foreground">{title}</span>
        <span className="block truncate text-xs text-muted-foreground">
          {description}
        </span>
      </span>
      {selected && <IconCheck className="ml-auto size-4 shrink-0 text-primary" />}
    </button>
  );
}

function ActionChoice({
  icon: IconComponent,
  label,
  description,
  active,
  onClick,
}: {
  icon: Icon;
  label: string;
  description: string;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "flex min-h-14 items-center gap-3 rounded-lg border px-3 text-left transition-colors",
        active
          ? "border-primary bg-primary/[0.045]"
          : "border-border hover:border-primary/45",
      )}
    >
      <IconComponent className="size-5 shrink-0" strokeWidth={1.7} />
      <span className="min-w-0">
        <span className="block text-sm font-semibold text-foreground">{label}</span>
        <span className="block truncate text-[11px] text-muted-foreground">
          {description}
        </span>
      </span>
    </button>
  );
}
