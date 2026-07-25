import { useCallback, useEffect, useState, type ComponentType } from "react";
import {
  Check,
  ChevronDown,
  ChevronUp,
  Crosshair,
  Eye,
  FlaskConical,
  Keyboard,
  Loader2,
  Mouse,
  MousePointerClick,
  Pipette,
  Play,
  Timer,
  Trash2,
  Type as TypeIcon,
  Wand2,
} from "lucide-react";

import { guardPickColor, macroToSteps, stepsRun, stepsSave, stepsTest, type Step } from "@/api";
import { hsvToCss } from "@/format";
import { useStaggerIn } from "@/lib/anime";
import { notify } from "@/lib/toast";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import type { EditorSheetProps } from "./GuardsSheet";

// The step editor — a recording opened as a flat list of actions the user can
// reorder, tweak, and slot detection steps between. It owns its own async seam
// (convert / save / play / test / pick-colour) rather than taking callbacks, so
// it drops straight into the macro list like the guard and checkpoint sheets.
//
// Detection steps match by COLOUR only: template / features are dead branches
// for AI steps (`ai_detect` never loads a template), so the editor never
// surfaces detect-mode, template, or confidence — they stay on the hidden
// defaults below. Everything else the old panel could edit is kept.

type StepIcon = ComponentType<{ className?: string }>;

const STEP_META: Record<string, { label: string; Icon: StepIcon; detect?: boolean }> = {
  click: { label: "Click", Icon: MousePointerClick },
  key: { label: "Key", Icon: Keyboard },
  type: { label: "Type", Icon: TypeIcon },
  scroll: { label: "Scroll", Icon: Mouse },
  delay: { label: "Pause", Icon: Timer },
  find_click: { label: "Find & click", Icon: Crosshair, detect: true },
  wait_for: { label: "Wait for", Icon: Eye, detect: true },
};

const INSERT_OPTIONS: { type: string; label: string; Icon: StepIcon; detect?: boolean }[] = [
  { type: "find_click", label: "Find colour", Icon: Crosshair, detect: true },
  { type: "wait_for", label: "Wait for colour", Icon: Eye, detect: true },
  { type: "delay", label: "Pause", Icon: Timer },
  { type: "click", label: "Click", Icon: MousePointerClick },
  { type: "key", label: "Key", Icon: Keyboard },
];

/** A freshly inserted step — a complete `Step`, since the list `macroToSteps`
 * returns is already complete objects. Mirrors the Rust `Step` defaults
 * (src-tauri/src/models/step.rs); the backend serde-fills any omitted field to
 * these same values, so only the type-specific ones ever render. Detection
 * inserts default to colour matching (the sole path `ai_detect` runs), keeping
 * template / region / confidence / min_area on their inert defaults. */
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
    key: { key: "", label: "Press a key" },
    type: { text: "", label: "Type text" },
    scroll: { scroll_amount: 3, label: "Scroll up" },
    delay: { delay: 1.0, label: "Wait 1s" },
    find_click: { detect_mode: "color", label: "Find & click a colour" },
    wait_for: { detect_mode: "color", timeout: 10, label: "Wait for a colour" },
  };
  return { ...base, ...(overlays[type] || {}) };
}

/** A colour step still on its full-range default — no colour picked, so it would
 * match every pixel. Drives the "pick one" hint. */
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

function InsertBar({ onInsert }: { onInsert: (type: string) => void }) {
  return (
    <div className="flex flex-wrap items-center gap-1">
      <span className="mr-0.5 text-[10px] font-medium uppercase tracking-wide text-muted-foreground/50">
        Insert
      </span>
      {INSERT_OPTIONS.map((o) => {
        const Icon = o.Icon;
        return (
          <Button
            key={o.type}
            type="button"
            variant="ghost"
            size="sm"
            className={cn(
              "h-6 gap-1 px-2 text-[11px] font-normal text-muted-foreground hover:text-foreground",
              o.detect && "text-primary/80 hover:text-primary",
            )}
            onClick={() => onInsert(o.type)}
          >
            <Icon className="size-3" /> {o.label}
          </Button>
        );
      })}
    </div>
  );
}

export function StepsSheet({ macroName, open, onOpenChange, onChanged }: EditorSheetProps) {
  const [steps, setSteps] = useState<Step[]>([]);
  const [loading, setLoading] = useState(false);
  const [busy, setBusy] = useState<"save" | "run" | null>(null);
  const [testingIdx, setTestingIdx] = useState<number | null>(null);
  const [pickingIdx, setPickingIdx] = useState<number | null>(null);
  const [testResult, setTestResult] = useState<{ idx: number; ok: boolean; preview: string | null } | null>(null);
  const [bulkValue, setBulkValue] = useState<string | null>(null);

  const listRef = useStaggerIn<HTMLDivElement>(steps.length);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const r = await macroToSteps(macroName);
      setSteps(r.ok ? r.steps ?? [] : []);
    } catch {
      setSteps([]);
    } finally {
      setLoading(false);
    }
  }, [macroName]);

  useEffect(() => {
    if (open) {
      setTestResult(null);
      setBulkValue(null);
      void load();
    }
  }, [open, load]);

  // ── In-memory list transforms ─────────────────────────────────────────────
  const updateStepAt = (idx: number, patch: Partial<Step>) =>
    setSteps((prev) => {
      const next = [...prev];
      next[idx] = { ...next[idx], ...patch };
      return next;
    });

  // Structural edits shift indices, so any test highlight is dropped to keep it
  // from pointing at a moved row.
  const deleteStepAt = (idx: number) => {
    setSteps((prev) => prev.filter((_, i) => i !== idx));
    setTestResult(null);
  };

  const moveStepAt = (idx: number, dir: number) => {
    setSteps((prev) => {
      const j = idx + dir;
      if (j < 0 || j >= prev.length) return prev;
      const next = [...prev];
      [next[idx], next[j]] = [next[j], next[idx]];
      return next;
    });
    setTestResult(null);
  };

  const insertStepAt = (idx: number, type: string) => {
    setSteps((prev) => {
      const next = [...prev];
      next.splice(idx + 1, 0, makeStep(type));
      return next;
    });
    setTestResult(null);
  };

  // Bulk-edit every pause at once — the most common post-recording cleanup, since
  // recorded waits are noisy from human hesitation. The `?? "0.5"` / `?? "1"`
  // fallbacks preserve the source quirk that a bare "× factor" multiplies by 1.
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

  // ── Async seam ────────────────────────────────────────────────────────────
  const save = async () => {
    setBusy("save");
    try {
      const r = await stepsSave(macroName, steps);
      if (r.ok) {
        notify("success", "Your changes are saved.");
        onChanged?.();
      } else {
        notify("error", r.error || "Couldn’t save your changes.");
      }
    } catch (e) {
      notify("error", String(e));
    } finally {
      setBusy(null);
    }
  };

  const run = async () => {
    setBusy("run");
    try {
      const r = await stepsRun(steps);
      if (r.ok) notify("success", "Playing these actions now.");
      else notify("error", r.error || "Couldn’t play these actions.");
    } catch (e) {
      notify("error", String(e));
    } finally {
      setBusy(null);
    }
  };

  const test = async (idx: number) => {
    setTestingIdx(idx);
    try {
      const r = await stepsTest(steps[idx]);
      setTestResult({
        idx,
        ok: !!r.ok,
        preview: r.preview ? "data:image/jpeg;base64," + r.preview : null,
      });
    } catch (e) {
      notify("error", String(e));
    } finally {
      setTestingIdx(null);
    }
  };

  // Sampling a colour also switches the step into colour mode — a step loaded in
  // the old template default would otherwise keep the picked colour but never act
  // on it, so the button would look like it did nothing.
  const pickColor = async (idx: number) => {
    setPickingIdx(idx);
    try {
      const r = await guardPickColor();
      if (r.ok && r.hsv_low && r.hsv_high) {
        updateStepAt(idx, { detect_mode: "color", hsv_low: r.hsv_low, hsv_high: r.hsv_high });
      } else if (r.error && r.error !== "cancelled") {
        notify("error", r.error);
      }
    } catch {
      notify("error", "Couldn’t pick a colour.");
    } finally {
      setPickingIdx(null);
    }
  };

  const delayCount = steps.filter((s) => s.type === "delay").length;
  const bulkDisplay = bulkValue ?? "0.5";

  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent className="flex w-full flex-col gap-0 p-0 sm:max-w-2xl">
        <SheetHeader className="px-6 pb-4 pr-12 pt-6">
          <SheetTitle>Fine-tune actions</SheetTitle>
          <SheetDescription>
            Edit each action in “{macroName}” one by one — reorder them, adjust the details, or drop in a step
            that waits for a colour on screen.
          </SheetDescription>
        </SheetHeader>

        <div className="flex-1 space-y-5 overflow-y-auto px-6 py-5">
          {loading ? (
            <div className="flex items-center justify-center gap-2 py-16 text-sm text-muted-foreground">
              <Loader2 className="size-4 animate-spin" /> Converting the recording…
            </div>
          ) : (
            <>
              {delayCount > 0 && (
                <div className="flex flex-wrap items-center gap-2 rounded-xl border border-primary/25 bg-primary/[0.04] p-3">
                  <span className="flex items-center gap-1.5 text-xs font-medium text-primary">
                    <Timer className="size-3.5" /> Tidy up the waits
                  </span>
                  <span className="text-xs text-muted-foreground">
                    {delayCount} {delayCount === 1 ? "pause" : "pauses"}
                  </span>
                  <div className="ml-auto flex items-center gap-1.5">
                    <Input
                      value={bulkDisplay}
                      onChange={(e) => setBulkValue(e.target.value)}
                      inputMode="decimal"
                      aria-label="seconds"
                      className="h-8 w-16"
                    />
                    <Button
                      variant="outline"
                      size="sm"
                      className="h-8"
                      onClick={() => bulkEditDelays("set", parseFloat(bulkValue ?? "0.5"))}
                      title="Set every pause to this many seconds"
                    >
                      Set all
                    </Button>
                    <Button
                      variant="outline"
                      size="sm"
                      className="h-8"
                      onClick={() => bulkEditDelays("multiply", parseFloat(bulkValue ?? "1"))}
                      title="Multiply every pause by this factor"
                    >
                      × factor
                    </Button>
                    <Button
                      variant="outline"
                      size="sm"
                      className="h-8"
                      onClick={() => bulkEditDelays("cap", parseFloat(bulkValue ?? "0.5"))}
                      title="Cap every pause at this many seconds"
                    >
                      Cap
                    </Button>
                  </div>
                </div>
              )}

              {steps.length > 0 ? (
                <div ref={listRef} className="space-y-1">
                  {steps.map((step, i) => {
                    const meta = STEP_META[step.type] ?? { label: step.type, Icon: MousePointerClick };
                    const Icon = meta.Icon;
                    const tested = testResult?.idx === i;
                    const isWaitFor = step.type === "wait_for";
                    const isDetect = step.type === "find_click" || isWaitFor;
                    const colourSet = !hsvIsUnset(step.hsv_low, step.hsv_high);
                    return (
                      <div key={step.id} className="space-y-1">
                        {i === 0 && <InsertBar onInsert={(t) => insertStepAt(-1, t)} />}

                        <div
                          className={cn(
                            "rounded-xl border bg-card p-3 transition-colors",
                            tested
                              ? testResult!.ok
                                ? "border-primary/60 bg-primary/[0.04]"
                                : "border-destructive/60 bg-destructive/[0.04]"
                              : "border-border",
                            !step.enabled && "opacity-55",
                          )}
                        >
                          {/* Badge · label · row actions */}
                          <div className="flex items-center gap-2.5">
                            <span
                              className={cn(
                                "flex size-8 shrink-0 items-center justify-center rounded-md",
                                meta.detect ? "bg-primary/10 text-primary" : "bg-secondary text-muted-foreground",
                              )}
                              title={meta.label}
                            >
                              <Icon className="size-4" />
                            </span>
                            <Input
                              value={step.label}
                              onChange={(e) => updateStepAt(i, { label: e.target.value })}
                              placeholder={meta.label}
                              className="h-9 flex-1"
                            />
                            <Button
                              variant="ghost"
                              size="sm"
                              onClick={() => test(i)}
                              disabled={testingIdx === i}
                              title="See if it can find this now"
                              className="px-2 text-muted-foreground"
                            >
                              {testingIdx === i ? (
                                <Loader2 className="size-4 animate-spin" />
                              ) : (
                                <FlaskConical className="size-4" />
                              )}
                            </Button>
                            <Button
                              variant="ghost"
                              size="sm"
                              onClick={() => moveStepAt(i, -1)}
                              disabled={i === 0}
                              title="Move up"
                              className="px-1.5 text-muted-foreground"
                            >
                              <ChevronUp className="size-4" />
                            </Button>
                            <Button
                              variant="ghost"
                              size="sm"
                              onClick={() => moveStepAt(i, 1)}
                              disabled={i === steps.length - 1}
                              title="Move down"
                              className="px-1.5 text-muted-foreground"
                            >
                              <ChevronDown className="size-4" />
                            </Button>
                            <Switch
                              checked={step.enabled}
                              onCheckedChange={(v) => updateStepAt(i, { enabled: v })}
                              aria-label={step.enabled ? "On" : "Off"}
                            />
                            <Button
                              variant="ghost"
                              size="sm"
                              onClick={() => deleteStepAt(i)}
                              title="Delete"
                              className="px-2 text-muted-foreground hover:text-destructive"
                            >
                              <Trash2 className="size-4" />
                            </Button>
                          </div>

                          {/* Type-specific detail, aligned under the label */}
                          <div className="mt-2 flex flex-wrap items-center gap-2 pl-[42px] text-xs text-muted-foreground">
                            {step.type === "click" && (
                              <>
                                <span>Clicks at</span>
                                <Input
                                  value={step.x}
                                  onChange={(e) => updateStepAt(i, { x: parseInt(e.target.value) || 0 })}
                                  inputMode="numeric"
                                  aria-label="X"
                                  placeholder="x"
                                  className="h-8 w-16"
                                />
                                <Input
                                  value={step.y}
                                  onChange={(e) => updateStepAt(i, { y: parseInt(e.target.value) || 0 })}
                                  inputMode="numeric"
                                  aria-label="Y"
                                  placeholder="y"
                                  className="h-8 w-16"
                                />
                              </>
                            )}

                            {step.type === "key" && (
                              <>
                                <span>Presses</span>
                                <Input
                                  value={step.key}
                                  onChange={(e) => updateStepAt(i, { key: e.target.value })}
                                  placeholder="key"
                                  className="h-8 w-28"
                                />
                              </>
                            )}

                            {step.type === "type" && (
                              <>
                                <span>Types</span>
                                <Input
                                  value={step.text}
                                  onChange={(e) => updateStepAt(i, { text: e.target.value })}
                                  placeholder="text to type"
                                  className="h-8 min-w-[8rem] flex-1"
                                />
                              </>
                            )}

                            {step.type === "scroll" && (
                              <>
                                <span>Scrolls</span>
                                <Input
                                  value={step.scroll_amount}
                                  onChange={(e) =>
                                    updateStepAt(i, { scroll_amount: parseInt(e.target.value) || 0 })
                                  }
                                  inputMode="numeric"
                                  aria-label="amount"
                                  className="h-8 w-16"
                                />
                                <span className="text-muted-foreground/70">＋ up · － down</span>
                              </>
                            )}

                            {step.type === "delay" && (
                              <>
                                <span>Waits</span>
                                <Input
                                  value={step.delay}
                                  onChange={(e) => updateStepAt(i, { delay: parseFloat(e.target.value) || 0 })}
                                  inputMode="decimal"
                                  aria-label="seconds"
                                  className="h-8 w-16"
                                />
                                <span>seconds</span>
                              </>
                            )}

                            {isDetect && (
                              <>
                                <span>
                                  {isWaitFor ? "Waits for a colour to appear" : "Finds a colour and clicks it"}
                                </span>
                                <Button
                                  variant="outline"
                                  size="sm"
                                  className="h-8"
                                  onClick={() => pickColor(i)}
                                  disabled={pickingIdx === i}
                                >
                                  {pickingIdx === i ? (
                                    <Loader2 className="size-3.5 animate-spin" />
                                  ) : (
                                    <Pipette className="size-3.5" />
                                  )}
                                  Pick colour
                                </Button>
                                {colourSet ? (
                                  <span className="inline-flex items-center gap-1.5 rounded-md border border-border px-2 py-1">
                                    <span
                                      className="size-4 rounded border border-border"
                                      style={{ background: hsvToCss(step.hsv_low, step.hsv_high) }}
                                    />
                                    Colour set
                                  </span>
                                ) : (
                                  <span className="text-muted-foreground/70">no colour yet — pick one</span>
                                )}
                                {isWaitFor && (
                                  <span className="inline-flex items-center gap-1.5">
                                    <span>· give up after</span>
                                    <Input
                                      value={step.timeout}
                                      onChange={(e) =>
                                        updateStepAt(i, { timeout: parseFloat(e.target.value) || 10 })
                                      }
                                      inputMode="numeric"
                                      aria-label="give up after (seconds)"
                                      className="h-8 w-14"
                                    />
                                    <span>s</span>
                                  </span>
                                )}
                              </>
                            )}
                          </div>

                          {/* Test outcome, attached to its row */}
                          {tested && (
                            <div className="mt-2 flex items-center gap-3 rounded-lg border border-border bg-background/50 p-2 pl-3">
                              <span
                                className={cn(
                                  "text-xs font-medium",
                                  testResult!.ok ? "text-primary" : "text-destructive",
                                )}
                              >
                                {testResult!.ok ? "Found it on screen" : "Couldn’t find it right now"}
                              </span>
                              {testResult!.preview && (
                                <img
                                  src={testResult!.preview}
                                  alt=""
                                  className="ml-auto h-14 rounded border border-border"
                                />
                              )}
                            </div>
                          )}
                        </div>

                        <InsertBar onInsert={(t) => insertStepAt(i, t)} />
                      </div>
                    );
                  })}
                </div>
              ) : (
                <div className="flex flex-col items-center gap-4 rounded-xl border border-dashed border-border px-6 py-12 text-center">
                  <div className="flex size-12 items-center justify-center rounded-full bg-secondary text-muted-foreground">
                    <Wand2 className="size-6" />
                  </div>
                  <div>
                    <p className="text-sm font-semibold text-foreground">No actions to fine-tune</p>
                    <p className="mx-auto mt-1 max-w-xs text-sm text-muted-foreground">
                      This recording didn’t capture any actions yet. Add a step to build one by hand.
                    </p>
                  </div>
                  <div className="flex flex-wrap justify-center gap-2">
                    {INSERT_OPTIONS.map((o) => {
                      const Icon = o.Icon;
                      return (
                        <Button
                          key={o.type}
                          variant={o.detect ? "default" : "outline"}
                          size="sm"
                          onClick={() => insertStepAt(-1, o.type)}
                        >
                          <Icon className="size-4" /> {o.label}
                        </Button>
                      );
                    })}
                  </div>
                </div>
              )}
            </>
          )}
        </div>

        <div className="flex items-center justify-between gap-3 border-t border-border px-6 py-4">
          <span className="text-xs text-muted-foreground">
            {steps.length} {steps.length === 1 ? "action" : "actions"}
          </span>
          <div className="flex gap-2">
            <Button variant="outline" onClick={run} disabled={busy !== null || steps.length === 0}>
              {busy === "run" ? <Loader2 className="size-4 animate-spin" /> : <Play className="size-4" />}
              Play now
            </Button>
            <Button onClick={save} disabled={busy !== null}>
              {busy === "save" ? <Loader2 className="size-4 animate-spin" /> : <Check className="size-4" />}
              Save changes
            </Button>
          </div>
        </div>
      </SheetContent>
    </Sheet>
  );
}
