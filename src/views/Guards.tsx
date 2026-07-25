import { useCallback, useEffect, useRef, useState } from "react";
import { animate } from "animejs";
import { Loader2, Plus, Settings2, Shield, ShieldCheck } from "lucide-react";

import { getAllGuardCounts, listMacros, type MacroListItem } from "@/api";
import { useStaggerIn } from "@/lib/anime";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { GuardsSheet } from "@/components/editors/GuardsSheet";
import type { ViewProps } from "./types";

export function Guards({ navigate }: ViewProps) {
  const [macros, setMacros] = useState<MacroListItem[]>([]);
  const [counts, setCounts] = useState<Record<string, number>>({});
  const [loading, setLoading] = useState(true);
  const [managing, setManaging] = useState<string | null>(null);

  const listRef = useStaggerIn<HTMLDivElement>(macros.length);
  const heroRef = useRef<HTMLDivElement>(null);

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

  useEffect(() => {
    if (heroRef.current) {
      animate(heroRef.current, { opacity: [0, 1], translateY: [12, 0], duration: 480, ease: "out(3)" });
    }
  }, []);

  // Protected macros float to the top, then alphabetical.
  const rows = [...macros].sort((a, b) => {
    const pa = (counts[a.name] ?? 0) > 0 ? 0 : 1;
    const pb = (counts[b.name] ?? 0) > 0 ? 0 : 1;
    return pa - pb || a.name.localeCompare(b.name);
  });

  return (
    <div className="space-y-8">
      <header className="space-y-1">
        <h1 className="text-2xl font-semibold tracking-tight text-foreground">Guards</h1>
        <p className="text-sm text-muted-foreground">
          Little watchers that keep your macros alive when something interrupts them.
        </p>
      </header>

      {/* Concept band — what a guard actually does */}
      <Card
        ref={heroRef}
        className="flex-row items-start gap-4 rounded-2xl border-primary/30 bg-primary/[0.06] p-6 shadow-none sm:p-8"
      >
        <div className="flex size-12 shrink-0 items-center justify-center rounded-full bg-primary/15 text-primary">
          <ShieldCheck className="size-6" />
        </div>
        <div className="space-y-1">
          <h2 className="text-lg font-semibold text-foreground">A safety net for every macro</h2>
          <p className="max-w-2xl text-sm text-muted-foreground">
            A guard keeps an eye on the screen while your macro runs. The moment it spots trouble — say the{" "}
            <em>Reconnect</em> button — it steps in, clicks it, waits for the game to come back, and lets your loop
            carry on.
          </p>
        </div>
      </Card>

      {/* Per-macro overview */}
      <section className="space-y-4">
        <div>
          <h3 className="text-sm font-semibold text-foreground">Your macros</h3>
          <p className="text-xs text-muted-foreground">Open any one to set up the guards that watch over it.</p>
        </div>

        {loading ? (
          <div className="flex items-center justify-center gap-2 py-16 text-sm text-muted-foreground">
            <Loader2 className="size-4 animate-spin" /> Loading…
          </div>
        ) : rows.length ? (
          <div ref={listRef} className="space-y-2">
            {rows.map((m) => {
              const n = counts[m.name] ?? 0;
              const guarded = n > 0;
              return (
                <div
                  key={m.name}
                  role="button"
                  tabIndex={0}
                  onClick={() => setManaging(m.name)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" || e.key === " ") {
                      e.preventDefault();
                      setManaging(m.name);
                    }
                  }}
                  className="flex cursor-pointer items-center gap-3 rounded-lg border border-border bg-card p-3 transition-colors hover:border-primary/40 hover:bg-secondary/40"
                >
                  <div
                    className={cn(
                      "flex size-9 shrink-0 items-center justify-center rounded-md border",
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
                    variant={guarded ? "outline" : "default"}
                    size="sm"
                    onClick={(e) => {
                      e.stopPropagation();
                      setManaging(m.name);
                    }}
                  >
                    {guarded ? (
                      <>
                        <Settings2 className="size-4" /> Manage guards
                      </>
                    ) : (
                      <>
                        <Plus className="size-4" /> Add a guard
                      </>
                    )}
                  </Button>
                </div>
              );
            })}
          </div>
        ) : (
          <div className="flex flex-col items-center gap-5 rounded-2xl border border-dashed border-border px-6 py-16 text-center">
            <div className="flex size-14 items-center justify-center rounded-full bg-secondary text-muted-foreground">
              <Shield className="size-7" />
            </div>
            <div>
              <h4 className="text-lg font-semibold text-foreground">No macros yet</h4>
              <p className="mx-auto mt-1 max-w-sm text-sm text-muted-foreground">
                Record one first, then you can protect it with a guard.
              </p>
            </div>
            <Button size="lg" onClick={() => navigate("macros")}>
              <Plus className="size-4" /> Record a macro
            </Button>
          </div>
        )}
      </section>

      <GuardsSheet
        macroName={managing ?? ""}
        open={managing !== null}
        onOpenChange={(o) => !o && setManaging(null)}
        onChanged={() => void load()}
      />
    </div>
  );
}
