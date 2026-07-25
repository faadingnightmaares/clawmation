import { useEffect, useState, type ReactNode } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Minus, Square, Copy, X } from "lucide-react";

import { cn } from "@/lib/utils";

/** The window handle, or null when there isn't one (tests, a plain browser). */
function appWindow() {
  try {
    return getCurrentWindow();
  } catch {
    return null;
  }
}

/**
 * Minimize / maximize / close for the frameless window. The native title bar is
 * off (`"decorations": false`), so these are the whole title bar. Every one of
 * them swallows its own failure rather than throwing into the render tree and
 * taking the header down with it.
 *
 * Close doesn't quit: the backend answers `CloseRequested` by hiding the window
 * to the tray, so a run in progress survives it. Quitting is the tray menu.
 */
export function WindowControls() {
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    const win = appWindow();
    if (!win) return;
    let alive = true;
    let unlisten: (() => void) | undefined;

    const sync = () => void win.isMaximized().then((m) => alive && setMaximized(m)).catch(() => {});
    sync();
    // Snapping and double-click-to-maximize change the state without a click on
    // our button, so the icon follows the window rather than our own actions.
    void win
      .onResized(sync)
      .then((off) => {
        if (alive) unlisten = off;
        else off();
      })
      .catch(() => {});

    return () => {
      alive = false;
      unlisten?.();
    };
  }, []);

  const run = (fn: (w: ReturnType<typeof getCurrentWindow>) => Promise<unknown>) => () => {
    const win = appWindow();
    if (win) void fn(win).catch(() => {});
  };

  return (
    <div className="flex items-center gap-0.5">
      <ControlButton label="Minimize" onClick={run((w) => w.minimize())}>
        <Minus className="size-4" />
      </ControlButton>
      <ControlButton
        label={maximized ? "Restore" : "Maximize"}
        onClick={run((w) => w.toggleMaximize())}
      >
        {maximized ? <Copy className="size-3.5 -scale-x-100" /> : <Square className="size-3.5" />}
      </ControlButton>
      <ControlButton
        label="Close to tray"
        onClick={run((w) => w.close())}
        className="hover:bg-destructive hover:text-destructive-foreground"
      >
        <X className="size-4" />
      </ControlButton>
    </div>
  );
}

function ControlButton({
  label,
  onClick,
  className,
  children,
}: {
  label: string;
  onClick: () => void;
  className?: string;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      onClick={onClick}
      className={cn(
        "flex size-8 items-center justify-center rounded-md text-muted-foreground transition-colors outline-none hover:bg-secondary hover:text-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50",
        className,
      )}
    >
      {children}
    </button>
  );
}
