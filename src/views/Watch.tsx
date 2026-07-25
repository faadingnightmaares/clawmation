import { Fragment, useCallback, useEffect, useState } from "react";
import { Eye, Loader2, Plus, ScanEye, Square } from "lucide-react";

import {
  guardTest,
  visionLoad,
  visionSave,
  visionStart,
  visionStatus,
  visionStop,
  type Guard,
  type VisionLogEntry,
} from "@/api";
import { useStaggerIn } from "@/lib/anime";
import { notify, notifyUndo } from "@/lib/toast";
import { cn } from "@/lib/utils";
import { draftFromGuard, guardFromDraft, newWatchDraft, type TriggerDraft } from "@/lib/triggers";
import { Button } from "@/components/ui/button";
import { TriggerEditor } from "@/components/editors/TriggerEditor";
import { TriggerRow } from "@/components/editors/TriggerRow";
import type { ViewProps } from "./types";

export function Watch(_props: ViewProps) {
  const [triggers, setTriggers] = useState<Guard[]>([]);
  const [loading, setLoading] = useState(true);
  const [running, setRunning] = useState(false);
  const [fired, setFired] = useState(0);
  const [log, setLog] = useState<VisionLogEntry[]>([]);
  const [busy, setBusy] = useState(false);
  const [editing, setEditing] = useState<TriggerDraft | null>(null);
  const [testingId, setTestingId] = useState<string | null>(null);

  const listRef = useStaggerIn<HTMLDivElement>(triggers.length);

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
      // `ok: false` means the backend was mid-stop and said "ask again"; keep
      // showing what we have rather than blanking the view for a beat.
      if (s.ok) {
        setRunning(s.running);
        setFired(s.fired);
        setLog(s.log ?? []);
      }
    } catch {
      /* backend not reachable (e.g. plain browser); leave last known state */
    }
  }, []);

  useEffect(() => {
    void load();
    void refreshStatus();
    const id = setInterval(refreshStatus, 1000);
    return () => clearInterval(id);
  }, [load, refreshStatus]);

  const enabledCount = triggers.filter((t) => t.enabled !== false).length;

  const persist = async (next: Guard[]) => {
    setTriggers(next);
    await visionSave(next);
  };

  // Save saves. Arming the watcher is the Start button's job, and folding it in
  // here meant describing a thing to look for and having it act on the screen in
  // the same click, before anyone had a chance to look at what they had written.
  const saveTrigger = async (guard: Guard) => {
    const exists = triggers.some((t) => String(t.id) === String(guard.id));
    const next = exists ? triggers.map((t) => (String(t.id) === String(guard.id) ? guard : t)) : [...triggers, guard];
    setEditing(null);
    await persist(next);
    notify("success", `“${String(guard.name)}” saved.`);
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

  const start = async () => {
    const count = enabledCount;
    setBusy(true);
    try {
      // Saved first, because the backend starts from the file rather than from
      // whatever the view is holding.
      await visionSave(triggers);
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
      // The backend is the authority on whether it actually came up: a start
      // that failed halfway must not leave the header claiming it is watching.
      void refreshStatus();
    }
  };

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
      void refreshStatus();
    }
  };

  const add = () => setEditing(newWatchDraft());
  const editingIsNew = editing ? !triggers.some((t) => String(t.id) === editing.id) : false;

  return (
    <div className="flex flex-col gap-6">
      {/* ── Header: the state of the watcher, and the one verb that changes it ── */}
      <header className="flex flex-wrap items-start justify-between gap-4">
        <div className="space-y-1">
          <h1 className="text-2xl font-semibold tracking-tight text-foreground">Watch</h1>
          <p className="max-w-xl text-sm text-muted-foreground">
            {running
              ? `Keeping an eye out for ${enabledCount} ${enabledCount === 1 ? "thing" : "things"}. Leave it running. It acts the moment one shows up.`
              : triggers.length === 0
                ? "Point Clawmation at something on screen (a button, an icon, a word) and it acts the moment that thing appears. No macro required."
                : `${enabledCount} ${enabledCount === 1 ? "thing" : "things"} ready. Press Start and leave the rest to me.`}
          </p>
        </div>

        {running ? (
          <div className="flex items-center gap-3 rounded-xl border border-primary/40 bg-primary/5 px-3 py-2">
            <span className="relative flex size-2.5">
              <span className="absolute inline-flex size-full animate-ping rounded-full bg-primary opacity-70" />
              <span className="relative inline-flex size-2.5 rounded-full bg-primary" />
            </span>
            <div className="leading-tight">
              <p className="font-mono text-lg font-semibold tabular-nums text-foreground">{fired}</p>
              <p className="text-xs text-muted-foreground">
                {fired === 1 ? "time it stepped in" : "times it stepped in"}
              </p>
            </div>
            <Button variant="ghost" size="icon" onClick={add} title="Add something to watch for">
              <Plus className="size-5" />
            </Button>
            <Button variant="outline" size="sm" onClick={stop} disabled={busy}>
              {busy ? <Loader2 className="size-4 animate-spin" /> : <Square className="size-4 fill-current" />}
              Stop
            </Button>
          </div>
        ) : triggers.length > 0 ? (
          <div className="flex items-center gap-2">
            <Button variant="ghost" size="icon" onClick={add} title="Add something to watch for">
              <Plus className="size-5" />
            </Button>
            <Button size="lg" onClick={start} disabled={busy || enabledCount === 0}>
              {busy ? <Loader2 className="size-4 animate-spin" /> : <Eye className="size-4" />}
              Start watching
            </Button>
          </div>
        ) : null}
      </header>

      {/* ── A new trigger, written right on the page instead of in a drawer ── */}
      {editing && editingIsNew && (
        <div className="overflow-hidden rounded-xl border border-primary/30 bg-card">
          <TriggerEditor
            initial={editing}
            showSurgical
            saveLabel="Save trigger"
            onSave={saveTrigger}
            onCancel={() => setEditing(null)}
          />
        </div>
      )}

      {/* ── The things being watched for ─────────────────────────────────── */}
      {loading ? (
        <div className="flex items-center justify-center gap-2 py-16 text-sm text-muted-foreground">
          <Loader2 className="size-4 animate-spin" /> Loading…
        </div>
      ) : triggers.length ? (
        <div ref={listRef} className="divide-y divide-border overflow-hidden rounded-xl border border-border bg-card">
          {triggers.map((t) => {
            const id = String(t.id);
            // The row opens onto its editor right beneath it, the way a macro row
            // does — no drawer sliding in over the page. Clicking the row again
            // (or Cancel) folds it back up.
            const open = !!editing && !editingIsNew && String(editing.id) === id;
            return (
              <Fragment key={id}>
                <TriggerRow
                  guard={t}
                  testing={testingId === id}
                  open={open}
                  onEdit={() => setEditing(open ? null : draftFromGuard(t))}
                  onTest={() => test(t)}
                  onToggle={(en) => toggle(t, en)}
                  onDelete={() => remove(t)}
                />
                {open && (
                  <div className="bg-muted/40">
                    <TriggerEditor
                      initial={draftFromGuard(t)}
                      showSurgical
                      saveLabel="Save trigger"
                      onSave={saveTrigger}
                      onCancel={() => setEditing(null)}
                    />
                  </div>
                )}
              </Fragment>
            );
          })}
        </div>
      ) : editing && editingIsNew ? null : (
        <div className="flex flex-col items-center gap-5 rounded-xl border border-border bg-card px-6 py-14 text-center">
          <div className="flex size-14 items-center justify-center rounded-full bg-secondary text-muted-foreground">
            <ScanEye className="size-7" />
          </div>
          <div>
            <h2 className="text-lg font-semibold text-foreground">Nothing to watch for yet</h2>
            <p className="mx-auto mt-1 max-w-sm text-sm text-muted-foreground">
              A trigger is one thing on screen plus what to do about it: spot the button, click the button.
            </p>
          </div>
          <ol className="mx-auto flex max-w-md flex-col gap-2 text-left text-sm text-muted-foreground">
            <NumStep n={1}>Show me what to look for: a colour, a picture of a button, or some words.</NumStep>
            <NumStep n={2}>Say what to do when it appears: click it, press a key, or nudge the mouse.</NumStep>
            <NumStep n={3}>Press Start and leave it running while you do something else.</NumStep>
          </ol>
          <Button size="lg" onClick={add}>
            <Plus className="size-4" /> Add the first thing to watch for
          </Button>
        </div>
      )}

      {/* ── What it has actually done, so "is this working?" has an answer ── */}
      {running && log.length > 0 && (
        <section className="flex flex-col gap-2">
          <h2 className="text-sm font-medium text-foreground">Just now</h2>
          <ul className="divide-y divide-border overflow-hidden rounded-xl border border-border bg-card">
            {log.slice(0, 6).map((e, i) => (
              <li key={`${i}-${e.msg}`} className="flex items-center gap-3 px-4 py-2">
                <span
                  className={cn(
                    "size-1.5 shrink-0 rounded-full",
                    e.kind === "act" ? "bg-primary" : "bg-muted-foreground/40",
                  )}
                />
                <span className="truncate text-sm text-muted-foreground">{humanEvent(e.msg)}</span>
              </li>
            ))}
          </ul>
        </section>
      )}

    </div>
  );
}

/** The agent writes its feed for a log file: `'Name' -> clicked (x, y)`. This
 *  is the one place that shape is spoken aloud, so it is translated here rather
 *  than in the engine, and anything unrecognised passes through untouched. */
function humanEvent(msg: string): string {
  const m = /^'(.*)' -> (.*)$/.exec(msg);
  if (!m) return msg;
  const [, name, what] = m;
  if (what.startsWith("clicked")) return `Clicked “${name}”.`;
  if (what.startsWith("dragged")) return `Dragged across “${name}”.`;
  if (what.startsWith("pressed ")) return `Pressed ${what.slice(8)} for “${name}”.`;
  if (what.startsWith("nudged")) return `Nudged the mouse for “${name}”.`;
  if (what.startsWith("running ")) return `“${name}” appeared: ${what}.`;
  return `“${name}”: ${what}`;
}

function NumStep({ n, children }: { n: number; children: React.ReactNode }) {
  return (
    <li className="flex gap-3">
      <span className="flex size-5 shrink-0 items-center justify-center rounded-full bg-primary/15 text-xs font-semibold text-primary">
        {n}
      </span>
      {children}
    </li>
  );
}
