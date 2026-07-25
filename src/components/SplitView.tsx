import type { ReactNode } from "react";
import type { LucideIcon } from "lucide-react";

import { cn } from "@/lib/utils";

export interface RailItem<T extends string> {
  id: T;
  label: string;
  Icon: LucideIcon;
}

/**
 * The two-pane shape Watch and Autopilot use instead of Macros' single page:
 * a short left rail that switches between the halves a view is made of, and
 * the chosen pane filling the rest of the width. Views render every pane and
 * hide the far ones themselves, so a switch drops nothing that was open.
 */
export function SplitView<T extends string>({
  items,
  active,
  onSelect,
  label,
  children,
}: {
  items: RailItem<T>[];
  active: T;
  onSelect: (id: T) => void;
  label: string;
  children: ReactNode;
}) {
  return (
    <div className="flex items-start gap-6">
      <nav aria-label={label} className="flex w-40 shrink-0 flex-col gap-1">
        {items.map((it) => {
          const current = it.id === active;
          return (
            <button
              key={it.id}
              type="button"
              onClick={() => onSelect(it.id)}
              aria-current={current ? "page" : undefined}
              className={cn(
                "flex items-center gap-2.5 rounded-lg px-3 py-2 text-sm font-medium outline-none transition-colors focus-visible:ring-[3px] focus-visible:ring-ring/50",
                current
                  ? "bg-primary/10 text-foreground"
                  : "text-muted-foreground hover:bg-muted/60 hover:text-foreground",
              )}
            >
              <it.Icon className={cn("size-4 shrink-0", current && "text-primary")} />
              {it.label}
            </button>
          );
        })}
      </nav>
      <div className="min-w-0 flex-1">{children}</div>
    </div>
  );
}
