import { useCallback, useEffect, useState } from "react";
import { GitBranch, SpinnerGap } from "@phosphor-icons/react";

import { listMacros, type MacroListItem } from "@/api";
import { NodeGraphEditor } from "@/components/nodes/NodeGraphEditor";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import type { ViewProps } from "./types";

export function Nodes(_props: ViewProps) {
  const [macros, setMacros] = useState<MacroListItem[]>([]);
  const [selectedName, setSelectedName] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");

  const load = useCallback(async () => {
    setLoading(true);
    setError("");
    try {
      const next = (await listMacros()) ?? [];
      setMacros(next);
      setSelectedName((current) => {
        if (current && next.some((macro) => macro.name === current)) return current;
        return next[0]?.name ?? null;
      });
    } catch (loadError) {
      setError(String(loadError));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  return (
    <div className="grid h-full min-h-0 w-full grid-cols-[220px_minmax(0,1fr)] overflow-hidden bg-background">
      <aside className="flex min-h-0 flex-col border-r border-border bg-card">
        <div className="shrink-0 border-b border-border px-4 py-3">
          <div className="flex items-center gap-2">
            <GitBranch className="size-4 text-primary" weight="bold" />
            <h1 className="text-sm font-semibold text-foreground">Nodes</h1>
          </div>
          <p className="mt-1 text-xs text-muted-foreground">
            {macros.length} {macros.length === 1 ? "macro" : "macros"}
          </p>
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto p-2">
          {loading ? (
            <div className="flex items-center gap-2 px-2 py-3 text-xs text-muted-foreground">
              <SpinnerGap className="size-4 animate-spin" />
              Loading macros
            </div>
          ) : error ? (
            <div className="p-2">
              <p className="text-xs text-destructive">Couldn&apos;t load macros.</p>
              <Button className="mt-3" variant="outline" size="sm" onClick={() => void load()}>
                Retry
              </Button>
            </div>
          ) : macros.length === 0 ? (
            <p className="px-2 py-3 text-xs leading-5 text-muted-foreground">
              Record or create a macro first. Its graph will appear here.
            </p>
          ) : (
            <div className="grid gap-1">
              {macros.map((macro) => {
                const active = macro.name === selectedName;
                return (
                  <button
                    key={macro.name}
                    type="button"
                    onClick={() => setSelectedName(macro.name)}
                    aria-current={active ? "page" : undefined}
                    className={cn(
                      "w-full rounded-md px-2.5 py-2 text-left outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring",
                      active
                        ? "bg-primary/10 text-foreground"
                        : "text-muted-foreground hover:bg-muted hover:text-foreground",
                    )}
                  >
                    <span className="block truncate text-xs font-medium">{macro.name}</span>
                    <span className="mt-0.5 block text-[10px] text-muted-foreground">
                      {macro.events} {macro.events === 1 ? "event" : "events"}
                    </span>
                  </button>
                );
              })}
            </div>
          )}
        </div>
      </aside>

      <section className="min-h-0 min-w-0">
        {selectedName ? (
          <NodeGraphEditor key={selectedName} macroName={selectedName} onChanged={load} />
        ) : (
          <div className="grid h-full place-items-center px-8 text-center">
            <div>
              <GitBranch className="mx-auto size-10 text-muted-foreground/40" />
              <p className="mt-3 text-sm font-medium text-foreground">No graph selected</p>
              <p className="mt-1 text-xs text-muted-foreground">
                Choose a macro from the library to build its node graph.
              </p>
            </div>
          </div>
        )}
      </section>
    </div>
  );
}
