import { useEffect, useMemo, useState } from "react";
import {
  CaretDown,
  CaretUp,
  DotsSixVertical,
  LinkSimple,
  Plus,
  SpinnerGap,
  Trash,
} from "@phosphor-icons/react";

import type { Chain, MacroListItem } from "@/api";
import { fmtDur } from "@/format";
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

export interface ChainDraft {
  name: string;
  macroNames: string[];
  delayBetween: number;
  repeat: number;
}

interface ChainComposerProps {
  chain: Chain;
  macros: MacroListItem[];
  disabled?: boolean;
  onSave: (chainId: string, draft: ChainDraft) => Promise<boolean>;
}

function draftFromChain(chain: Chain): ChainDraft {
  return {
    name: String(chain.name || "Untitled chain"),
    macroNames: Array.isArray(chain.macro_names)
      ? chain.macro_names.map(String)
      : [],
    delayBetween: Math.max(0, Number(chain.delay_between ?? 1)),
    repeat: Math.max(0, Math.floor(Number(chain.repeat ?? 1))),
  };
}

function move<T>(items: T[], from: number, to: number): T[] {
  if (from === to || to < 0 || to >= items.length) return items;
  const next = [...items];
  const [item] = next.splice(from, 1);
  next.splice(to, 0, item);
  return next;
}

export function ChainComposer({
  chain,
  macros,
  disabled = false,
  onSave,
}: ChainComposerProps) {
  const chainId = String(chain.id || "");
  const [draft, setDraft] = useState<ChainDraft>(() => draftFromChain(chain));
  const [baseline, setBaseline] = useState<ChainDraft>(() =>
    draftFromChain(chain),
  );
  const [macroToAdd, setMacroToAdd] = useState("");
  const [dragIndex, setDragIndex] = useState<number | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    const next = draftFromChain(chain);
    setDraft(next);
    setBaseline(next);
    setMacroToAdd("");
    setDragIndex(null);
  }, [chain]);

  const dirty = JSON.stringify(draft) !== JSON.stringify(baseline);
  const macroByName = useMemo(
    () => new Map(macros.map((macro) => [macro.name, macro])),
    [macros],
  );

  const patch = (value: Partial<ChainDraft>) => {
    setDraft((current) => ({ ...current, ...value }));
  };

  const addMacro = (name: string) => {
    if (!name) return;
    patch({ macroNames: [...draft.macroNames, name] });
    setMacroToAdd("");
  };

  const save = async () => {
    if (!chainId || !draft.name.trim() || saving || !dirty) return;
    setSaving(true);
    try {
      const normalized: ChainDraft = {
        name: draft.name.trim(),
        macroNames: draft.macroNames,
        delayBetween: Math.max(0, Number(draft.delayBetween) || 0),
        repeat: Math.max(0, Math.floor(Number(draft.repeat) || 0)),
      };
      if (await onSave(chainId, normalized)) {
        setDraft(normalized);
        setBaseline(normalized);
      }
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="grid gap-4">
      <label className="grid gap-1.5 text-xs text-muted-foreground">
        Chain name
        <Input
          value={draft.name}
          disabled={disabled || saving}
          onChange={(event) => patch({ name: event.target.value })}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              void save();
            }
          }}
        />
      </label>

      <section aria-label="Chain sequence" className="overflow-hidden rounded-lg border border-border">
        <div className="flex items-center justify-between gap-3 border-b border-border bg-muted/25 px-3 py-2.5">
          <div className="flex items-center gap-2">
            <LinkSimple className="size-4 text-primary" weight="duotone" />
            <p className="text-xs font-semibold text-foreground">Sequence</p>
          </div>
          <p className="text-[10px] tabular-nums text-muted-foreground">
            {draft.macroNames.length} macro
            {draft.macroNames.length === 1 ? "" : "s"}
          </p>
        </div>

        {draft.macroNames.length > 0 ? (
          <ol className="divide-y divide-border">
            {draft.macroNames.map((name, index) => {
              const macro = macroByName.get(name);
              return (
                <li
                  key={`${name}-${index}`}
                  draggable={!disabled && !saving}
                  className={cn(
                    "group grid min-h-12 grid-cols-[auto_auto_minmax(0,1fr)_auto] items-center gap-2 bg-background px-2.5 py-2 transition-colors",
                    dragIndex === index && "bg-primary/[0.07]",
                  )}
                  onDragStart={() => setDragIndex(index)}
                  onDragEnd={() => setDragIndex(null)}
                  onDragOver={(event) => event.preventDefault()}
                  onDrop={(event) => {
                    event.preventDefault();
                    if (dragIndex === null) return;
                    patch({
                      macroNames: move(draft.macroNames, dragIndex, index),
                    });
                    setDragIndex(null);
                  }}
                >
                  <DotsSixVertical
                    className="size-4 cursor-grab text-muted-foreground/60 active:cursor-grabbing"
                    aria-hidden="true"
                  />
                  <span className="grid size-6 place-items-center rounded-md bg-primary/10 text-[10px] font-semibold tabular-nums text-primary">
                    {index + 1}
                  </span>
                  <div className="min-w-0">
                    <p className="truncate text-xs font-medium text-foreground">
                      {name}
                    </p>
                    <p className="mt-0.5 text-[10px] text-muted-foreground">
                      {macro
                        ? `${macro.events.toLocaleString()} actions · ${fmtDur(macro.duration ?? 0)}`
                        : "Macro is missing"}
                    </p>
                  </div>
                  <div className="flex items-center">
                    <Button
                      variant="ghost"
                      size="icon-xs"
                      aria-label={`Move ${name} up`}
                      disabled={disabled || saving || index === 0}
                      onClick={() =>
                        patch({
                          macroNames: move(draft.macroNames, index, index - 1),
                        })
                      }
                    >
                      <CaretUp className="size-3.5" />
                    </Button>
                    <Button
                      variant="ghost"
                      size="icon-xs"
                      aria-label={`Move ${name} down`}
                      disabled={
                        disabled ||
                        saving ||
                        index === draft.macroNames.length - 1
                      }
                      onClick={() =>
                        patch({
                          macroNames: move(draft.macroNames, index, index + 1),
                        })
                      }
                    >
                      <CaretDown className="size-3.5" />
                    </Button>
                    <Button
                      variant="ghost"
                      size="icon-xs"
                      aria-label={`Remove ${name}`}
                      className="text-muted-foreground hover:text-destructive"
                      disabled={disabled || saving}
                      onClick={() =>
                        patch({
                          macroNames: draft.macroNames.filter(
                            (_, itemIndex) => itemIndex !== index,
                          ),
                        })
                      }
                    >
                      <Trash className="size-3.5" />
                    </Button>
                  </div>
                </li>
              );
            })}
          </ol>
        ) : (
          <div className="px-4 py-6 text-center">
            <p className="text-xs font-medium text-foreground">
              Add the first macro
            </p>
            <p className="mt-1 text-[10px] leading-4 text-muted-foreground">
              Macros run from top to bottom.
            </p>
          </div>
        )}

        <div className="border-t border-border bg-muted/15 p-2.5">
          <Select
            value={macroToAdd}
            onValueChange={addMacro}
            disabled={disabled || saving || macros.length === 0}
          >
            <SelectTrigger className="w-full bg-background" aria-label="Add macro to chain">
              <Plus className="size-4 text-primary" weight="bold" />
              <SelectValue placeholder="Add macro…" />
            </SelectTrigger>
            <SelectContent>
              {macros.map((macro) => (
                <SelectItem key={macro.name} value={macro.name}>
                  {macro.name}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      </section>

      <div className="grid grid-cols-2 gap-3">
        <label className="grid gap-1.5 text-xs text-muted-foreground">
          Delay between
          <div className="relative">
            <Input
              type="number"
              min={0}
              step={0.1}
              className="pr-8"
              value={draft.delayBetween}
              disabled={disabled || saving}
              onChange={(event) =>
                patch({ delayBetween: Number(event.target.value) })
              }
            />
            <span className="pointer-events-none absolute right-3 top-1/2 -translate-y-1/2 text-[10px] text-muted-foreground">
              sec
            </span>
          </div>
        </label>
        <label className="grid gap-1.5 text-xs text-muted-foreground">
          Repeat chain
          <Input
            type="number"
            min={0}
            step={1}
            value={draft.repeat}
            disabled={disabled || saving}
            onChange={(event) => patch({ repeat: Number(event.target.value) })}
          />
          <span className="text-[9px] leading-3 text-muted-foreground">
            0 runs until stopped
          </span>
        </label>
      </div>

      <div className="flex items-center justify-between gap-3 border-t border-border pt-3">
        <p className="text-[10px] text-muted-foreground">
          {dirty ? "Unsaved chain changes" : "Chain is saved"}
        </p>
        <Button
          size="sm"
          disabled={
            disabled || saving || !dirty || !draft.name.trim() || !chainId
          }
          onClick={() => void save()}
        >
          {saving && <SpinnerGap className="size-4 animate-spin" />}
          Save chain
        </Button>
      </div>
    </div>
  );
}

