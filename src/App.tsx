import { useEffect, useState } from "react";

import { onUpdateAvailable } from "@/api";
import { notifyAction } from "@/lib/toast";
import { useStatus } from "@/useStatus";
import { NAV, type ViewId } from "@/nav";
import { CommandBar } from "@/components/CommandBar";
import { Toaster } from "@/components/ui/sonner";
import { TooltipProvider } from "@/components/ui/tooltip";
import { Home } from "@/views/Home";
import { Macros } from "@/views/Macros";
import { Watch } from "@/views/Watch";
import { Guards } from "@/views/Guards";
import { Chains } from "@/views/Chains";
import { Guide } from "@/views/Guide";
import { Settings } from "@/views/Settings";
import type { ViewProps } from "@/views/types";

function renderView(view: ViewId, props: ViewProps) {
  switch (view) {
    case "dashboard":
      return <Home {...props} />;
    case "macros":
      return <Macros {...props} />;
    case "ai":
      return <Guards {...props} />;
    case "vision":
      return <Watch {...props} />;
    case "chains":
      return <Chains {...props} />;
    case "guide":
      return <Guide {...props} />;
    case "settings":
      return <Settings {...props} />;
  }
}

export default function App() {
  const status = useStatus();
  const [view, setView] = useState<ViewId>("macros");

  // Alt+1..7 jump straight to a view (index into NAV order), unless typing in a
  // field. Preserves the original shortcut mapping.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!e.altKey || e.ctrlKey || e.metaKey) return;
      const t = e.target as HTMLElement | null;
      const tag = t?.tagName;
      if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT" || t?.isContentEditable) {
        return;
      }
      const i = Number(e.key) - 1;
      if (i >= 0 && i < NAV.length) {
        e.preventDefault();
        setView(NAV[i].id);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // The backend checks for a release once at launch and announces the result
  // here. Settings › About owns the install itself, so the toast just points at
  // it — an update never interrupts a run.
  useEffect(() => {
    let alive = true;
    let unlisten: (() => void) | undefined;
    void onUpdateAvailable((info) => {
      notifyAction(`Clawmation ${info.latest} is out — you’re on ${info.current}.`, "Show me", () =>
        setView("settings"),
      );
    })
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

  return (
    <TooltipProvider delayDuration={300}>
      <div className="flex h-full flex-col">
        <CommandBar status={status} view={view} navigate={setView} />
        <main className="flex-1 overflow-y-auto">
          <div className="mx-auto w-full max-w-5xl px-6 py-8 md:px-8">
            {renderView(view, { status, navigate: setView })}
          </div>
        </main>
      </div>
      <Toaster position="bottom-right" />
    </TooltipProvider>
  );
}
