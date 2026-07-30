import {
  Suspense,
  memo,
  startTransition,
  useCallback,
  useEffect,
  useState,
} from "react";

import { onAssociatedImport, onUpdateAvailable } from "@/api";
import { notifyAction } from "@/lib/toast";
import { useStatus } from "@/useStatus";
import { NAV, type ViewId } from "@/nav";
import { CommandBar } from "@/components/CommandBar";
import { Toaster } from "@/components/ui/sonner";
import { TooltipProvider } from "@/components/ui/tooltip";
import { Home } from "@/views/Home";
import { Macros } from "@/views/Macros";
import { Nodes } from "@/views/Nodes";
import { Watch } from "@/views/Watch";
import { Settings } from "@/views/Settings";
import type { ViewProps } from "@/views/types";

const VIEW_CACHE_LIMIT = 3;

export function updateViewCache(current: ViewId[], next: ViewId): ViewId[] {
  return [...current.filter((view) => view !== next), next].slice(
    -VIEW_CACHE_LIMIT,
  );
}

function renderView(view: ViewId, props: ViewProps) {
  switch (view) {
    case "dashboard":
      return <Home {...props} />;
    case "macros":
      return <Macros {...props} />;
    case "nodes":
      return <Nodes {...props} />;
    case "vision":
      return <Watch {...props} />;
    case "settings":
      return <Settings {...props} />;
  }
}

const ViewSurface = memo(function ViewSurface({
  view,
  active,
  status,
  navigate,
}: {
  view: ViewId;
  active: boolean;
  status: ViewProps["status"];
  navigate: ViewProps["navigate"];
}) {
  const content = renderView(view, { status, navigate, active });
  const workspace = view === "nodes" || view === "macros";
  const contentClass =
    view === "dashboard" || view === "macros" || view === "vision"
      ? view === "macros"
        ? "mx-auto h-full w-full max-w-[2200px] px-5 py-5 md:px-7"
        : "mx-auto w-full max-w-[1480px] px-5 py-5 md:px-7"
      : "mx-auto w-full max-w-[1320px] px-5 py-6 md:px-7 md:py-8";

  return (
    <section
      data-view-surface={view}
      data-active={active ? "true" : "false"}
      aria-hidden={!active}
      inert={!active}
      hidden={!active}
      className={
        workspace
          ? "h-full min-h-0 overflow-hidden"
          : "h-full min-h-0 overflow-y-auto"
      }
    >
      {view === "nodes" ? content : <div className={contentClass}>{content}</div>}
    </section>
  );
}, (previous, next) =>
  previous.view === next.view &&
  previous.active === next.active &&
  previous.navigate === next.navigate &&
  (!next.active || previous.status === next.status),
);

export default function App() {
  const status = useStatus();
  const [view, setView] = useState<ViewId>("dashboard");
  const [contentView, setContentView] = useState<ViewId>("dashboard");
  const [cachedViews, setCachedViews] = useState<ViewId[]>(["dashboard"]);

  const navigate = useCallback((next: ViewId) => {
    if (document.activeElement instanceof HTMLElement) {
      document.activeElement.blur();
    }
    document.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
    );
    // The command bar updates urgently. Mounting or revealing a heavy workspace
    // is concurrent, so it cannot hold the active-tab feedback behind its work.
    setView(next);
    startTransition(() => {
      setCachedViews((current) => updateViewCache(current, next));
      setContentView(next);
    });
  }, []);

  // Alt+1..6 jump straight to a view (index into NAV order), unless typing in a
  // field.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!e.altKey || e.ctrlKey || e.metaKey) return;
      const t = e.target as HTMLElement | null;
      const tag = t?.tagName;
      if (
        tag === "INPUT" ||
        tag === "TEXTAREA" ||
        tag === "SELECT" ||
        t?.isContentEditable
      ) {
        return;
      }
      const i = Number(e.key) - 1;
      if (i >= 0 && i < NAV.length && !NAV[i].disabled) {
        e.preventDefault();
        navigate(NAV[i].id);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [navigate]);

  useEffect(() => {
    let alive = true;
    let unlisten: (() => void) | undefined;
    void onAssociatedImport((imported) => {
      if (imported.kind !== "loop") return;
      localStorage.setItem("clawmation:pending-loop-selection", imported.name);
      navigate("nodes");
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
  }, [navigate]);

  // The backend checks for a release once at launch and announces the result
  // here. Settings › About owns the install itself, so the toast just points at
  // it; an update never interrupts a run.
  useEffect(() => {
    let alive = true;
    let unlisten: (() => void) | undefined;
    void onUpdateAvailable((info) => {
      notifyAction(
        `Clawmation ${info.latest} is out; you’re on ${info.current}.`,
        "Show me",
        () => navigate("settings"),
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
  }, [navigate]);

  return (
    <TooltipProvider delayDuration={300}>
      <div className="flex h-[100dvh] min-h-0 flex-col overflow-hidden">
        <CommandBar status={status} view={view} navigate={navigate} />
        <main className="relative min-h-0 flex-1 overflow-hidden">
          <Suspense fallback={null}>
            {cachedViews.map((cachedView) => (
              <ViewSurface
                key={cachedView}
                view={cachedView}
                active={cachedView === contentView}
                status={status}
                navigate={navigate}
              />
            ))}
          </Suspense>
        </main>
      </div>
      <Toaster
        position="top-right"
        offset={{ top: 76, right: 20 }}
        mobileOffset={{ top: 72, right: 12, left: 12 }}
      />
    </TooltipProvider>
  );
}
