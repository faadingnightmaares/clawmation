import { useEffect, useState } from "react";
import { useTheme } from "next-themes";
import { Moon, Sun } from "@phosphor-icons/react";

import type { Status } from "@/api";
import {
  VIEW_ICONS,
  VIEW_ICON_STROKE_WIDTH,
  type ViewId,
} from "@/nav";
import { reducedMotion } from "@/lib/anime";
import { cn } from "@/lib/utils";
import { Logo } from "@/components/Logo";
import { ViewSwitch } from "@/components/ViewSwitch";
import { WindowControls } from "@/components/WindowControls";
import { Button } from "@/components/ui/button";

interface CommandBarProps {
  status: Status | null;
  view: ViewId;
  navigate: (v: ViewId) => void;
}

const SettingsIcon = VIEW_ICONS.settings;

export function CommandBar({ view, navigate }: CommandBarProps) {
  const { setTheme, resolvedTheme } = useTheme();
  const [mounted, setMounted] = useState(false);
  useEffect(() => setMounted(true), []);
  const isDark = (mounted ? resolvedTheme : "dark") === "dark";

  const changeTheme = () => {
    const nextTheme = isDark ? "light" : "dark";
    if (!document.startViewTransition || reducedMotion()) {
      setTheme(nextTheme);
      return;
    }

    document.documentElement.dataset.themeTransition = "";
    const transition = document.startViewTransition(() => {
      setTheme(nextTheme);
    });
    void transition.finished.finally(() => {
      delete document.documentElement.dataset.themeTransition;
    });
  };

  return (
    <header
      data-tauri-drag-region
      className="sticky top-0 z-40 grid h-[72px] shrink-0 grid-cols-[1fr_auto_1fr] items-center gap-3 border-b border-border/75 bg-background/92 px-5 backdrop-blur-md select-none"
    >
      <button
        type="button"
        onClick={() => navigate("dashboard")}
        className="w-fit rounded-lg px-1.5 py-1 outline-none focus-visible:ring-[3px] focus-visible:ring-ring/35"
        title="Home"
      >
        <Logo />
      </button>

      <ViewSwitch view={view} navigate={navigate} />

      <div
        data-tauri-drag-region
        className="flex min-w-0 items-center justify-end gap-1.5"
      >
        <Button
          variant="ghost"
          size="icon-sm"
          onClick={() => navigate("settings")}
          aria-label="Settings"
          aria-current={view === "settings" ? "page" : undefined}
          title="Settings · Alt+6"
          className={cn(
            view === "settings"
              ? "bg-secondary text-foreground hover:bg-secondary"
              : "text-muted-foreground",
          )}
        >
          <SettingsIcon
            className="size-[17px]"
            strokeWidth={VIEW_ICON_STROKE_WIDTH}
          />
        </Button>

        <Button
          variant="ghost"
          size="icon-sm"
          onClick={changeTheme}
          aria-label={isDark ? "Switch to light" : "Switch to dark"}
          title={isDark ? "Switch to light" : "Switch to dark"}
          className="text-muted-foreground"
        >
          {isDark ? (
            <Sun className="size-[17px]" />
          ) : (
            <Moon className="size-[17px]" />
          )}
        </Button>

        <div className="mx-1 h-6 w-px bg-border" />
        <WindowControls />
      </div>
    </header>
  );
}
