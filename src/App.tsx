import { useEffect, useState } from "react";

import { TopBar } from "./components/TopBar";
import { Sidebar } from "./components/Sidebar";
import { ToastProvider } from "./components/Toasts";
import { NAV_ITEMS, type ViewId } from "./nav";
import { useStatus } from "./useStatus";
import { View } from "./views";
import "./theme.css";

function App() {
  const status = useStatus();
  const [view, setView] = useState<ViewId>("dashboard");

  // Alt+1..7 jump straight to a view (index into the sidebar order), unless the
  // user is typing in a field. Mirrors the Python UI's keyboard nav.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!e.altKey || e.ctrlKey || e.metaKey) return;
      const target = e.target as HTMLElement | null;
      const tag = target?.tagName;
      if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT" || target?.isContentEditable) {
        return;
      }
      const index = Number(e.key) - 1;
      if (index >= 0 && index < NAV_ITEMS.length) {
        e.preventDefault();
        setView(NAV_ITEMS[index].id);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  return (
    <ToastProvider>
      <TopBar status={status} />
      <div
        style={{
          flex: 1,
          display: "flex",
          minHeight: 0,
          position: "relative",
          zIndex: 1,
        }}
      >
        <Sidebar view={view} onSelect={setView} macroCount={status?.macro_count ?? 0} />
        <main
          style={{
            flex: 1,
            overflowY: "auto",
            minWidth: 0,
            padding: "34px 40px 64px",
          }}
        >
          <div className="ds-page">
            <View view={view} status={status} onNavigate={setView} />
          </div>
        </main>
      </div>
    </ToastProvider>
  );
}

export default App;
