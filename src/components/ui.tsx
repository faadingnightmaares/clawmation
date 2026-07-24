import type { ButtonHTMLAttributes, CSSProperties, ReactNode } from "react";

// ════════════════════════════════════════════════════════════════════
// Shared design primitives — "Trace by Bindery Citadel".
//
// Every view composes these instead of hand-rolling cards and headers, so
// the whole app's density and weight live in a handful of knobs here (card
// surface, corner, shadow; page rhythm; empty-state and button shape). The
// palette itself stays in theme.css — this file only decides proportion.
// ════════════════════════════════════════════════════════════════════

/** The one card recipe: solid enough to read as a surface on parchment,
 *  soft enough to belong to it. Spread it, then add padding per use. */
export const CARD: CSSProperties = {
  background: "var(--surface-card)",
  border: "1px solid var(--border-default)",
  borderRadius: "var(--radius-xl)",
  boxShadow: "var(--shadow-2)",
};

/** The mono ALLCAPS overline used to title a card or a group. */
export const LABEL: CSSProperties = {
  fontFamily: "var(--font-mono)",
  fontSize: 10,
  letterSpacing: "0.08em",
  textTransform: "uppercase",
  color: "var(--text-50)",
};

// ─── Page: the standard header (gold rule + serif title + optional lead
//     and a right-aligned action) followed by the view's body. ───
export function Page({
  label,
  title,
  subtitle,
  action,
  children,
}: {
  label?: string;
  title: string;
  subtitle?: ReactNode;
  action?: ReactNode;
  children: ReactNode;
}) {
  return (
    <div data-screen-label={title}>
      {/* A view's title block — not a landmark. The lone <header role="banner">
          is the app's TopBar; keeping this a plain div avoids a second banner. */}
      <div
        style={{
          display: "flex",
          alignItems: "flex-end",
          justifyContent: "space-between",
          gap: 24,
          marginBottom: 26,
        }}
      >
        <div style={{ minWidth: 0 }}>
          <div className="ds-rule" />
          {label && (
            <div className="t-label-gold" style={{ marginBottom: 9 }}>
              {label}
            </div>
          )}
          <h1 className="t-h1">{title}</h1>
          {subtitle && (
            <p
              className="t-body-sm"
              style={{ marginTop: 9, maxWidth: "64ch", color: "var(--text-50)" }}
            >
              {subtitle}
            </p>
          )}
        </div>
        {action && (
          <div style={{ flexShrink: 0, display: "flex", gap: 8, alignItems: "center" }}>
            {action}
          </div>
        )}
      </div>
      {children}
    </div>
  );
}

// ─── Card: the CARD recipe as an element, with a default 20px padding. ───
export function Card({
  children,
  style,
  pad = 20,
}: {
  children: ReactNode;
  style?: CSSProperties;
  pad?: number | string;
}) {
  return <div style={{ ...CARD, padding: pad, ...style }}>{children}</div>;
}

// ─── CardHead: a card's overline + optional serif title, description, and
//     a right-aligned action (an Add button, a toggle, a menu). ───
export function CardHead({
  label,
  title,
  desc,
  action,
  style,
}: {
  label?: string;
  title?: ReactNode;
  desc?: ReactNode;
  action?: ReactNode;
  style?: CSSProperties;
}) {
  return (
    <div
      style={{
        display: "flex",
        alignItems: "flex-start",
        justifyContent: "space-between",
        gap: 12,
        marginBottom: 14,
        ...style,
      }}
    >
      <div style={{ minWidth: 0 }}>
        {label && <div style={LABEL}>{label}</div>}
        {title && (
          <div
            style={{
              fontFamily: "var(--font-serif)",
              fontSize: 18,
              fontWeight: 400,
              letterSpacing: "-0.01em",
              color: "var(--color-ink)",
              marginTop: label ? 5 : 0,
            }}
          >
            {title}
          </div>
        )}
        {desc && (
          <div
            style={{
              fontFamily: "var(--font-sans)",
              fontSize: 11.5,
              color: "var(--text-40)",
              marginTop: 4,
              lineHeight: 1.5,
            }}
          >
            {desc}
          </div>
        )}
      </div>
      {action && <div style={{ flexShrink: 0 }}>{action}</div>}
    </div>
  );
}

// ─── EmptyState: a generous dashed panel with an icon, a headline, and a
//     hint — so "nothing here yet" reads as designed, not forgotten. ───
export function EmptyState({
  icon,
  title,
  hint,
  style,
}: {
  icon?: ReactNode;
  title: string;
  hint?: ReactNode;
  style?: CSSProperties;
}) {
  return (
    <div
      style={{
        border: "1px dashed var(--border-default)",
        borderRadius: "var(--radius-lg)",
        background: "var(--surface-inset)",
        padding: "34px 24px",
        textAlign: "center",
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        gap: 9,
        ...style,
      }}
    >
      {icon && (
        <div style={{ color: "var(--text-25)", display: "flex", marginBottom: 2 }}>{icon}</div>
      )}
      <div style={{ fontFamily: "var(--font-sans)", fontSize: 13, fontWeight: 600, color: "var(--text-68)" }}>
        {title}
      </div>
      {hint && (
        <div
          style={{
            fontFamily: "var(--font-sans)",
            fontSize: 12,
            color: "var(--text-40)",
            maxWidth: "44ch",
            lineHeight: 1.55,
          }}
        >
          {hint}
        </div>
      )}
    </div>
  );
}

// ─── Stat: a labelled figure in an inset well (counts, backends, FPS). ───
export function Stat({
  label,
  value,
  accent,
}: {
  label: string;
  value: ReactNode;
  accent?: boolean;
}) {
  return (
    <div className="ds-inset" style={{ padding: "12px 14px" }}>
      <div
        style={{
          fontFamily: "var(--font-mono)",
          fontSize: 9,
          letterSpacing: "0.08em",
          textTransform: "uppercase",
          color: "var(--text-40)",
        }}
      >
        {label}
      </div>
      <div
        style={{
          fontFamily: "var(--font-mono)",
          fontSize: 20,
          fontWeight: 700,
          color: accent ? "var(--color-gold)" : "var(--color-ink)",
          marginTop: 4,
          letterSpacing: "-0.01em",
        }}
      >
        {value}
      </div>
    </div>
  );
}

// ─── Buttons ───
// The primary action: dark ink field, gold label — the app's one loud move.
type BtnProps = ButtonHTMLAttributes<HTMLButtonElement> & {
  full?: boolean;
  icon?: ReactNode;
};

const btnBase: CSSProperties = {
  display: "inline-flex",
  alignItems: "center",
  justifyContent: "center",
  gap: 8,
  height: 38,
  padding: "0 18px",
  borderRadius: "var(--radius-md)",
  fontFamily: "var(--font-sans)",
  fontSize: 12.5,
  fontWeight: 600,
  cursor: "pointer",
  whiteSpace: "nowrap",
  transition: "background .15s, box-shadow .15s, opacity .15s",
};

export function PrimaryButton({ children, icon, full, disabled, style, ...rest }: BtnProps) {
  return (
    <button
      type="button"
      {...rest}
      disabled={disabled}
      style={{
        ...btnBase,
        width: full ? "100%" : undefined,
        border: "1px solid var(--color-ink)",
        background: "var(--color-ink)",
        color: "var(--color-gold)",
        opacity: disabled ? 0.45 : 1,
        cursor: disabled ? "default" : "pointer",
        ...style,
      }}
      onMouseEnter={(e) => {
        if (disabled) return;
        e.currentTarget.style.background = "#2b2825";
        e.currentTarget.style.boxShadow = "0 3px 14px rgba(26,24,22,0.22)";
      }}
      onMouseLeave={(e) => {
        e.currentTarget.style.background = "var(--color-ink)";
        e.currentTarget.style.boxShadow = "none";
      }}
    >
      {icon}
      {children}
    </button>
  );
}

// The secondary action: outlined, transparent — recedes next to primary.
export function GhostButton({ children, icon, full, disabled, style, ...rest }: BtnProps) {
  return (
    <button
      type="button"
      {...rest}
      disabled={disabled}
      style={{
        ...btnBase,
        width: full ? "100%" : undefined,
        border: "1px solid var(--border-strong)",
        background: "transparent",
        color: "var(--text-78)",
        opacity: disabled ? 0.45 : 1,
        cursor: disabled ? "default" : "pointer",
        ...style,
      }}
      onMouseEnter={(e) => {
        if (disabled) return;
        e.currentTarget.style.background = "rgba(26,24,22,0.03)";
      }}
      onMouseLeave={(e) => {
        e.currentTarget.style.background = "transparent";
      }}
    >
      {icon}
      {children}
    </button>
  );
}

// The gold-outlined "Add" affordance used to open editors/creators.
export function AddButton({ children, icon, style, ...rest }: BtnProps) {
  return (
    <button
      type="button"
      {...rest}
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: 6,
        height: 32,
        padding: "0 13px",
        borderRadius: "var(--radius-sm)",
        border: "1px solid var(--gold-border)",
        background: "rgba(194,163,112,0.08)",
        color: "var(--color-gold)",
        fontFamily: "var(--font-sans)",
        fontSize: 12,
        fontWeight: 600,
        cursor: "pointer",
        whiteSpace: "nowrap",
        transition: "background .15s",
        ...style,
      }}
      onMouseEnter={(e) => (e.currentTarget.style.background = "rgba(194,163,112,0.16)")}
      onMouseLeave={(e) => (e.currentTarget.style.background = "rgba(194,163,112,0.08)")}
    >
      {icon}
      {children}
    </button>
  );
}

// A bare icon-only control (edit/test/delete on list rows).
export function IconButton({
  children,
  title,
  onClick,
  color = "var(--text-50)",
}: {
  children: ReactNode;
  title: string;
  onClick: () => void;
  color?: string;
}) {
  return (
    <button
      type="button"
      title={title}
      onClick={onClick}
      style={{
        display: "inline-flex",
        alignItems: "center",
        justifyContent: "center",
        width: 28,
        height: 28,
        borderRadius: "var(--radius-sm)",
        border: "none",
        background: "transparent",
        color,
        cursor: "pointer",
        transition: "background .15s",
        flexShrink: 0,
      }}
      onMouseEnter={(e) => (e.currentTarget.style.background = "rgba(26,24,22,0.05)")}
      onMouseLeave={(e) => (e.currentTarget.style.background = "transparent")}
    >
      {children}
    </button>
  );
}

// The pill toggle switch used for enable/disable on list rows.
export function Toggle({ on, onClick }: { on: boolean; onClick: () => void }) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      onClick={onClick}
      style={{
        width: 32,
        height: 18,
        borderRadius: 9,
        border: "none",
        background: on ? "var(--color-ink)" : "rgba(26,24,22,0.15)",
        position: "relative",
        cursor: "pointer",
        flexShrink: 0,
        transition: "background .18s",
      }}
    >
      <span
        style={{
          display: "block",
          width: 13,
          height: 13,
          borderRadius: "50%",
          background: "#fff",
          position: "absolute",
          top: 2.5,
          left: on ? 16 : 2.5,
          transition: "left .18s",
          boxShadow: "0 1px 2px rgba(26,24,22,0.3)",
        }}
      />
    </button>
  );
}
