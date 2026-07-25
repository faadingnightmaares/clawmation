import { useCallback, useEffect, useState } from "react";
import { Loader2, Plus, Shield, ShieldCheck } from "lucide-react";

import { getAllGuardCounts, listMacros, type MacroListItem } from "@/api";
import { useStaggerIn } from "@/lib/anime";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { GuardsSheet } from "@/components/editors/GuardsSheet";
import type { ViewProps } from "./types";

/** The protection half of Autopilot: one row per macro, unprotected ones last.
 *  `Autopilot` owns the page header; this owns its own section heading. */
export function Guards({ navigate }: ViewProps) {
  const [macros, setMacros] = useState<MacroListItem[]>([]);
  const [counts, setCounts] = useState<Record<string, number>>({});
  const [loading, setLoading] = useState(true);
  const [managing, setManaging] = useState<string | null>(null);

  const listRef = useStaggerIn<HTMLDivElement>(macros.length);

  const load = useCallback(async () => {
    setLoading(true);
    const macrosP = listMacros();
    const countsP = getAllGuardCounts();
    try {
      setMacros(await macrosP);
    } catch {
      setMacros([]);
    }
    try {
      const c = await countsP;
      setCounts(c.ok ? c.counts : {});
    } catch {
      setCounts({});
    }
    setLoading(false);
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  // Protected macros float to the top, then alphabetical.
  const rows = [...macros].sort((a, b) => {
    const pa = (counts[a.name] ?? 0) > 0 ? 0 : 1;
    const pb = (counts[b.name] ?? 0) > 0 ? 0 : 1;
    return pa - pb || a.name.localeCompare(b.name);
  });
  const protectedCount = macros.filter((m) => (counts[m.name] ?? 0) > 0).length;

  return (
    <section className="flex flex-col gap-3">
      <div>
        <h2 className="text-sm font-semibold text-foreground">Protection</h2>
        <p className="text-xs text-muted-foreground">
          {macros.length === 0
            ? "A guard watches the screen while a macro runs and steps in when trouble shows up."
            : protectedCount === 0
              ? "Nothing is protected yet. Open a macro to give it a guard that clicks Reconnect for you."
              : `${protectedCount} of ${macros.length} macros have a guard watching over them.`}
        </p>
      </div>

      {loading ? (
        <div className="flex items-center justify-center gap-2 py-16 text-sm text-muted-foreground">
          <Loader2 className="size-4 animate-spin" /> Loading…
        </div>
      ) : rows.length ? (
        <div ref={listRef} className="divide-y divide-border overflow-hidden rounded-xl border border-border bg-card">
          {rows.map((m) => {
            const n = counts[m.name] ?? 0;
            const guarded = n > 0;
            return (
              <div key={m.name} className="group relative">
                <button
                  type="button"
                  onClick={() => setManaging(m.name)}
                  className="absolute inset-0 z-0 outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
                >
                  <span className="sr-only">
                    {guarded ? "Manage guards for" : "Add a guard to"} {m.name}
                  </span>
                </button>

                <div className="pointer-events-none relative z-10 flex items-center gap-3 px-4 py-3">
                  <div
                    className={cn(
                      "flex size-9 shrink-0 items-center justify-center rounded-lg border",
                      guarded
                        ? "border-primary/30 bg-primary/10 text-primary"
                        : "border-border bg-secondary/50 text-muted-foreground",
                    )}
                  >
                    {guarded ? <ShieldCheck className="size-4" /> : <Shield className="size-4" />}
                  </div>

                  <div className="min-w-0 flex-1">
                    <p className="truncate text-sm font-medium text-foreground">{m.name}</p>
                    <p className={cn("truncate text-xs", guarded ? "text-primary" : "text-muted-foreground")}>
                      {guarded ? `${n} guard${n === 1 ? "" : "s"} watching` : "Not protected yet"}
                    </p>
                  </div>

                  <Button
                    variant="ghost"
                    size="sm"
                    className="pointer-events-auto shrink-0 text-muted-foreground opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100"
                    onClick={() => setManaging(m.name)}
                  >
                    {guarded ? "Manage" : "Add a guard"}
                  </Button>
                </div>
              </div>
            );
          })}
        </div>
      ) : (
        <div className="flex flex-col items-center gap-4 rounded-xl border border-border bg-card px-6 py-12 text-center">
          <div className="flex size-12 items-center justify-center rounded-full bg-secondary text-muted-foreground">
            <Shield className="size-6" />
          </div>
          <div>
            <h3 className="text-base font-semibold text-foreground">No macros to protect yet</h3>
            <p className="mx-auto mt-1 max-w-sm text-sm text-muted-foreground">
              Record one first, then a guard can watch over it while it runs.
            </p>
          </div>
          <Button onClick={() => navigate("macros")}>
            <Plus className="size-4" /> Record a macro
          </Button>
        </div>
      )}

      <GuardsSheet
        macroName={managing ?? ""}
        open={managing !== null}
        onOpenChange={(o) => !o && setManaging(null)}
        onChanged={() => void load()}
      />
    </section>
  );
}
