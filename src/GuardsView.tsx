import { ShieldCheck } from "@phosphor-icons/react";

import { Card, CardHead, LABEL, Page, PrimaryButton } from "./components/ui";

// The Guards screen (`ai` view) is a static explainer, ported verbatim from the
// Python UI's Guards view. It carries no live data — the only interactive part
// is the "Open Macros" button, which navigates to the Macros tab where guards
// are actually attached.

const STEPS = [
  "You tell the guard what to look for — a color on screen (like the Reconnect button) and where to look.",
  "While the macro plays, the guard checks the screen a few times a second in the background.",
  "The moment it spots the trigger, it pauses the macro, clicks the button (or presses a key), and waits.",
  "After the wait, it resumes the macro exactly where it stopped. Your loop never dies.",
];

const RECONNECT: [string, string][] = [
  ["Macro", "your farming loop, set to ∞ reps"],
  ["Guard trigger", "color of the Reconnect button"],
  ["Guard action", "click it"],
  ["Wait after", "~10s (loading back in)"],
];

export function GuardsView({ onOpenMacros }: { onOpenMacros: () => void }) {
  return (
    <Page
      title="Guards."
      subtitle="Guards watch the screen while your macro runs. When something goes wrong — a disconnect, a popup, a dialog — a guard pauses the macro, handles it, then picks up exactly where it left off."
    >
      {/* Guards are attached per-macro, not here — a gold call-to-action points
          at the Macros tab where the work actually happens. */}
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: 14,
          padding: "16px 20px",
          border: "1px solid var(--gold-border)",
          background: "rgba(194,163,112,0.06)",
          borderRadius: "var(--radius-lg)",
          marginBottom: 18,
        }}
      >
        <ShieldCheck size={18} color="var(--color-gold)" style={{ flexShrink: 0 }} />
        <span
          style={{
            fontFamily: "var(--font-sans)",
            fontSize: 12.5,
            lineHeight: 1.55,
            color: "var(--color-ink)",
          }}
        >
          Guards live on each macro. Open the <strong>Macros</strong> tab, then press the{" "}
          <strong>Guards</strong> button on any macro to add one.
        </span>
        <PrimaryButton onClick={onOpenMacros} style={{ marginLeft: "auto", flexShrink: 0 }}>
          Open Macros
        </PrimaryButton>
      </div>

      <div
        style={{
          display: "grid",
          gridTemplateColumns: "minmax(0, 1fr) 340px",
          gap: 18,
          alignItems: "start",
        }}
      >
        {/* The concept, as a numbered sequence. */}
        <Card pad={22}>
          <CardHead title="How a guard works" />
          <div style={{ display: "flex", flexDirection: "column", gap: 14 }}>
            {STEPS.map((text, i) => (
              <div
                key={i}
                style={{
                  display: "grid",
                  gridTemplateColumns: "26px 1fr",
                  gap: 14,
                  alignItems: "start",
                }}
              >
                <span
                  style={{
                    display: "inline-flex",
                    alignItems: "center",
                    justifyContent: "center",
                    width: 26,
                    height: 26,
                    borderRadius: "50%",
                    border: "1px solid var(--gold-border)",
                    background: "rgba(194,163,112,0.10)",
                    fontFamily: "var(--font-mono)",
                    fontSize: 11,
                    color: "var(--color-gold)",
                    flexShrink: 0,
                  }}
                >
                  {i + 1}
                </span>
                <span className="t-body-sm" style={{ paddingTop: 3 }}>
                  {text}
                </span>
              </div>
            ))}
          </div>
        </Card>

        {/* The same idea made concrete — a worked example beside the steps. */}
        <Card pad={20}>
          <CardHead title="The classic reconnect setup" />
          <div className="ds-inset" style={{ overflow: "hidden" }}>
            {RECONNECT.map(([label, value], i) => (
              <div
                key={label}
                style={{
                  padding: "11px 14px",
                  borderBottom:
                    i < RECONNECT.length - 1 ? "1px solid var(--border-faint)" : undefined,
                }}
              >
                <div style={LABEL}>{label}</div>
                <div
                  style={{
                    marginTop: 4,
                    fontFamily: "var(--font-sans)",
                    fontSize: 12.5,
                    color: "var(--color-ink)",
                    lineHeight: 1.5,
                  }}
                >
                  {value}
                </div>
              </div>
            ))}
          </div>
        </Card>
      </div>

      {/* The how-to, as a full-width band below the row: a lead line, then the
          three concrete controls spread across the width as labelled cells. */}
      <Card pad={22} style={{ marginTop: 18 }}>
        <CardHead title="Setting one up" />
        <p className="t-body-sm" style={{ margin: "0 0 16px", maxWidth: "68ch" }}>
          In the Macros tab, press <strong>Guards</strong> on a macro, then{" "}
          <strong>Add Guard</strong>. Two clicks do most of the work:
        </p>
        <div
          style={{
            display: "grid",
            gridTemplateColumns: "repeat(3, minmax(0, 1fr))",
            gap: 16,
          }}
        >
          <div className="ds-inset" style={{ padding: "14px 16px" }}>
            <p className="t-body-sm" style={{ margin: 0 }}>
              <strong>Pick from screen</strong> samples the trigger color for you — just click the
              Reconnect button and Clawmation builds a tolerance range around it automatically.
            </p>
          </div>
          <div className="ds-inset" style={{ padding: "14px 16px" }}>
            <p className="t-body-sm" style={{ margin: 0 }}>
              <strong>Drag area</strong> narrows where the guard looks, so it only watches the part of
              the screen where the dialog appears. Smaller areas are faster and avoid false triggers.
            </p>
          </div>
          <div className="ds-inset" style={{ padding: "14px 16px" }}>
            <p className="t-body-sm" style={{ margin: 0 }}>
              <strong>Wait after</strong> gives the game time to react (load screens, reconnecting)
              before the macro resumes. Use the <strong>Test</strong> button to confirm the guard sees
              the button before you rely on it.
            </p>
          </div>
        </div>
      </Card>
    </Page>
  );
}
