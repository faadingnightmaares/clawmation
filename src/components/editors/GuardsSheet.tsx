import { Fragment, useCallback, useEffect, useState } from "react";
import { Loader2, Plus, ShieldCheck } from "lucide-react";

import { guardList, guardSave, guardTest, type Guard } from "@/api";
import { notify, notifyUndo } from "@/lib/toast";
import { draftFromGuard, guardFromDraft, newTriggerDraft, type TriggerDraft } from "@/lib/triggers";
import { Button } from "@/components/ui/button";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import { TriggerEditor } from "./TriggerEditor";
import { TriggerRow } from "./TriggerRow";

export interface EditorSheetProps {
  macroName: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onChanged?: () => void;
}

/** The guards-for-one-macro state machine, shared by the Macros drawer and the
 *  in-place Protection editor. `active` gates the load: the drawer is always
 *  mounted and loads when it opens; the inline editor is mounted only while its
 *  row is open, so it passes `true` and loads on mount. Opening on a macro with
 *  no guards drops straight into the editor — an empty list with a button would
 *  just be a button you came here to press. */
function useGuardsEditor(macroName: string, active: boolean, onChanged?: () => void) {
  const [guards, setGuards] = useState<Guard[]>([]);
  const [loading, setLoading] = useState(false);
  const [editing, setEditing] = useState<TriggerDraft | null>(null);
  const [testingId, setTestingId] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const r = await guardList(macroName);
      const list = r.ok ? r.guards ?? [] : [];
      setGuards(list);
      if (!list.length) setEditing(newTriggerDraft());
    } catch {
      setGuards([]);
    } finally {
      setLoading(false);
    }
  }, [macroName]);

  useEffect(() => {
    if (active) {
      setEditing(null);
      void load();
    }
  }, [active, load]);

  const persist = useCallback(
    async (next: Guard[]) => {
      setGuards(next);
      await guardSave(macroName, next);
      onChanged?.();
    },
    [macroName, onChanged],
  );

  const saveTrigger = useCallback(
    async (guard: Guard) => {
      const exists = guards.some((g) => String(g.id) === String(guard.id));
      const next = exists ? guards.map((g) => (String(g.id) === String(guard.id) ? guard : g)) : [...guards, guard];
      await persist(next);
      setEditing(null);
      notify("success", `Guard “${String(guard.name)}” saved.`);
    },
    [guards, persist],
  );

  const toggle = useCallback(
    (g: Guard, enabled: boolean) =>
      persist(guards.map((x) => (String(x.id) === String(g.id) ? { ...x, enabled } : x))),
    [guards, persist],
  );

  const remove = useCallback(
    async (g: Guard) => {
      const before = guards;
      await persist(guards.filter((x) => String(x.id) !== String(g.id)));
      notifyUndo(`Deleted “${String(g.name) || "guard"}”.`, () => void persist(before));
    },
    [guards, persist],
  );

  const test = useCallback(async (g: Guard) => {
    setTestingId(String(g.id));
    try {
      const r = await guardTest(guardFromDraft(draftFromGuard(g), { forTest: true }));
      notify(r.ok ? "success" : "info", r.ok ? `Found “${String(g.name)}” on screen.` : r.message || "Not visible right now.");
    } catch (e) {
      notify("error", String(e));
    } finally {
      setTestingId(null);
    }
  }, []);

  const editingIsNew = editing ? !guards.some((g) => String(g.id) === editing.id) : false;

  return { guards, loading, editing, editingIsNew, testingId, setEditing, saveTrigger, toggle, remove, test };
}

/** The Macros guard editor: a slide-in drawer. Kept exactly as it was — Macros
 *  holds this up as the pattern to match, so its drawer is untouched. */
export function GuardsSheet({ macroName, open, onOpenChange, onChanged }: EditorSheetProps) {
  const { guards, loading, editing, editingIsNew, testingId, setEditing, saveTrigger, toggle, remove, test } =
    useGuardsEditor(macroName, open, onChanged);

  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent className="flex w-full flex-col gap-0 p-0 sm:max-w-lg">
        {editing ? (
          <>
            <SheetHeader className="px-6 pb-2 pr-12 pt-6">
              <SheetTitle>{editingIsNew ? "New guard" : "Edit guard"}</SheetTitle>
              <SheetDescription>Catch a problem mid-run, deal with it, then pick up where it left off.</SheetDescription>
            </SheetHeader>
            <TriggerEditor
              initial={editing}
              saveLabel="Save guard"
              onSave={saveTrigger}
              onCancel={() => setEditing(null)}
            />
          </>
        ) : (
          <>
            <SheetHeader className="px-6 pb-2 pr-12 pt-6">
              <SheetTitle>Safety guards</SheetTitle>
              <SheetDescription>
                While “{macroName}” runs, a guard watches the screen, and when trouble shows up, it steps in and
                keeps your loop alive.
              </SheetDescription>
            </SheetHeader>

            <div className="flex-1 space-y-2 overflow-y-auto px-6 py-4">
              {loading ? (
                <div className="flex items-center justify-center gap-2 py-16 text-sm text-muted-foreground">
                  <Loader2 className="size-4 animate-spin" /> Loading…
                </div>
              ) : guards.length ? (
                <div className="divide-y divide-border overflow-hidden rounded-xl border border-border bg-card">
                  {guards.map((g) => (
                    <TriggerRow
                      key={String(g.id)}
                      guard={g}
                      testing={testingId === String(g.id)}
                      onEdit={() => setEditing(draftFromGuard(g))}
                      onTest={() => test(g)}
                      onToggle={(en) => toggle(g, en)}
                      onDelete={() => remove(g)}
                    />
                  ))}
                </div>
              ) : (
                <div className="flex flex-col items-center gap-3 py-14 text-center">
                  <div className="flex size-12 items-center justify-center rounded-full bg-secondary text-muted-foreground">
                    <ShieldCheck className="size-6" />
                  </div>
                  <div>
                    <p className="text-sm font-semibold text-foreground">No guards yet</p>
                    <p className="mx-auto mt-1 max-w-xs text-sm text-muted-foreground">
                      The classic one: watch for the <em>Reconnect</em> button, click it, and carry on farming.
                    </p>
                  </div>
                </div>
              )}
            </div>

            <div className="border-t border-border px-6 py-4">
              <Button className="w-full" onClick={() => setEditing(newTriggerDraft())}>
                <Plus className="size-4" /> Add a guard
              </Button>
            </div>
          </>
        )}
      </SheetContent>
    </Sheet>
  );
}

/** The same editor, but in place — expanded under a Protection row instead of in
 *  a drawer, the way a trigger or a macro row opens onto its editor. A guard
 *  row expands onto its editor right beneath it; a new guard is a card above the
 *  list. */
export function GuardsEditor({ macroName, onChanged }: { macroName: string; onChanged?: () => void }) {
  const h = useGuardsEditor(macroName, true, onChanged);
  const title = h.editing ? (h.editingIsNew ? "New guard" : "Edit guard") : "Safety guards";
  const desc = h.editing
    ? "Catch a problem mid-run, deal with it, then pick up where it left off."
    : `While “${macroName}” runs, a guard watches the screen and steps in when trouble shows up.`;
  const openId = h.editing && !h.editingIsNew ? String(h.editing.id) : null;

  return (
    <div>
      <div className="space-y-1 border-b border-border px-4 py-3">
        <h3 className="text-sm font-semibold text-foreground">{title}</h3>
        <p className="text-xs text-muted-foreground">{desc}</p>
      </div>

      <div className="space-y-3 px-4 py-4">
        {h.loading ? (
          <div className="flex items-center justify-center gap-2 py-10 text-sm text-muted-foreground">
            <Loader2 className="size-4 animate-spin" /> Loading…
          </div>
        ) : (
          <>
            {h.editing && h.editingIsNew && (
              <div className="overflow-hidden rounded-lg border border-primary/30 bg-card">
                <TriggerEditor
                  initial={h.editing}
                  saveLabel="Save guard"
                  onSave={h.saveTrigger}
                  onCancel={() => h.setEditing(null)}
                />
              </div>
            )}

            {h.guards.length ? (
              <div className="divide-y divide-border overflow-hidden rounded-lg border border-border bg-card">
                {h.guards.map((g) => {
                  const id = String(g.id);
                  const open = openId === id;
                  return (
                    <Fragment key={id}>
                      <TriggerRow
                        guard={g}
                        testing={h.testingId === id}
                        open={open}
                        onEdit={() => h.setEditing(open ? null : draftFromGuard(g))}
                        onTest={() => h.test(g)}
                        onToggle={(en) => h.toggle(g, en)}
                        onDelete={() => h.remove(g)}
                      />
                      {open && (
                        <div className="bg-muted/40">
                          <TriggerEditor
                            initial={draftFromGuard(g)}
                            saveLabel="Save guard"
                            onSave={h.saveTrigger}
                            onCancel={() => h.setEditing(null)}
                          />
                        </div>
                      )}
                    </Fragment>
                  );
                })}
              </div>
            ) : h.editing && h.editingIsNew ? null : (
              <div className="flex flex-col items-center gap-3 rounded-lg border border-dashed border-border px-3 py-10 text-center">
                <ShieldCheck className="size-6 text-muted-foreground" />
                <p className="text-sm text-muted-foreground">
                  No guards yet. The classic: watch for <em>Reconnect</em>, click it, carry on.
                </p>
              </div>
            )}

            {!h.editing && (
              <Button variant="outline" className="w-full" onClick={() => h.setEditing(newTriggerDraft())}>
                <Plus className="size-4" /> Add a guard
              </Button>
            )}
          </>
        )}
      </div>
    </div>
  );
}
