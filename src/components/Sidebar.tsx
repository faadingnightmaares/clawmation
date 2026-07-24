import type { CSSProperties } from "react";

import { NAV_ITEMS, type ViewId } from "../nav";

interface SidebarProps {
  view: ViewId;
  onSelect: (view: ViewId) => void;
  /** Live macro count for the Macros badge (from `get_status`). */
  macroCount: number;
}

const asideStyle: CSSProperties = {
  width: 206,
  flexShrink: 0,
  borderRight: "1px solid var(--border-faint)",
  background: "rgba(252,250,247,0.55)",
  display: "flex",
  flexDirection: "column",
  padding: "14px 10px",
};

const badgeStyle: CSSProperties = {
  display: "inline-flex",
  alignItems: "center",
  justifyContent: "center",
  minWidth: 18,
  height: 18,
  padding: "0 5px",
  borderRadius: 9,
  background: "rgba(194,163,112,0.15)",
  border: "1px solid var(--gold-border)",
  fontFamily: "var(--font-mono)",
  fontSize: 10,
  fontWeight: 600,
  color: "var(--color-gold)",
};

function navButtonStyle(active: boolean): CSSProperties {
  return {
    display: "flex",
    alignItems: "center",
    gap: 10,
    padding: "9px 12px",
    borderRadius: "var(--radius-sm)",
    border: "none",
    cursor: "pointer",
    textAlign: "left",
    fontFamily: "var(--font-sans)",
    fontSize: 12.5,
    fontWeight: 500,
    whiteSpace: "nowrap",
    background: active ? "var(--color-ink)" : "transparent",
    color: active ? "var(--color-parchment)" : "var(--text-78)",
  };
}

export function Sidebar({ view, onSelect, macroCount }: SidebarProps) {
  return (
    <aside style={asideStyle}>
      <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
        {NAV_ITEMS.map(({ id, label, Icon }) => {
          const active = view === id;
          // Only the Macros badge has a live count in this build; other view
          // counts arrive with their own slices.
          const count = id === "macros" ? macroCount : undefined;
          return (
            <button
              key={id}
              onClick={() => onSelect(id)}
              data-active={active}
              style={navButtonStyle(active)}
            >
              <Icon size={16} />
              <span style={{ flex: 1 }}>{label}</span>
              {count !== undefined && count > 0 && (
                <span style={badgeStyle}>{count}</span>
              )}
            </button>
          );
        })}
      </div>
      <div style={{ flex: 1 }} />
    </aside>
  );
}
