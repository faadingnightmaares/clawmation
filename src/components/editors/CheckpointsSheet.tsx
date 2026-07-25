import { useCallback, useEffect, useState } from "react";
import { Camera, Crop, Crosshair, Eye, Flag, Grab, Loader2, Pipette, Plus, Trash2 } from "lucide-react";

import {
  addCheckpoint,
  captureTemplate,
  guardPickColor,
  guardPickRegion,
  listCheckpoints,
  removeCheckpoint,
  surgicalCapture,
  type CheckpointItem,
  type Json,
} from "@/api";
import { hsvToCss } from "@/format";
import { pxRectToPct } from "@/lib/screen";
import { notify } from "@/lib/toast";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import type { EditorSheetProps } from "./GuardsSheet";

// A checkpoint pauses a macro mid-run: it watches the screen until a colour or
// image shows up, then clicks it / presses a key (Wait & act), or holds a button
// as the target moves and lets go when it appears/disappears (Hold & follow).
// The draft here is the working copy the form edits; `add` freezes it into the
// exact config map the backend stores; every one of those fields is load-bearing
// (the checkpoint runner reads them by name), so the object below is transcribed,
// not paraphrased, and tsc cannot police it (`addCheckpoint` takes an open `Json`).

interface CheckpointDraft {
  mode: "wait_for" | "hold_follow";
  method: "color" | "template";
  hsv_low: number[] | null;
  hsv_high: number[] | null;
  colorHex: string | null;
  template: string | null;
  templateThumb: string | null;
  click_offset: number[] | null;
  click_line: number[] | null;
  click_lines: number[][] | null;
  region: number[] | null;
  action: "click" | "key" | "none";
  key: string;
  releaseWhen: "lost" | "found";
  timeout: number | string;
}

function freshDraft(): CheckpointDraft {
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

/** One plain-language sentence describing a stored checkpoint, for its list row. */
function describeCheckpoint(c: Json): string {
  const thing = c.method === "color" ? "a colour" : "an image";
  if (c.mode === "hold_follow") {
    const when = c.release_when === "found" ? "it shows up" : "it disappears";
    return `Holds on ${thing} and follows it, letting go when ${when}.`;
  }
  const act =
    c.action === "click"
      ? "clicks it"
      : c.action === "key"
        ? `presses ${String(c.key || "a key")}`
        : "just waits";
  return `Waits for ${thing}, then ${act}.`;
}

export function CheckpointsSheet({ macroName, open, onOpenChange, onChanged }: EditorSheetProps) {
  const [checkpoints, setCheckpoints] = useState<CheckpointItem[]>([]);
  const [loading, setLoading] = useState(false);
  const [busy, setBusy] = useState(false);
  const [draft, setDraft] = useState<CheckpointDraft>(freshDraft);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const r = await listCheckpoints(macroName);
      setCheckpoints(r.ok ? r.checkpoints ?? [] : []);
    } catch {
      setCheckpoints([]);
    } finally {
      setLoading(false);
    }
  }, [macroName]);

  useEffect(() => {
    if (open) {
      setDraft(freshDraft());
      void load();
    }
  }, [open, load]);

  const patch = (p: Partial<CheckpointDraft>) => setDraft((d) => ({ ...d, ...p }));

  // ── Native screen selections: each merges its result back into the draft ──
  const onPickColor = async () => {
    try {
      const r = await guardPickColor();
      if (r.ok) patch({ hsv_low: r.hsv_low ?? null, hsv_high: r.hsv_high ?? null, colorHex: r.hex ?? null });
      else if (r.error && r.error !== "cancelled") notify("error", r.error);
    } catch {
      notify("error", "Couldn’t pick a colour.");
    }
  };

  const onCaptureImage = async () => {
    try {
      const r = await captureTemplate();
      if (r.ok) patch({ template: r.path ?? null, templateThumb: r.thumb ? "data:image/png;base64," + r.thumb : null });
      else if (r.error && r.error !== "cancelled") notify("error", r.error);
    } catch {
      notify("error", "Couldn’t capture the image.");
    }
  };

  const onSurgical = async () => {
    try {
      const r = await surgicalCapture();
      if (r.ok)
        patch({
          method: "template",
          template: r.path ?? null,
          templateThumb: r.thumb ? "data:image/png;base64," + r.thumb : null,
          click_offset: r.offset ?? null,
          click_line: r.click_line ?? null,
          click_lines: r.click_lines ?? null,
        });
      else if (r.error && r.error !== "cancelled") notify("error", r.error);
    } catch {
      notify("error", "Couldn’t capture.");
    }
  };

  const onPickRegion = async () => {
    try {
      const r = await guardPickRegion();
      if (r.ok) patch({ region: pxRectToPct(r.x ?? 0, r.y ?? 0, r.w ?? 0, r.h ?? 0) });
      else if (r.error && r.error !== "cancelled") notify("error", r.error);
    } catch {
      notify("error", "Couldn’t set the watch area.");
    }
  };

  const add = async () => {
    if (draft.method === "color" && !draft.hsv_low) {
      notify("error", "Pick a colour first.");
      return;
    }
    if (draft.method === "template" && !draft.template) {
      notify("error", "Capture an image first.");
      return;
    }
    // The stored config map: every field the checkpoint runner reads.
    const checkpoint = {
      mode: draft.mode,
      method: draft.method,
      hsv_low: draft.hsv_low,
      hsv_high: draft.hsv_high,
      template: draft.template,
      click_offset: draft.click_offset,
      click_line: draft.click_line,
      click_lines: draft.click_lines,
      region: draft.region || [0, 0, 100, 100],
      threshold: 0.75,
      timeout: parseFloat(String(draft.timeout)) || 10,
      action: draft.action,
      key: draft.key || "",
      button: "left",
      min_area: 40,
      hold_button: "left",
      release_when: draft.releaseWhen,
      poll: draft.mode === "hold_follow" ? 0.02 : 0.05,
    };
    setBusy(true);
    try {
      const res = await addCheckpoint(macroName, -1, checkpoint);
      if (res.ok) {
        notify("success", "Checkpoint added.");
        setDraft(freshDraft());
        await load();
        onChanged?.();
      } else {
        notify("error", res.error || "Couldn’t add the checkpoint.");
      }
    } catch (e) {
      notify("error", String(e));
    } finally {
      setBusy(false);
    }
  };

  const remove = async (index: number) => {
    try {
      const res = await removeCheckpoint(macroName, index);
      if (res.ok) {
        notify("info", "Checkpoint removed.");
        await load();
        onChanged?.();
      }
    } catch {
      notify("error", "Couldn’t remove it.");
    }
  };

  const isHold = draft.mode === "hold_follow";
  const isColor = draft.method === "color";
  const hasClickSpot =
    !!(draft.click_lines && draft.click_lines.length) ||
    (draft.click_line?.length === 4) ||
    (draft.click_offset?.length === 2);
  const regionSet = !!draft.region && !(draft.region[0] === 0 && draft.region[1] === 0 && draft.region[2] === 100 && draft.region[3] === 100);

  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent className="flex w-full flex-col gap-0 p-0 sm:max-w-lg">
        <SheetHeader className="px-6 pb-2 pr-12 pt-6">
          <SheetTitle>Checkpoints</SheetTitle>
          <SheetDescription>
            Make “{macroName}” wait for the screen to be ready before it carries on. Perfect for loading
            screens, popups, and moving targets.
          </SheetDescription>
        </SheetHeader>

        <div className="flex-1 space-y-6 overflow-y-auto px-6 py-5">
          {/* Existing checkpoints */}
          {loading ? (
            <div className="flex items-center justify-center gap-2 py-10 text-sm text-muted-foreground">
              <Loader2 className="size-4 animate-spin" /> Loading…
            </div>
          ) : checkpoints.length ? (
            <section className="space-y-2">
              <h3 className="text-sm font-semibold text-foreground">In this macro</h3>
              {checkpoints.map((cp) => {
                const c = cp.checkpoint ?? {};
                const hold = c.mode === "hold_follow";
                return (
                  <div
                    key={cp.index}
                    className="flex items-center gap-3 rounded-lg border border-border bg-card p-3"
                  >
                    <div className="flex size-9 shrink-0 items-center justify-center rounded-md bg-secondary text-primary">
                      {hold ? <Grab className="size-4" /> : <Eye className="size-4" />}
                    </div>
                    <div className="min-w-0 flex-1">
                      <p className="text-sm font-medium text-foreground">
                        {hold ? "Hold & follow" : "Wait & act"}
                      </p>
                      <p className="truncate text-xs text-muted-foreground">{describeCheckpoint(c)}</p>
                    </div>
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => remove(cp.index)}
                      title="Remove"
                      className="text-muted-foreground hover:text-destructive"
                    >
                      <Trash2 className="size-4" />
                    </Button>
                  </div>
                );
              })}
            </section>
          ) : (
            <div className="flex flex-col items-center gap-3 py-8 text-center">
              <div className="flex size-12 items-center justify-center rounded-full bg-secondary text-muted-foreground">
                <Flag className="size-6" />
              </div>
              <div>
                <p className="text-sm font-semibold text-foreground">No checkpoints yet</p>
                <p className="mx-auto mt-1 max-w-xs text-sm text-muted-foreground">
                  Add one below to pause the macro until something you choose shows up on screen.
                </p>
              </div>
            </div>
          )}

          {/* Add form */}
          <section className="space-y-4 rounded-xl border border-border bg-secondary/30 p-4">
            <h3 className="text-sm font-semibold text-foreground">
              {checkpoints.length ? "Add another" : "Add a checkpoint"}
            </h3>

            {/* Mode */}
            <div className="grid grid-cols-2 gap-2">
              <Button
                type="button"
                variant={!isHold ? "default" : "outline"}
                size="sm"
                className="h-9"
                onClick={() => patch({ mode: "wait_for" })}
              >
                <Eye className="size-4" /> Wait &amp; act
              </Button>
              <Button
                type="button"
                variant={isHold ? "default" : "outline"}
                size="sm"
                className="h-9"
                onClick={() => patch({ mode: "hold_follow" })}
              >
                <Grab className="size-4" /> Hold &amp; follow
              </Button>
            </div>

            {/* What to watch for */}
            <div className="space-y-2">
              <p className="text-xs font-medium text-muted-foreground">What should it watch for?</p>
              <div className="grid grid-cols-2 gap-2">
                <Button
                  type="button"
                  variant={isColor ? "default" : "outline"}
                  size="sm"
                  className="h-9"
                  onClick={() => patch({ method: "color" })}
                >
                  A colour
                </Button>
                <Button
                  type="button"
                  variant={!isColor ? "default" : "outline"}
                  size="sm"
                  className="h-9"
                  onClick={() => patch({ method: "template" })}
                >
                  An image
                </Button>
              </div>

              {/* Pickers for the chosen kind */}
              <div className="flex flex-wrap items-center gap-2 pt-1">
                {isColor ? (
                  <>
                    <Button type="button" variant="outline" size="sm" onClick={onPickColor}>
                      <Pipette className="size-4" /> Pick colour
                    </Button>
                    {draft.hsv_low && (
                      <span className="inline-flex items-center gap-2 rounded-md border border-border px-2.5 py-1.5 text-xs text-muted-foreground">
                        <span
                          className="size-4 rounded border border-border"
                          style={{ background: hsvToCss(draft.hsv_low ?? undefined, draft.hsv_high ?? undefined) }}
                        />
                        {draft.colorHex || "Colour set"}
                      </span>
                    )}
                  </>
                ) : (
                  <>
                    <Button type="button" variant="outline" size="sm" onClick={onCaptureImage}>
                      <Camera className="size-4" /> Capture image
                    </Button>
                    <Button
                      type="button"
                      variant="outline"
                      size="sm"
                      onClick={onSurgical}
                      title="Snip the image, then draw the exact spot to click"
                      className="border-primary/50 text-primary hover:text-primary"
                    >
                      <Crosshair className="size-4" /> Exact spot
                    </Button>
                    {draft.templateThumb && (
                      <img
                        src={draft.templateThumb}
                        alt=""
                        className="h-8 rounded-md border border-border"
                      />
                    )}
                    {hasClickSpot && (
                      <span className="inline-flex items-center gap-1.5 rounded-md border border-primary/40 bg-primary/10 px-2.5 py-1.5 text-xs text-primary">
                        <Crosshair className="size-3.5" /> Exact spot set
                      </span>
                    )}
                  </>
                )}
              </div>
            </div>

            {/* Watch area */}
            <div className="flex items-center gap-2">
              <Button
                type="button"
                variant={regionSet ? "secondary" : "outline"}
                size="sm"
                onClick={onPickRegion}
              >
                <Crop className="size-4" /> {regionSet ? "Watch area set" : "Limit to an area"}
              </Button>
              <span className="text-xs text-muted-foreground">Optional. Keeps it looking where it matters.</span>
            </div>

            {/* Then… */}
            <div className="flex flex-wrap items-center gap-2">
              <span className="text-xs font-medium text-muted-foreground">Then</span>
              <Select value={draft.action} onValueChange={(v) => patch({ action: v as CheckpointDraft["action"] })}>
                <SelectTrigger size="sm" className="min-w-[130px]">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="click">click it</SelectItem>
                  <SelectItem value="key">press a key</SelectItem>
                  <SelectItem value="none">do nothing</SelectItem>
                </SelectContent>
              </Select>
              {draft.action === "key" && (
                <Input
                  value={draft.key}
                  onChange={(e) => patch({ key: e.target.value })}
                  placeholder="key"
                  className="h-9 w-20"
                />
              )}
              {isHold && (
                <>
                  <span className="text-xs font-medium text-muted-foreground">and let go when</span>
                  <Select
                    value={draft.releaseWhen}
                    onValueChange={(v) => patch({ releaseWhen: v as CheckpointDraft["releaseWhen"] })}
                  >
                    <SelectTrigger size="sm" className="min-w-[150px]">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="lost">it disappears</SelectItem>
                      <SelectItem value="found">it shows up</SelectItem>
                    </SelectContent>
                  </Select>
                </>
              )}
            </div>

            {/* Give up after */}
            <div className="flex items-center gap-2">
              <span className="text-xs font-medium text-muted-foreground">Give up after</span>
              <Input
                value={String(draft.timeout)}
                onChange={(e) => patch({ timeout: e.target.value })}
                inputMode="numeric"
                className="h-9 w-16"
              />
              <span className="text-xs text-muted-foreground">seconds</span>
            </div>

            <Button className={cn("w-full")} onClick={add} disabled={busy}>
              {busy ? <Loader2 className="size-4 animate-spin" /> : <Plus className="size-4" />}
              Add checkpoint
            </Button>
          </section>
        </div>
      </SheetContent>
    </Sheet>
  );
}
