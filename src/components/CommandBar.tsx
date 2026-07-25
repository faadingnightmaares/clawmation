import { useEffect, useState } from "react";
import { useTheme } from "next-themes";
import {
  Square,
  ChevronDown,
  Minimize2,
  Sun,
  Moon,
  Repeat,
} from "lucide-react";

import { emergencyStop, stopRecord, windowMinimizeToTray } from "@/api";
import type { Status } from "@/api";
import { MORE_VIEWS, type ViewId } from "@/nav";
import { notify } from "@/lib/toast";
import { cn } from "@/lib/utils";
import { Logo } from "@/components/Logo";
import { ViewSwitch } from "@/components/ViewSwitch";
import { WindowControls } from "@/components/WindowControls";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";

interface CommandBarProps {
  status: Status | null;
  view: ViewId;
  navigate: (v: ViewId) => void;
}

interface ModeLook {
  label: string;
  dot: string; // tailwind bg-* for the live dot
  text: string; // tailwind text-* for the label
}

function modeLook(mode: string): ModeLook | null {
  switch (mode) {
    case "recording":
      return { label: "Recording", dot: "bg-destructive", text: "text-destructive" };
    case "playing":
      return { label: "Playing", dot: "bg-success", text: "text-success" };
    case "paused":
      return { label: "Paused", dot: "bg-primary", text: "text-primary" };
    default:
      return null; // idle says nothing — an empty bar is the resting state
  }
}

/** The live status pill — a pulsing dot the user can read from across the room,
 *  plus a rep counter while a macro loops. Absent while idle. */
function RunState({ status }: { status: Status | null }) {
  const look = modeLook(status?.mode ?? "idle");
  if (!look) return null;
  const showReps = status?.mode === "playing" && (status?.play_iteration ?? 0) > 0;

  return (
    <div className="flex items-center gap-3">
      <div className="flex items-center gap-2 rounded-full bg-secondary/60 px-3 py-1.5 text-xs font-medium">
        <span className="relative flex size-2">
          <span
            className={cn(
              "absolute inline-flex size-full animate-ping rounded-full opacity-60",
              look.dot,
            )}
          />
          <span className={cn("relative inline-flex size-2 rounded-full", look.dot)} />
        </span>
        <span className={look.text}>{look.label}</span>
      </div>

      {showReps && status && (
        <span className="flex items-center gap-1.5 text-xs font-medium text-primary">
          <Repeat className="size-3.5" />
          {status.play_total_reps > 0
            ? `${status.play_iteration} / ${status.play_total_reps}`
            : `${status.play_iteration} / ∞`}
        </span>
      )}
    </div>
  );
}

export function CommandBar({ status, view, navigate }: CommandBarProps) {
  const mode = status?.mode ?? "idle";
  const idle = mode === "idle";
  const { setTheme, resolvedTheme } = useTheme();
  const [mounted, setMounted] = useState(false);
  useEffect(() => setMounted(true), []);
  const isDark = (mounted ? resolvedTheme : "dark") === "dark";

  // Stop, ported 1:1: while recording it routes through stop_record so the macro
  // is still saved and toasted; otherwise it's the global emergency stop.
  const onStop = async () => {
    try {
      if (mode === "recording") {
        const res = await stopRecord();
        if (res?.ok) notify("success", `Saved ${res.name} — ${res.events} events`);
        else notify("error", res?.error || "Stop failed");
        return;
      }
      await emergencyStop();
      notify("info", "Stopped.");
    } catch {
      notify("error", "Stop failed");
    }
  };

  const onMinimize = async () => {
    try {
      await windowMinimizeToTray();
      notify("info", "Minimized to tray — click the tray icon to restore.");
    } catch {
      notify("error", "Minimize failed");
    }
  };

  // This header *is* the title bar — the window is undecorated, so the bare
  // stretches of it carry `data-tauri-drag-region` and move the window. Only
  // elements without the attribute are clickable, so controls work as usual.
  return (
    <header
      data-tauri-drag-region
      className="sticky top-0 z-40 flex h-14 shrink-0 items-center gap-3 border-b border-border bg-background/80 px-3 backdrop-blur-md select-none"
    >
      <button
        type="button"
        onClick={() => navigate("dashboard")}
        className="rounded-md px-1 outline-none focus-visible:ring-2 focus-visible:ring-ring"
        title="Home"
      >
        <Logo />
      </button>

      <ViewSwitch view={view} navigate={navigate} />

      <div data-tauri-drag-region className="h-full flex-1" />

      <RunState status={status} />

      {!idle && (
        <Button
          variant="ghost"
          size="sm"
          onClick={onStop}
          className="gap-1.5 text-destructive hover:bg-destructive/10 hover:text-destructive"
        >
          <Square className="size-3.5 fill-current" />
          Stop
        </Button>
      )}

      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button variant="ghost" size="sm" className="gap-1 text-muted-foreground">
            More
            <ChevronDown className="size-4" />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="w-48">
          {MORE_VIEWS.map((v) => (
            <DropdownMenuItem
              key={v.id}
              onSelect={() => navigate(v.id)}
              className={cn(view === v.id && "text-primary")}
            >
              <v.Icon className="size-4" />
              {v.label}
            </DropdownMenuItem>
          ))}
          <DropdownMenuSeparator />
          <DropdownMenuItem onSelect={onMinimize}>
            <Minimize2 className="size-4" />
            Minimize to tray
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>

      <Button
        variant="ghost"
        size="icon-sm"
        onClick={() => setTheme(isDark ? "light" : "dark")}
        title={isDark ? "Switch to light" : "Switch to dark"}
        className="text-muted-foreground"
      >
        {isDark ? <Sun className="size-4" /> : <Moon className="size-4" />}
      </Button>

      <div className="mx-0.5 h-6 w-px bg-border" />

      <WindowControls />
    </header>
  );
}
