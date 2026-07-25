import { useCallback, useEffect, useState } from "react";
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

export function GuardsSheet({ macroName, open, onOpenChange, onChanged }: EditorSheetProps) {
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
      // Opening guards on a macro that has none can only mean adding one, so land
      // in the editor instead of on an empty list with a button. Cancel goes back.
      if (!list.length) setEditing(newTriggerDraft());
    } catch {
      setGuards([]);
    } finally {
      setLoading(false);
    }
  }, [macroName]);

  useEffect(() => {
    if (open) {
      setEditing(null);
      void load();
    }
  }, [open, load]);

  const persist = async (next: Guard[]) => {
    setGuards(next);
    await guardSave(macroName, next);
    onChanged?.();
  };

  const saveTrigger = async (guard: Guard) => {
    const exists = guards.some((g) => String(g.id) === String(guard.id));
    const next = exists ? guards.map((g) => (String(g.id) === String(guard.id) ? guard : g)) : [...guards, guard];
    await persist(next);
    setEditing(null);
    notify("success", `Guard “${String(guard.name)}” saved.`);
  };

  const toggle = (g: Guard, enabled: boolean) =>
    persist(guards.map((x) => (String(x.id) === String(g.id) ? { ...x, enabled } : x)));

  const remove = async (g: Guard) => {
    const before = guards;
    await persist(guards.filter((x) => String(x.id) !== String(g.id)));
    notifyUndo(`Deleted “${String(g.name) || "guard"}”.`, () => void persist(before));
  };

  const test = async (g: Guard) => {
    setTestingId(String(g.id));
    try {
      const r = await guardTest(guardFromDraft(draftFromGuard(g), { forTest: true }));
      notify(r.ok ? "success" : "info", r.ok ? `Found “${String(g.name)}” on screen.` : r.message || "Not visible right now.");
    } catch (e) {
      notify("error", String(e));
    } finally {
      setTestingId(null);
    }
  };

  const editingIsNew = editing ? !guards.some((g) => String(g.id) === editing.id) : false;

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
