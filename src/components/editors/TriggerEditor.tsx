import { useState } from "react";
import {
  Camera,
  Check,
  Crop,
  FlaskConical,
  FolderOpen,
  ImageIcon,
  Keyboard,
  Loader2,
  MousePointerClick,
  Palette,
  Pipette,
  ScanLine,
  Type,
} from "lucide-react";

import {
  addTemplateImage,
  captureTemplate,
  guardPickColor,
  guardPickRegion,
  guardTest,
  surgicalCapture,
  type Guard,
} from "@/api";
import { hsvToCss } from "@/format";
import { useStaggerIn } from "@/lib/anime";
import { pxRectToPct } from "@/lib/screen";
import { notify } from "@/lib/toast";
import { cn } from "@/lib/utils";
import {
  ACCURACY_STOPS,
  RESUME_STOPS,
  accuracyOf,
  guardFromDraft,
  isAnywhere,
  isTriggerReady,
  lookOf,
  methodFor,
  resumeStopOf,
  type LookKind,
  type TriggerDraft,
} from "@/lib/triggers";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

export interface TriggerEditorProps {
  initial: TriggerDraft;
  /** Watch triggers get the surgical picker (pick the exact spot to click on the
   *  captured image); per-macro guards do not. */
  showSurgical?: boolean;
  saveLabel?: string;
  onSave: (guard: Guard) => void | Promise<void>;
  onCancel: () => void;
}

const CANCELLED = "cancelled";
const isDefaultColor = (lo: number[], hi: number[]) =>
  lo[0] === 0 && lo[1] === 0 && lo[2] === 0 && hi[0] === 179 && hi[1] === 255 && hi[2] === 255;
const asDataUri = (thumb: string, mime = "image/png") =>
  thumb.startsWith("data:") ? thumb : `data:${mime};base64,${thumb}`;

const LOOKS: { key: LookKind; icon: typeof Palette; title: string; blurb: string }[] = [
  { key: "color", icon: Palette, title: "A colour", blurb: "A button or badge with a distinct colour" },
  { key: "image", icon: ImageIcon, title: "A picture", blurb: "Snap the exact button so I can spot it again" },
  { key: "text", icon: Type, title: "Some words", blurb: "Text that appears, like “Reconnect”" },
];

export function TriggerEditor({ initial, showSurgical, saveLabel = "Save", onSave, onCancel }: TriggerEditorProps) {
  const [d, setD] = useState<TriggerDraft>(initial);
  // Until the name is typed in, a text trigger names itself after the words it
  // watches for — typing "Reconnect" twice was the most common thing to do here.
  const [namedByHand, setNamedByHand] = useState(!!initial.name);
  const [thumb, setThumb] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [testing, setTesting] = useState(false);
  const [saving, setSaving] = useState(false);
  const formRef = useStaggerIn<HTMLDivElement>();

  const patch = (p: Partial<TriggerDraft>) => setD((prev) => ({ ...prev, ...p }));
  const look = lookOf(d.method);
  const ready = isTriggerReady(d);
  const preview = d._preview;

  const run = async (tag: string, fn: () => Promise<void>) => {
    setBusy(tag);
    try {
      await fn();
    } catch (e) {
      notify("error", String(e));
    } finally {
      setBusy(null);
    }
  };

  const pickColor = () =>
    run("color", async () => {
      const r = await guardPickColor();
      if (r.ok && r.hsv_low && r.hsv_high) {
        patch({ hsv_low: r.hsv_low, hsv_high: r.hsv_high, _preview: null });
        notify("success", "Colour picked.");
      } else if (r.error && r.error !== CANCELLED) {
        notify("error", r.error);
      }
    });

  const captureImage = (surgical: boolean) =>
    run("image", async () => {
      const r = surgical ? await surgicalCapture() : await captureTemplate();
      if (r.error === CANCELLED) return;
      if (r.ok && r.path) {
        patch({
          template_path: r.path,
          click_offset: (r as { offset?: number[] }).offset ?? [],
          click_line: (r as { click_line?: number[] }).click_line ?? [],
          click_lines: (r as { click_lines?: number[][] }).click_lines ?? [],
          _preview: null,
        });
        if (r.thumb) setThumb(asDataUri(r.thumb));
        notify("success", surgical ? "Picture and click point saved." : "Picture saved.");
      } else if (r.error) {
        notify("error", r.error);
      }
    });

  const chooseFile = () =>
    run("image", async () => {
      const r = await addTemplateImage();
      if (r.error === CANCELLED) return;
      if (r.ok && r.path) {
        patch({ template_path: r.path, click_offset: [], click_line: [], click_lines: [], _preview: null });
        if (r.thumb) setThumb(asDataUri(r.thumb));
        notify("success", "Image chosen.");
      } else if (r.error) {
        notify("error", r.error);
      }
    });

  const pickRegion = () =>
    run("region", async () => {
      const r = await guardPickRegion();
      if (r.ok && r.x != null && r.y != null && r.w != null && r.h != null) {
        patch({ region: pxRectToPct(r.x, r.y, r.w, r.h), _preview: null });
        notify("success", "Area set.");
      } else if (r.error && r.error !== CANCELLED) {
        notify("error", r.error);
      }
    });

  const test = async () => {
    setTesting(true);
    try {
      const r = await guardTest(guardFromDraft(d, { forTest: true }));
      patch({
        _preview: {
          src: r.preview ? `data:${r.preview_mime || "image/png"};base64,${r.preview}` : "",
          ok: !!r.ok,
          msg: r.message,
        },
      });
      notify(r.ok ? "success" : "info", r.ok ? "Found it on screen." : r.message || "Couldn’t see it right now.");
    } catch (e) {
      notify("error", "Test failed: " + String(e));
    } finally {
      setTesting(false);
    }
  };

  const save = async () => {
    setSaving(true);
    try {
      await onSave(guardFromDraft(d));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div ref={formRef} className="flex-1 space-y-7 overflow-y-auto px-6 py-6">
        {/* Name */}
        <section className="space-y-2">
          <Label htmlFor="tr-name" className="text-sm font-medium text-foreground">
            Give it a name
          </Label>
          <Input
            id="tr-name"
            value={d.name}
            onChange={(e) => {
              setNamedByHand(true);
              patch({ name: e.target.value });
            }}
            placeholder="Reconnect button"
            autoComplete="off"
          />
        </section>

        {/* What to look for */}
        <section className="space-y-3">
          <Question title="What should I watch for?" hint="Pick whatever’s easiest to spot on screen." />
          <div className="grid grid-cols-3 gap-2">
            {LOOKS.map((l) => {
              const active = look === l.key;
              const Icon = l.icon;
              return (
                <button
                  key={l.key}
                  type="button"
                  onClick={() => patch({ method: methodFor(l.key), _preview: null })}
                  className={cn(
                    "flex flex-col items-start gap-2 rounded-lg border p-3 text-left transition-colors",
                    active
                      ? "border-primary bg-primary/10 text-foreground"
                      : "border-border bg-card text-muted-foreground hover:border-primary/40 hover:text-foreground",
                  )}
                >
                  <Icon className={cn("size-5", active ? "text-primary" : "text-muted-foreground")} />
                  <span className="text-sm font-medium text-foreground">{l.title}</span>
                  <span className="text-xs leading-snug">{l.blurb}</span>
                </button>
              );
            })}
          </div>

          {/* The look-specific picker */}
          <div className="rounded-lg border border-border bg-secondary/40 p-4">
            {look === "color" && (
              <div className="flex items-center gap-4">
                <div
                  className="size-12 shrink-0 rounded-md border border-border shadow-inner"
                  style={{ background: isDefaultColor(d.hsv_low, d.hsv_high) ? "transparent" : hsvToCss(d.hsv_low, d.hsv_high) }}
                >
                  {isDefaultColor(d.hsv_low, d.hsv_high) && (
                    <div className="flex size-full items-center justify-center text-muted-foreground">
                      <Palette className="size-5" />
                    </div>
                  )}
                </div>
                <div className="min-w-0 flex-1">
                  <p className="text-sm font-medium text-foreground">
                    {isDefaultColor(d.hsv_low, d.hsv_high) ? "No colour picked yet" : "Colour picked"}
                  </p>
                  <p className="text-xs text-muted-foreground">Click the thing on screen and I’ll learn its colour.</p>
                </div>
                <Button variant="secondary" size="sm" onClick={pickColor} disabled={busy === "color"}>
                  {busy === "color" ? <Loader2 className="size-4 animate-spin" /> : <Pipette className="size-4" />}
                  Pick on screen
                </Button>
              </div>
            )}

            {look === "image" && (
              <div className="space-y-3">
                <div className="flex items-center gap-4">
                  <div className="flex size-12 shrink-0 items-center justify-center overflow-hidden rounded-md border border-border bg-card">
                    {thumb ? (
                      <img src={thumb} alt="" className="size-full object-contain" />
                    ) : d.template_path ? (
                      <Check className="size-5 text-primary" />
                    ) : (
                      <ImageIcon className="size-5 text-muted-foreground" />
                    )}
                  </div>
                  <div className="min-w-0 flex-1">
                    <p className="text-sm font-medium text-foreground">
                      {d.template_path ? "Picture ready" : "No picture yet"}
                    </p>
                    <p className="text-xs text-muted-foreground">
                      {showSurgical
                        ? "Snap the button, then mark exactly where to click it."
                        : "Snap the button so I can recognise it later."}
                    </p>
                  </div>
                </div>
                {/* Where both capture paths exist, the surgical one leads: it comes
                    back with the picture *and* the click point, so the alternative
                    is one overlay pass short of a finished trigger. */}
                <div className="flex flex-wrap gap-2">
                  {showSurgical ? (
                    <>
                      <Button variant="secondary" size="sm" onClick={() => captureImage(true)} disabled={busy === "image"}>
                        {busy === "image" ? <Loader2 className="size-4 animate-spin" /> : <ScanLine className="size-4" />}
                        Snap &amp; mark click
                      </Button>
                      <Button variant="outline" size="sm" onClick={() => captureImage(false)} disabled={busy === "image"}>
                        <Camera className="size-4" />
                        Just the picture
                      </Button>
                    </>
                  ) : (
                    <Button variant="secondary" size="sm" onClick={() => captureImage(false)} disabled={busy === "image"}>
                      {busy === "image" ? <Loader2 className="size-4 animate-spin" /> : <Camera className="size-4" />}
                      Snap from screen
                    </Button>
                  )}
                  <Button variant="ghost" size="sm" onClick={chooseFile} disabled={busy === "image"}>
                    <FolderOpen className="size-4" />
                    Choose a file
                  </Button>
                </div>
              </div>
            )}

            {look === "text" && (
              <div className="space-y-2">
                <Label htmlFor="tr-text" className="text-sm font-medium text-foreground">
                  What words appear?
                </Label>
                <Input
                  id="tr-text"
                  value={d.ocr_text}
                  onChange={(e) => {
                    const ocr_text = e.target.value;
                    patch(
                      namedByHand
                        ? { ocr_text, _preview: null }
                        : { ocr_text, name: ocr_text, _preview: null },
                    );
                  }}
                  placeholder="Reconnect"
                  autoComplete="off"
                />
                <p className="text-xs text-muted-foreground">I’ll read the screen and look for this text.</p>
              </div>
            )}
          </div>
        </section>

        {/* Action */}
        <section className="space-y-3">
          <Question title="When it shows up, what should I do?" />
          <Segmented
            options={[
              { key: "click", label: "Click it", icon: MousePointerClick },
              { key: "key", label: "Press a key", icon: Keyboard },
            ]}
            value={d.action}
            onChange={(v) => patch({ action: v })}
          />
          {d.action === "key" && (
            <Input
              value={d.key}
              onChange={(e) => patch({ key: e.target.value })}
              placeholder="e.g. space, enter, f"
              autoComplete="off"
              className="max-w-xs font-mono"
            />
          )}
        </section>

        {/* Where */}
        <section className="space-y-3">
          <Question title="Where should I look?" hint="A smaller area is faster and avoids false alarms." />
          <div className="flex flex-wrap items-center gap-2">
            <Chip active={isAnywhere(d.region)} onClick={() => patch({ region: [0, 0, 100, 100], _preview: null })}>
              Anywhere on screen
            </Chip>
            <Chip active={!isAnywhere(d.region)} onClick={pickRegion} disabled={busy === "region"}>
              {busy === "region" ? (
                <Loader2 className="size-3.5 animate-spin" />
              ) : (
                <Crop className="size-3.5" />
              )}
              {isAnywhere(d.region) ? "Just one area" : "A chosen area"}
              {!isAnywhere(d.region) && <Check className="size-3.5 text-primary" />}
            </Chip>
            {!isAnywhere(d.region) && (
              <Button variant="ghost" size="xs" onClick={pickRegion} disabled={busy === "region"}>
                Change
              </Button>
            )}
          </div>
        </section>

        {/* Accuracy — only meaningful for a picture match */}
        {look === "image" && (
          <section className="space-y-3">
            <Question title="How close a match?" hint="Looser catches more; exact avoids mistakes." />
            <div className="flex flex-wrap gap-2">
              {ACCURACY_STOPS.map((s) => (
                <Chip
                  key={s.key}
                  active={accuracyOf(d.threshold) === s.key}
                  onClick={() => patch({ threshold: s.threshold, _preview: null })}
                  title={s.desc}
                >
                  {s.label}
                </Chip>
              ))}
            </div>
          </section>
        )}

        {/* After it acts */}
        <section className="space-y-3">
          <Question title="After it acts, how long should I wait?" hint="Give the game time to catch up before carrying on." />
          <div className="flex flex-wrap gap-2">
            {RESUME_STOPS.map((s) => (
              <Chip
                key={s.seconds}
                active={resumeStopOf(d.resume_delay) === s.seconds}
                onClick={() => patch({ resume_delay: s.seconds })}
                title={s.hint}
              >
                {s.label}
              </Chip>
            ))}
          </div>
        </section>

        {/* Test */}
        <section className="space-y-3 rounded-lg border border-dashed border-border p-4">
          <div className="flex items-center justify-between gap-3">
            <div>
              <p className="text-sm font-medium text-foreground">See if I can spot it</p>
              <p className="text-xs text-muted-foreground">Set the screen up first, then run a quick check.</p>
            </div>
            <Button variant="default" size="sm" onClick={test} disabled={testing || !ready}>
              {testing ? <Loader2 className="size-4 animate-spin" /> : <FlaskConical className="size-4" />}
              Test it
            </Button>
          </div>
          {preview && (
            <div className="space-y-2">
              <div
                className={cn(
                  "inline-flex items-center gap-1.5 rounded-full px-2.5 py-1 text-xs font-medium",
                  preview.ok ? "bg-primary/15 text-primary" : "bg-muted text-muted-foreground",
                )}
              >
                {preview.ok ? <Check className="size-3.5" /> : <FlaskConical className="size-3.5" />}
                {preview.ok ? "I can see it" : preview.msg || "Not visible right now"}
              </div>
              {preview.src && (
                <img
                  src={preview.src}
                  alt="What I saw"
                  className="max-h-56 w-full rounded-md border border-border object-contain"
                />
              )}
            </div>
          )}
        </section>
      </div>

      {/* Footer */}
      <div className="flex items-center justify-between gap-3 border-t border-border px-6 py-4">
        <p className="text-xs text-muted-foreground">
          {ready ? "Looks good to save." : "Tell me what to watch for first."}
        </p>
        <div className="flex gap-2">
          <Button variant="ghost" onClick={onCancel} disabled={saving}>
            Cancel
          </Button>
          <Button onClick={save} disabled={!ready || saving}>
            {saving ? <Loader2 className="size-4 animate-spin" /> : <Check className="size-4" />}
            {saveLabel}
          </Button>
        </div>
      </div>
    </div>
  );
}

function Question({ title, hint }: { title: string; hint?: string }) {
  return (
    <div className="space-y-0.5">
      <p className="text-sm font-medium text-foreground">{title}</p>
      {hint && <p className="text-xs text-muted-foreground">{hint}</p>}
    </div>
  );
}

function Segmented({
  options,
  value,
  onChange,
}: {
  options: { key: string; label: string; icon: typeof Palette }[];
  value: string;
  onChange: (v: string) => void;
}) {
  return (
    <div className="inline-flex rounded-lg border border-border bg-secondary/40 p-1">
      {options.map((o) => {
        const active = value === o.key;
        const Icon = o.icon;
        return (
          <button
            key={o.key}
            type="button"
            onClick={() => onChange(o.key)}
            className={cn(
              "inline-flex items-center gap-2 rounded-md px-3 py-1.5 text-sm font-medium transition-colors",
              active ? "bg-primary text-primary-foreground shadow-sm" : "text-muted-foreground hover:text-foreground",
            )}
          >
            <Icon className="size-4" />
            {o.label}
          </button>
        );
      })}
    </div>
  );
}

function Chip({
  active,
  onClick,
  disabled,
  title,
  children,
}: {
  active?: boolean;
  onClick?: () => void;
  disabled?: boolean;
  title?: string;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      title={title}
      className={cn(
        "inline-flex items-center gap-1.5 rounded-full border px-3 py-1.5 text-xs font-medium transition-colors disabled:opacity-60",
        active
          ? "border-primary bg-primary/10 text-foreground"
          : "border-border bg-card text-muted-foreground hover:border-primary/40 hover:text-foreground",
      )}
    >
      {children}
    </button>
  );
}
