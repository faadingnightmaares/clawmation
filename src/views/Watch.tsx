import { useCallback, useEffect, useRef, useState } from "react";
import { animate } from "animejs";
import { Eye, Loader2, Plus, ScanEye, Square } from "lucide-react";

import {
  guardTest,
  visionLoad,
  visionSave,
  visionStart,
  visionStatus,
  visionStop,
  type Guard,
} from "@/api";
import { useStaggerIn } from "@/lib/anime";
import { notify, notifyUndo } from "@/lib/toast";
import { cn } from "@/lib/utils";
import { draftFromGuard, guardFromDraft, newTriggerDraft, type TriggerDraft } from "@/lib/triggers";
import { Button } from "@/components/ui/button";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import { TriggerEditor } from "@/components/editors/TriggerEditor";
import { TriggerRow } from "@/components/editors/TriggerRow";
import type { ViewProps } from "./types";

export function Watch(_props: ViewProps) {
  const [triggers, setTriggers] = useState<Guard[]>([]);
  const [loading, setLoading] = useState(true);
  const [running, setRunning] = useState(false);
  const [fired, setFired] = useState(0);
  const [busy, setBusy] = useState(false);
  const [editing, setEditing] = useState<TriggerDraft | null>(null);
  const [testingId, setTestingId] = useState<string | null>(null);

  const listRef = useStaggerIn<HTMLDivElement>(triggers.length);
  const heroRef = useRef<HTMLDivElement>(null);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const r = await visionLoad();
      setTriggers(r.ok ? r.triggers ?? [] : []);
    } catch {
      setTriggers([]);
    } finally {
      setLoading(false);
    }
  }, []);

  const refreshStatus = useCallback(async () => {
    try {
      const s = await visionStatus();
      if (s.ok) {
        setRunning(s.running);
        setFired(s.fired);
      }
    } catch {
      /* backend not reachable (e.g. plain browser) — leave last known state */
    }
  }, []);

  useEffect(() => {
    void load();
    void refreshStatus();
    const id = setInterval(refreshStatus, 1000);
    return () => clearInterval(id);
  }, [load, refreshStatus]);

  useEffect(() => {
    if (heroRef.current) {
      animate(heroRef.current, { opacity: [0, 1], translateY: [12, 0], duration: 480, ease: "out(3)" });
    }
  }, []);

  const enabledCount = triggers.filter((t) => t.enabled !== false).length;

  const persist = async (next: Guard[]) => {
    setTriggers(next);
    await visionSave(next);
  };

  // Saving while nothing is running arms the watcher in the same click. Otherwise
  // the trigger is saved, the sheet closes, and the user still has to find the
  // hero button and press Start — which is what they wanted all along.
  const saveTrigger = async (guard: Guard) => {
    const exists = triggers.some((t) => String(t.id) === String(guard.id));
    const next = exists ? triggers.map((t) => (String(t.id) === String(guard.id) ? guard : t)) : [...triggers, guard];
    setEditing(null);
    if (running) {
      await persist(next);
      notify("success", `“${String(guard.name)}” saved.`);
      return;
    }
    setTriggers(next);
    await startWith(next);
  };

  const toggle = (t: Guard, enabled: boolean) =>
    persist(triggers.map((x) => (String(x.id) === String(t.id) ? { ...x, enabled } : x)));

  const remove = async (t: Guard) => {
    const before = triggers;
    await persist(triggers.filter((x) => String(x.id) !== String(t.id)));
    notifyUndo(`Deleted “${String(t.name) || "trigger"}”.`, () => void persist(before));
  };

  const test = async (t: Guard) => {
    setTestingId(String(t.id));
    try {
      const r = await guardTest(guardFromDraft(draftFromGuard(t), { forTest: true }));
      notify(r.ok ? "success" : "info", r.ok ? `Found “${String(t.name)}” on screen.` : r.message || "Not visible right now.");
    } catch (e) {
      notify("error", String(e));
    } finally {
      setTestingId(null);
    }
  };

  // Takes the list explicitly: a save-and-start passes the trigger it just added,
  // which the `triggers` state hasn't caught up to yet.
  const startWith = async (list: Guard[]) => {
    const count = list.filter((t) => t.enabled !== false).length;
    setBusy(true);
    try {
      await visionSave(list);
      const r = await visionStart();
      if (r.ok) {
        setRunning(true);
        notify("success", `Watching for ${count} ${count === 1 ? "thing" : "things"}.`);
      } else {
        notify("error", r.error || "Couldn’t start watching.");
      }
    } catch (e) {
      notify("error", String(e));
    } finally {
      setBusy(false);
    }
  };

  const start = () => startWith(triggers);

  const stop = async () => {
    setBusy(true);
    try {
      await visionStop();
      setRunning(false);
      notify("info", "Stopped watching.");
    } catch (e) {
      notify("error", String(e));
    } finally {
      setBusy(false);
    }
  };

  const editingIsNew = editing ? !triggers.some((t) => String(t.id) === editing.id) : false;

  return (
    <div className="space-y-8">
      <header className="space-y-1">
        <h1 className="text-2xl font-semibold tracking-tight text-foreground">Watch</h1>
        <p className="text-sm text-muted-foreground">
          Let Clawmation keep an eye on the screen and react on its own — no macro required.
        </p>
      </header>

      {/* Hero — the on/off heart of the view */}
      <div
        ref={heroRef}
        className={cn(
          "relative overflow-hidden rounded-2xl border p-8",
          running ? "border-primary/40 bg-primary/[0.07]" : "border-border bg-card",
        )}
      >
        <div className="flex flex-col items-start gap-6 sm:flex-row sm:items-center sm:justify-between">
          <div className="flex items-start gap-4">
            <div
              className={cn(
                "flex size-14 shrink-0 items-center justify-center rounded-full",
                running ? "bg-primary/15 text-primary" : "bg-secondary text-muted-foreground",
              )}
            >
              {running ? <ScanEye className="size-7" /> : <Eye className="size-7" />}
            </div>
            <div>
              <div className="flex items-center gap-2">
                <h2 className="text-lg font-semibold text-foreground">
                  {running ? "Keeping watch" : "Ready when you are"}
                </h2>
                {running && (
                  <span className="relative flex size-2.5">
                    <span className="absolute inline-flex size-full animate-ping rounded-full bg-primary opacity-70" />
                    <span className="relative inline-flex size-2.5 rounded-full bg-primary" />
                  </span>
                )}
              </div>
              <p className="mt-1 max-w-md text-sm text-muted-foreground">
                {running
                  ? fired > 0
                    ? `Stepped in ${fired} ${fired === 1 ? "time" : "times"} so far.`
                    : "Watching every second. I’ll act the moment I spot something."
                  : enabledCount > 0
                    ? `${enabledCount} ${enabledCount === 1 ? "thing" : "things"} ready to watch for.`
                    : "Add something to watch for to get started."}
              </p>
            </div>
          </div>

          {running ? (
            <Button size="lg" variant="secondary" onClick={stop} disabled={busy}>
              {busy ? <Loader2 className="size-4 animate-spin" /> : <Square className="size-4" />}
              Stop watching
            </Button>
          ) : (
            <Button size="lg" onClick={start} disabled={busy || enabledCount === 0}>
              {busy ? <Loader2 className="size-4 animate-spin" /> : <Eye className="size-4" />}
              Start watching
            </Button>
          )}
        </div>
      </div>

      {/* Triggers */}
      <section className="space-y-4">
        <div className="flex items-center justify-between">
          <div>
            <h3 className="text-sm font-semibold text-foreground">Things to watch for</h3>
            <p className="text-xs text-muted-foreground">Each one watches the screen and acts on its own.</p>
          </div>
          {triggers.length > 0 && (
            <Button variant="outline" size="sm" onClick={() => setEditing(newTriggerDraft())}>
              <Plus className="size-4" /> Add
            </Button>
          )}
        </div>

        {loading ? (
          <div className="flex items-center justify-center gap-2 py-16 text-sm text-muted-foreground">
            <Loader2 className="size-4 animate-spin" /> Loading…
          </div>
        ) : triggers.length ? (
          <div ref={listRef} className="space-y-2">
            {triggers.map((t) => (
              <TriggerRow
                key={String(t.id)}
                guard={t}
                testing={testingId === String(t.id)}
                onEdit={() => setEditing(draftFromGuard(t))}
                onTest={() => test(t)}
                onToggle={(en) => toggle(t, en)}
                onDelete={() => remove(t)}
              />
            ))}
          </div>
        ) : (
          <div className="flex flex-col items-center gap-5 rounded-2xl border border-dashed border-border px-6 py-16 text-center">
            <div className="flex size-14 items-center justify-center rounded-full bg-secondary text-muted-foreground">
              <ScanEye className="size-7" />
            </div>
            <div>
              <h4 className="text-lg font-semibold text-foreground">Nothing to watch for yet</h4>
              <p className="mx-auto mt-1 max-w-sm text-sm text-muted-foreground">
                Point Clawmation at something on screen — a button, an icon, a word — and tell it what to do when it
                appears. It’ll handle the rest while you’re away.
              </p>
            </div>
            <Button size="lg" onClick={() => setEditing(newTriggerDraft())}>
              <Plus className="size-4" /> Add the first thing to watch for
            </Button>
          </div>
        )}
      </section>

      {/* Editor */}
      <Sheet open={!!editing} onOpenChange={(o) => !o && setEditing(null)}>
        <SheetContent className="flex w-full flex-col gap-0 p-0 sm:max-w-lg">
          {editing && (
            <>
              <SheetHeader className="px-6 pb-2 pr-12 pt-6">
                <SheetTitle>{editingIsNew ? "New trigger" : "Edit trigger"}</SheetTitle>
                <SheetDescription>Tell Clawmation what to watch for and what to do when it appears.</SheetDescription>
              </SheetHeader>
              <TriggerEditor
                initial={editing}
                showSurgical
                saveLabel={running ? "Save trigger" : "Save & start watching"}
                onSave={saveTrigger}
                onCancel={() => setEditing(null)}
              />
            </>
          )}
        </SheetContent>
      </Sheet>
    </div>
  );
}
