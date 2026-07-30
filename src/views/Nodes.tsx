import { useCallback, useEffect, useState } from "react";
import {
  BookOpenText,
  DownloadSimple,
  Plus,
  SpinnerGap,
} from "@phosphor-icons/react";

import {
  listChains,
  listMacros,
  importLoop,
  nodeGraphCreate,
  nodeGraphDelete,
  nodeGraphList,
  nodeGraphRename,
  nodeGraphSave,
  type Chain,
  type MacroListItem,
  type NodeLoopItem,
} from "@/api";
import { NodeGraphEditor } from "@/components/nodes/NodeGraphEditor";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import {
  LOOP_TEMPLATES,
  createLoopTemplateGraph,
  type LoopTemplateId,
} from "@/lib/nodeGraphTemplates";
import { notify } from "@/lib/toast";
import { VIEW_ICONS, VIEW_ICON_STROKE_WIDTH } from "@/nav";
import type { ViewProps } from "./types";

const LoopsIcon = VIEW_ICONS.nodes;

export function Nodes({ status, active = true }: ViewProps) {
  const [loops, setLoops] = useState<NodeLoopItem[]>([]);
  const [macros, setMacros] = useState<MacroListItem[]>([]);
  const [chains, setChains] = useState<Chain[]>([]);
  const [selectedName, setSelectedName] = useState<string | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<string | null>(null);
  const [emptyMenu, setEmptyMenu] = useState<{ left: number; top: number } | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  const load = useCallback(async (preferredName?: string) => {
    setLoading(true);
    setError("");
    try {
      const [nextLoops, nextMacros, nextChains] = await Promise.all([
        nodeGraphList(),
        listMacros(),
        listChains(),
      ]);
      setLoops(nextLoops);
      setMacros(nextMacros);
      setChains(nextChains ?? []);
      setSelectedName((current) => {
        const preferred = preferredName || current;
        if (preferred && nextLoops.some((loop) => loop.name === preferred)) return preferred;
        return nextLoops[0]?.name ?? null;
      });
    } catch (loadError) {
      setError(String(loadError));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    const pending = localStorage.getItem("clawmation:pending-loop-selection");
    if (pending) localStorage.removeItem("clawmation:pending-loop-selection");
    void load(pending || undefined);
  }, [load]);

  useEffect(() => {
    if (active) return;
    setDeleteTarget(null);
    setEmptyMenu(null);
  }, [active]);

  useEffect(() => {
    if (!active || !emptyMenu) return;
    const close = () => setEmptyMenu(null);
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") close();
    };
    window.addEventListener("pointerdown", close);
    window.addEventListener("keydown", closeOnEscape);
    return () => {
      window.removeEventListener("pointerdown", close);
      window.removeEventListener("keydown", closeOnEscape);
    };
  }, [active, emptyMenu]);

  const createLoop = async (templateId?: LoopTemplateId) => {
    setBusy(true);
    let createdName: string | null = null;
    try {
      const template = templateId
        ? LOOP_TEMPLATES.find((candidate) => candidate.id === templateId)
        : undefined;
      const result = await nodeGraphCreate(template?.name ?? "Loop");
      if (!result.ok || !result.name) {
        notify("error", result.error || "Couldn’t create the Loop.");
        return;
      }
      createdName = result.name;
      if (templateId) {
        const saved = await nodeGraphSave(
          result.name,
          createLoopTemplateGraph(templateId, result.name),
        );
        if (!saved.ok) {
          await nodeGraphDelete(result.name);
          createdName = null;
          notify("error", saved.error || "Could not create the Loop template.");
          return;
        }
      }
      setEmptyMenu(null);
      await load(result.name);
    } catch (createError) {
      if (createdName && templateId) {
        try {
          await nodeGraphDelete(createdName);
        } catch {
          // Preserve the original template error if cleanup also fails.
        }
      }
      notify("error", String(createError));
    } finally {
      setBusy(false);
    }
  };

  const importPortableLoop = async () => {
    setBusy(true);
    try {
      const result = await importLoop();
      if (!result.ok || !result.name) {
        if (result.error !== "cancelled") {
          notify("error", result.error || "Couldn’t import the Loop.");
        }
        return;
      }
      await load(result.name);
      notify("success", `Imported “${result.name}” with all images.`);
    } catch (importError) {
      notify("error", String(importError));
    } finally {
      setBusy(false);
    }
  };

  const renameLoop = async (oldName: string, newName: string): Promise<boolean> => {
    setBusy(true);
    try {
      const result = await nodeGraphRename(oldName, newName);
      if (!result.ok || !result.name) {
        notify("error", result.error || "Couldn’t rename the Loop.");
        return false;
      }
      await load(selectedName === oldName ? result.name : selectedName ?? undefined);
      return true;
    } catch (renameError) {
      notify("error", String(renameError));
      return false;
    } finally {
      setBusy(false);
    }
  };

  const deleteLoop = async () => {
    if (!deleteTarget) return;
    setBusy(true);
    try {
      const result = await nodeGraphDelete(deleteTarget);
      if (!result.ok) {
        notify("error", result.error || "Couldn’t delete the Loop.");
        return;
      }
      localStorage.removeItem(`clawmation:node-draft:${deleteTarget}`);
      const preferredName = selectedName === deleteTarget ? undefined : selectedName ?? undefined;
      setDeleteTarget(null);
      await load(preferredName);
    } catch (deleteError) {
      notify("error", String(deleteError));
    } finally {
      setBusy(false);
    }
  };

  if (loading && loops.length === 0) {
    return (
      <div className="grid h-full place-items-center bg-background text-sm text-muted-foreground">
        <span className="flex items-center gap-2">
          <SpinnerGap className="size-4 animate-spin" />
          Loading Loops
        </span>
      </div>
    );
  }

  if (error) {
    return (
      <div className="grid h-full place-items-center bg-background px-8 text-center">
        <div>
          <p className="text-sm font-medium text-destructive">Couldn’t load Loops.</p>
          <Button className="mt-3" variant="outline" size="sm" onClick={() => void load()}>
            Retry
          </Button>
        </div>
      </div>
    );
  }

  return (
    <div className="h-full min-h-0 w-full overflow-hidden bg-background">
      {selectedName ? (
        <NodeGraphEditor
          key={selectedName}
          loopName={selectedName}
          loops={loops}
          macros={macros}
          chains={chains}
          status={status}
          active={active}
          workspaceBusy={busy}
          onSelectLoop={setSelectedName}
          onCreateLoop={createLoop}
          onImportLoop={importPortableLoop}
          onRenameLoop={renameLoop}
          onDeleteLoop={setDeleteTarget}
          onChanged={() => load(selectedName)}
        />
      ) : (
        <div
          className="node-empty-canvas relative grid h-full place-items-center overflow-hidden bg-background px-8 text-center"
          onContextMenu={(event) => {
            event.preventDefault();
            const bounds = event.currentTarget.getBoundingClientRect();
            setEmptyMenu({
              left: Math.max(8, Math.min(event.clientX - bounds.left, bounds.width - 190)),
              top: Math.max(8, Math.min(event.clientY - bounds.top, bounds.height - 62)),
            });
          }}
        >
          <div className="pointer-events-none absolute inset-0 bg-[radial-gradient(circle,hsl(var(--border))_1px,transparent_1px)] [background-size:24px_24px]" />
          <div className="relative max-w-sm">
            <LoopsIcon
              className="mx-auto size-10 text-muted-foreground/45"
              strokeWidth={VIEW_ICON_STROKE_WIDTH}
            />
            <p className="mt-3 text-sm font-semibold text-foreground">Create your first Loop</p>
            <p className="mt-1 text-xs leading-5 text-muted-foreground">
              A Loop is a complete node workflow containing any number of macros, waits, guards,
              and branches.
            </p>
            <div className="mt-4 flex items-center justify-center gap-2">
              <Button size="sm" onClick={() => void createLoop()} disabled={busy}>
                {busy ? <SpinnerGap className="size-4 animate-spin" /> : <Plus className="size-4" />}
                New Loop
              </Button>
              <Button
                variant="outline"
                size="sm"
                onClick={() => void createLoop("learn-loops")}
                disabled={busy}
              >
                <BookOpenText className="size-4" weight="duotone" />
                Learn Loops
              </Button>
              <Button
                variant="outline"
                size="sm"
                onClick={() => void importPortableLoop()}
                disabled={busy}
              >
                <DownloadSimple className="size-4" />
                Import
              </Button>
            </div>
            <p className="mt-3 text-[10px] text-muted-foreground">
              You can also right-click anywhere on the canvas.
            </p>
          </div>

          {emptyMenu && (
            <div
              role="menu"
              aria-label="Loop menu"
              className="absolute z-20 w-44 rounded-md border border-border bg-popover p-1.5 shadow-xl"
              style={{ left: emptyMenu.left, top: emptyMenu.top }}
              onPointerDown={(event) => event.stopPropagation()}
            >
              <button
                type="button"
                role="menuitem"
                className="flex w-full items-center gap-2 rounded-sm px-2 py-2 text-left text-xs font-medium text-popover-foreground hover:bg-accent"
                onClick={() => void createLoop()}
              >
                <Plus className="size-4 text-primary" weight="bold" />
                New Loop
              </button>
              <button
                type="button"
                role="menuitem"
                className="flex w-full items-center gap-2 rounded-sm px-2 py-2 text-left text-xs font-medium text-popover-foreground hover:bg-accent"
                onClick={() => void createLoop("learn-loops")}
              >
                <BookOpenText className="size-4 text-primary" weight="duotone" />
                Learn Loops
              </button>
              <button
                type="button"
                role="menuitem"
                className="flex w-full items-center gap-2 rounded-sm px-2 py-2 text-left text-xs font-medium text-popover-foreground hover:bg-accent"
                onClick={() => void importPortableLoop()}
              >
                <DownloadSimple className="size-4 text-primary" />
                Import Loop
              </button>
            </div>
          )}
        </div>
      )}

      <AlertDialog
        open={deleteTarget !== null}
        onOpenChange={(open) => {
          if (!open) setDeleteTarget(null);
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete “{deleteTarget}”?</AlertDialogTitle>
            <AlertDialogDescription>
              This removes only the Loop workflow. Recorded macros inside it are not deleted.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              onClick={() => void deleteLoop()}
              disabled={busy}
            >
              Delete Loop
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}
