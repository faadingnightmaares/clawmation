import { FlaskConical, ImageIcon, Loader2, Pencil, Trash2, Type } from "lucide-react";

import { type Guard } from "@/api";
import { hsvToCss } from "@/format";
import { cn } from "@/lib/utils";
import { describeTrigger, draftFromGuard, lookOf } from "@/lib/triggers";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";

export interface TriggerRowProps {
  guard: Guard;
  testing?: boolean;
  onEdit: () => void;
  onTest: () => void;
  onToggle: (enabled: boolean) => void;
  onDelete: () => void;
}

export function TriggerRow({ guard, testing, onEdit, onTest, onToggle, onDelete }: TriggerRowProps) {
  const d = draftFromGuard(guard);
  const look = lookOf(d.method);
  const enabled = d.enabled;

  return (
    <div
      className={cn(
        "flex items-center gap-3 rounded-lg border border-border bg-card p-3 transition-opacity",
        !enabled && "opacity-55",
      )}
    >
      {/* Look badge */}
      <div className="flex size-9 shrink-0 items-center justify-center rounded-md border border-border bg-secondary/50">
        {look === "color" ? (
          <span
            className="size-5 rounded-full border border-border"
            style={{ background: hsvToCss(d.hsv_low, d.hsv_high) }}
          />
        ) : look === "image" ? (
          <ImageIcon className="size-4 text-muted-foreground" />
        ) : (
          <Type className="size-4 text-muted-foreground" />
        )}
      </div>

      {/* Name + plain description */}
      <div className="min-w-0 flex-1">
        <p className="truncate text-sm font-medium text-foreground">{d.name || "Untitled trigger"}</p>
        <p className="truncate text-xs text-muted-foreground">{describeTrigger(d)}</p>
      </div>

      {/* Actions */}
      <Button variant="ghost" size="sm" onClick={onTest} disabled={testing} title="See if it can spot it now">
        {testing ? <Loader2 className="size-4 animate-spin" /> : <FlaskConical className="size-4" />}
      </Button>
      <Button variant="ghost" size="sm" onClick={onEdit} title="Edit">
        <Pencil className="size-4" />
      </Button>
      <Switch checked={enabled} onCheckedChange={onToggle} aria-label={enabled ? "On" : "Off"} />
      <Button
        variant="ghost"
        size="sm"
        onClick={onDelete}
        title="Delete"
        className="text-muted-foreground hover:text-destructive"
      >
        <Trash2 className="size-4" />
      </Button>
    </div>
  );
}
