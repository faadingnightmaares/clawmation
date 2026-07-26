import { useState } from "react";
import {
  Camera,
  Check,
  ChevronDown,
  Crop,
  FlaskConical,
  FolderOpen,
  ImageIcon,
  Keyboard,
  Loader2,
  MousePointerClick,
  Move,
  Palette,
  Pipette,
  ScanLine,
  Type,
  type LucideIcon,
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
  REPEAT_STOPS,
  RESUME_STOPS,
  accuracyOf,
  guardFromDraft,
  isAnywhere,
  isTriggerReady,
  lookOf,
  methodFor,
  repeatStopOf,
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
const asDataUri = (thumb: string, mime = "image/png") =>
  thumb.startsWith("data:") ? thumb : `data:${mime};base64,${thumb}`;

/**
 * One trigger, edited as the sentence it is: "when this appears, do that".
 *
 * The shape of this panel is the point. It used to open on a three-up grid of
 * detection methods, which asked the reader to classify their problem before
 * they could describe it, and then stacked six equally loud sections underneath.
 * Now the first thing on screen is the gesture itself: three rows, each of which
 * *does* the thing when clicked rather than selecting a mode to configure. What
 * kind of detection that implies is settled from what they showed it, never
 * named. Everything that has a right answer already lives behind one disclosure,
 * so the common path is show it, say what to do, save.
 */
export function TriggerEditor({ initial, showSurgical, saveLabel = "Save", onSave, onCancel }: TriggerEditorProps) {
  const [d, setD] = useState<TriggerDraft>(initial);
  // Until the name is typed in, a text trigger names itself after the words it
  // watches for. Typing "Reconnect" twice was the most common thing to do here.
  const [namedByHand, setNamedByHand] = useState(!!initial.name);
  const [thumb, setThumb] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [testing, setTesting] = useState(false);
  const [saving, setSaving] = useState(false);
  const [tuning, setTuning] = useState(false);
  /** Set when the reader asks to watch for something else, so the chooser comes
   *  back over a trigger that is already specified. */
  const [reselect, setReselect] = useState(false);
  const formRef = useStaggerIn<HTMLDivElement>();

  const patch = (p: Partial<TriggerDraft>) => setD((prev) => ({ ...prev, ...p }));
  const look = lookOf(d.method);
  const ready = isTriggerReady(d);
  const preview = d._preview;
  const choosing = reselect || (!ready && look !== "text");

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
        patch({ method: methodFor("color"), hsv_low: r.hsv_low, hsv_high: r.hsv_high, _preview: null });
        setReselect(false);
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
          method: methodFor("image"),
          template_path: r.path,
          click_offset: (r as { offset?: number[] }).offset ?? [],
          click_line: (r as { click_line?: number[] }).click_line ?? [],
          click_lines: (r as { click_lines?: number[][] }).click_lines ?? [],
          _preview: null,
        });
        setReselect(false);
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
        patch({
          method: methodFor("image"),
          template_path: r.path,
          click_offset: [],
          click_line: [],
          click_lines: [],
          _preview: null,
        });
        setReselect(false);
        if (r.thumb) setThumb(asDataUri(r.thumb));
        notify("success", "Image chosen.");
      } else if (r.error) {
        notify("error", r.error);
      }
    });

  const chooseWords = () => {
    patch({ method: methodFor("text"), _preview: null });
    setReselect(false);
  };

  const pickRegion = () =>
    run("region", async () => {
      const r = await guardPickRegion();
      if (r.ok) {
        // Prefer the frame-relative percentages the picker computed; fall back to
        // a local conversion only for a backend that omits them.
        const region = r.region ?? pxRectToPct(r.x ?? 0, r.y ?? 0, r.w ?? 0, r.h ?? 0);
        patch({ region, _preview: null });
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

  // What the disclosure holds, said in one line so it is never a mystery box.
  const tuningSummary = [
    isAnywhere(d.region) ? "anywhere on screen" : "one chosen area",
    look === "image" ? ACCURACY_STOPS.find((s) => s.key === accuracyOf(d.threshold))?.label.toLowerCase() : null,
    showSurgical
      ? REPEAT_STOPS.find((s) => s.seconds === repeatStopOf(d.cooldown))?.label.toLowerCase()
      : RESUME_STOPS.find((s) => s.seconds === resumeStopOf(d.resume_delay))?.label.toLowerCase(),
  ]
    .filter(Boolean)
    .join(" · ");

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div ref={formRef} className="flex-1 overflow-y-auto px-6 pb-6">
        {/* The name reads as the title of the thing being edited, not as the
            first question of a form. It is also the only one with a sane
            default, so it never blocks anyone. */}
        <input
          value={d.name}
          onChange={(e) => {
            setNamedByHand(true);
            patch({ name: e.target.value });
          }}
          placeholder="Name this trigger"
          autoComplete="off"
          aria-label="Trigger name"
          className="w-full border-0 bg-transparent py-4 text-lg font-semibold tracking-tight text-foreground outline-none placeholder:font-normal placeholder:text-muted-foreground/60"
        />

        {/* ── When this appears ──────────────────────────────────────────── */}
        <Step n={1} title="When this appears">
          {choosing ? (
            <div className="divide-y divide-border overflow-hidden rounded-xl border border-border">
              <ChooseRow
                icon={ScanLine}
                title={showSurgical ? "Snap it and mark where to click" : "Snap it on screen"}
                blurb={
                  showSurgical
                    ? "Draw round the button, then mark the exact spot to press."
                    : "Draw round the button so I can recognise it again."
                }
                busy={busy === "image"}
                onClick={() => captureImage(!!showSurgical)}
              />
              <ChooseRow
                icon={Pipette}
                title="Pick its colour"
                blurb="Click the thing on screen and I learn the colour."
                busy={busy === "color"}
                onClick={pickColor}
              />
              <ChooseRow
                icon={Type}
                title="Watch for words"
                blurb="Type what shows up, like Reconnect."
                onClick={chooseWords}
              />
            </div>
          ) : look === "text" ? (
            <div className="space-y-2">
              <Input
                id="tr-text"
                value={d.ocr_text}
                onChange={(e) => {
                  const ocr_text = e.target.value;
                  patch(namedByHand ? { ocr_text, _preview: null } : { ocr_text, name: ocr_text, _preview: null });
                }}
                placeholder="Reconnect"
                autoComplete="off"
                autoFocus
              />
              <div className="flex items-center justify-between gap-3">
                <p className="text-xs text-muted-foreground">I read the screen and look for these words.</p>
                <SwapLink onClick={() => setReselect(true)} />
              </div>
            </div>
          ) : (
            <Specified
              look={look}
              draft={d}
              thumb={thumb}
              busy={busy}
              onChange={() => (look === "image" ? captureImage(!!showSurgical) : pickColor())}
              onChooseFile={chooseFile}
              onSwap={() => setReselect(true)}
            />
          )}
        </Step>

        {/* ── Do this ────────────────────────────────────────────────────── */}
        <Step n={2} title="Do this">
          <div className="flex flex-wrap items-center gap-3">
            <Segmented
              options={[
                { key: "click", label: "Click it", icon: MousePointerClick },
                { key: "key", label: "Press a key", icon: Keyboard },
                // Watch-only: a per-macro guard acts on the game, it does not
                // keep the game thinking someone is there.
                ...(showSurgical ? [{ key: "nudge", label: "Nudge the mouse", icon: Move }] : []),
              ]}
              value={d.action}
              onChange={(v) => patch({ action: v })}
            />
            {d.action === "key" && (
              <Input
                value={d.key}
                onChange={(e) => patch({ key: e.target.value })}
                placeholder="space, enter, f"
                autoComplete="off"
                className="w-40 font-mono"
              />
            )}
          </div>
        </Step>

        {/* ── Everything with a right answer already ─────────────────────── */}
        <div className="mt-2 border-t border-border pt-2">
          <button
            type="button"
            onClick={() => setTuning((t) => !t)}
            aria-expanded={tuning}
            className="flex w-full items-center gap-3 rounded-lg px-1 py-3 text-left transition-colors hover:bg-secondary/40"
          >
            <ChevronDown className={cn("size-4 shrink-0 text-muted-foreground transition-transform", tuning && "rotate-180")} />
            <span className="text-sm font-medium text-foreground">Fine tuning</span>
            <span className="ml-auto truncate text-xs text-muted-foreground">{tuningSummary}</span>
          </button>

          {tuning && (
            <div className="space-y-6 px-1 pb-4 pt-2">
              <Field label="Where should I look?" hint="A smaller area is faster and avoids false alarms.">
                <div className="flex flex-wrap items-center gap-2">
                  <Chip active={isAnywhere(d.region)} onClick={() => patch({ region: [0, 0, 100, 100], _preview: null })}>
                    Anywhere on screen
                  </Chip>
                  <Chip active={!isAnywhere(d.region)} onClick={pickRegion} disabled={busy === "region"}>
                    {busy === "region" ? <Loader2 className="size-3.5 animate-spin" /> : <Crop className="size-3.5" />}
                    {isAnywhere(d.region) ? "Just one area" : "A chosen area"}
                  </Chip>
                  {!isAnywhere(d.region) && (
                    <Button variant="ghost" size="xs" onClick={pickRegion} disabled={busy === "region"}>
                      Change
                    </Button>
                  )}
                </div>
              </Field>

              {look === "image" && (
                <Field label="How close a match?" hint="Forgiving catches more, exact avoids mistakes.">
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
                </Field>
              )}

              {/* The two engines pace differently and only one control does
                  anything in each: the standalone watcher re-looks on `cooldown`
                  and never reads `resume_delay`, while a per-macro guard pauses
                  playback for `resume_delay` before letting the macro carry on.
                  Each side is asked only about the number it actually obeys. */}
              {showSurgical ? (
                <Field label="How soon can it act again?" hint="“As fast as it can” keeps clicking the instant it comes back.">
                  <div className="flex flex-wrap gap-2">
                    {REPEAT_STOPS.map((s) => (
                      <Chip
                        key={s.seconds}
                        active={repeatStopOf(d.cooldown) === s.seconds}
                        onClick={() => patch({ cooldown: s.seconds })}
                        title={s.hint}
                      >
                        {s.label}
                      </Chip>
                    ))}
                  </div>
                </Field>
              ) : (
                <Field label="After it acts, how long should I wait?" hint="Give the game time to catch up before carrying on.">
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
                </Field>
              )}
            </div>
          )}
        </div>
      </div>

      {/* What the last test saw, sitting directly above the button that ran it. */}
      {preview && (
        <div className="border-t border-border px-6 py-3">
          <div
            className={cn(
              "inline-flex items-center gap-1.5 rounded-full px-2.5 py-1 text-xs font-medium",
              preview.ok ? "bg-primary/15 text-primary" : "bg-muted text-muted-foreground",
            )}
          >
            {preview.ok ? <Check className="size-3.5" /> : <FlaskConical className="size-3.5" />}
            {preview.ok ? "I can see it right now" : preview.msg || "Not visible right now"}
          </div>
          {preview.src && (
            <img
              src={preview.src}
              alt="What I saw"
              className="mt-2 max-h-48 w-full rounded-md border border-border object-contain"
            />
          )}
        </div>
      )}

      <div className="flex items-center justify-between gap-3 border-t border-border px-6 py-4">
        <Button variant="ghost" size="sm" onClick={test} disabled={testing || !ready}>
          {testing ? <Loader2 className="size-4 animate-spin" /> : <FlaskConical className="size-4" />}
          Test it
        </Button>
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

/** A numbered half of the sentence. The number is the sentence order, not
 *  decoration: there are exactly two, and the second is meaningless without the
 *  first. */
function Step({ n, title, children }: { n: number; title: string; children: React.ReactNode }) {
  return (
    <section className="border-t border-border py-5">
      <div className="mb-3 flex items-center gap-2.5">
        <span className="flex size-5 shrink-0 items-center justify-center rounded-full bg-primary/15 text-xs font-semibold text-primary">
          {n}
        </span>
        <h3 className="text-sm font-medium text-foreground">{title}</h3>
      </div>
      {children}
    </section>
  );
}

/** One row of the chooser. Clicking it performs the gesture straight away rather
 *  than selecting a mode that then has to be configured. */
function ChooseRow({
  icon: Icon,
  title,
  blurb,
  busy,
  onClick,
}: {
  icon: LucideIcon;
  title: string;
  blurb: string;
  busy?: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={busy}
      className="flex w-full items-center gap-3 px-4 py-3 text-left transition-colors hover:bg-secondary/50 disabled:opacity-60"
    >
      <span className="flex size-9 shrink-0 items-center justify-center rounded-lg border border-border bg-secondary/50 text-muted-foreground">
        {busy ? <Loader2 className="size-4 animate-spin" /> : <Icon className="size-4" />}
      </span>
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm font-medium text-foreground">{title}</span>
        <span className="block truncate text-xs text-muted-foreground">{blurb}</span>
      </span>
    </button>
  );
}

/** What it is watching for, once it knows. */
function Specified({
  look,
  draft,
  thumb,
  busy,
  onChange,
  onChooseFile,
  onSwap,
}: {
  look: LookKind;
  draft: TriggerDraft;
  thumb: string | null;
  busy: string | null;
  onChange: () => void;
  onChooseFile: () => void;
  onSwap: () => void;
}) {
  const redoing = busy === "image" || busy === "color";
  const marked =
    draft.click_lines.length > 0
      ? "A drag is marked on it"
      : draft.click_offset.length === 2
        ? "The click point is marked on it"
        : "It presses the middle of the picture";

  return (
    <div className="space-y-3">
      <div className="flex items-center gap-3 rounded-xl border border-border px-4 py-3">
        <div className="flex size-11 shrink-0 items-center justify-center overflow-hidden rounded-lg border border-border bg-secondary/50">
          {look === "image" ? (
            thumb ? (
              <img src={thumb} alt="" className="size-full object-contain" />
            ) : (
              <ImageIcon className="size-4 text-muted-foreground" />
            )
          ) : (
            <span
              className="size-5 rounded-full border border-border"
              style={{ background: hsvToCss(draft.hsv_low, draft.hsv_high) }}
            />
          )}
        </div>
        <div className="min-w-0 flex-1">
          <p className="text-sm font-medium text-foreground">
            {look === "image" ? "Picture saved" : "Colour picked"}
          </p>
          <p className="truncate text-xs text-muted-foreground">
            {look === "image" ? marked : "It acts wherever that colour shows up"}
          </p>
        </div>
        <Button variant="secondary" size="sm" onClick={onChange} disabled={redoing}>
          {redoing ? (
            <Loader2 className="size-4 animate-spin" />
          ) : look === "image" ? (
            <Camera className="size-4" />
          ) : (
            <Palette className="size-4" />
          )}
          Redo
        </Button>
      </div>
      <div className="flex items-center justify-between gap-3">
        {look === "image" ? (
          <Button variant="ghost" size="xs" onClick={onChooseFile} disabled={busy === "image"}>
            <FolderOpen className="size-3.5" />
            Use an image file instead
          </Button>
        ) : (
          <span />
        )}
        <SwapLink onClick={onSwap} />
      </div>
    </div>
  );
}

function SwapLink({ onClick }: { onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="shrink-0 text-xs text-muted-foreground underline-offset-4 transition-colors hover:text-foreground hover:underline"
    >
      Watch for something else
    </button>
  );
}

function Field({ label, hint, children }: { label: string; hint?: string; children: React.ReactNode }) {
  return (
    <div className="space-y-2">
      <Label className="text-sm font-medium text-foreground">{label}</Label>
      {hint && <p className="-mt-1 text-xs text-muted-foreground">{hint}</p>}
      {children}
    </div>
  );
}

function Segmented({
  options,
  value,
  onChange,
}: {
  options: { key: string; label: string; icon: LucideIcon }[];
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
