import { FlaskConical, ImageIcon, Loader2, Trash2, Type, Zap } from "lucide-react";

import { type Guard } from "@/api";
import { hsvToCss } from "@/format";
import { cn } from "@/lib/utils";
import { describeTrigger, draftFromGuard, lookOf } from "@/lib/triggers";
import { Badge } from "@/components/ui/badge";
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

/** One hairline row in a `divide-y` list: both the Watch list and the per-macro
 *  guards sheet. The row itself is the edit target; the pencil it replaces was a
 *  small thing to aim at for the reason people open this list at all. */
export function TriggerRow({ guard, testing, onEdit, onTest, onToggle, onDelete }: TriggerRowProps) {
  const d = draftFromGuard(guard);
  const look = lookOf(d.method);
  const enabled = d.enabled;
  const name = d.name || "Untitled trigger";

  return (
    <div className={cn("group relative transition-opacity", !enabled && "opacity-55")}>
      <button
        type="button"
        onClick={onEdit}
        className="absolute inset-0 z-0 outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
      >
        <span className="sr-only">Edit {name}</span>
      </button>

      <div className="pointer-events-none relative z-10 flex items-center gap-3 px-4 py-3">
        {/* Look badge */}
        <div className="flex size-9 shrink-0 items-center justify-center rounded-lg border border-border bg-secondary/50">
          {look === "color" ? (
            <span
              className="size-4 rounded-full border border-border"
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
          <div className="flex items-center gap-2">
            <span className="truncate text-sm font-medium text-foreground">{name}</span>
            {d.cooldown <= 0 && (
              <Badge
                variant="outline"
                className="shrink-0 gap-1 font-normal text-muted-foreground"
                title="No wait between repeats. It acts every time it sees this"
              >
                <Zap className="size-3" />
                No wait
              </Badge>
            )}
          </div>
          <p className="truncate text-xs text-muted-foreground">{describeTrigger(d)}</p>
        </div>

        {/* The switch always shows: it is the row's state, not an action. The
            other two surface when you reach for the row. */}
        <div className="pointer-events-auto flex items-center gap-1">
          <Button
            variant="ghost"
            size="sm"
            onClick={onTest}
            disabled={testing}
            title="See if it can spot it now"
            className={cn(
              "opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100",
              testing && "opacity-100",
            )}
          >
            {testing ? <Loader2 className="size-4 animate-spin" /> : <FlaskConical className="size-4" />}
          </Button>
          <Button
            variant="ghost"
            size="sm"
            onClick={onDelete}
            title="Delete"
            className="text-muted-foreground opacity-0 transition-opacity hover:text-destructive group-hover:opacity-100 group-focus-within:opacity-100"
          >
            <Trash2 className="size-4" />
          </Button>
          <Switch
            checked={enabled}
            onCheckedChange={onToggle}
            aria-label={`${name}, ${enabled ? "on" : "off"}`}
          />
        </div>
      </div>
    </div>
  );
}
