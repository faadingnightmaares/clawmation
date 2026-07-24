import type { CSSProperties, ReactNode } from "react";

import type { Status } from "./api";
import { fmtHotkey } from "./format";
import { Card, Page } from "./components/ui";

// The Guide is static teaching prose, ported verbatim from the Python UI's
// `guide` view. The only live data is the keyboard-shortcut bindings, which read
// the current hotkeys off the status heartbeat.

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <Card pad="22px 26px">
      <h2 className="t-h3" style={{ marginBottom: 14 }}>
        {title}
      </h2>
      {children}
    </Card>
  );
}

/** A bold lead-in inside a body paragraph, matching the source's `<strong>`. */
function Lead({ children }: { children: ReactNode }) {
  return <strong style={{ fontWeight: 600, color: "var(--color-ink)" }}>{children}</strong>;
}

const listStyle: CSSProperties = {
  margin: 0,
  paddingLeft: 22,
  display: "flex",
  flexDirection: "column",
  gap: 10,
};

export function GuideView({ status }: { status: Status | null }) {
  const cfg = status?.config;
  const shortcuts: [string, string][] = [
    ["Record", fmtHotkey(cfg?.hotkey_record ?? "")],
    ["Play", fmtHotkey(cfg?.hotkey_play ?? "")],
    ["Stop", fmtHotkey(cfg?.hotkey_stop ?? "")],
  ];

  return (
    <Page title="Getting to know Clawmation.">
      <div style={{ display: "flex", flexDirection: "column", gap: 18, maxWidth: 680 }}>
        <Section title="Quick start">
          <ol className="t-body" style={listStyle}>
            <li>Open Clawmation near the window you want to automate.</li>
            <li>Press Record and do the thing once, at your normal speed.</li>
            <li>Press Stop — Clawmation saves every click, key, and pause automatically.</li>
            <li>Click the macro name to rename it something you'll recognize.</li>
            <li>Play it back anytime — drag the repeat dial to loop it as many times as you need.</li>
          </ol>
        </Section>

        <Section title="Guards & Vision">
          <ul className="t-body" style={listStyle}>
            <li>
              <Lead>Guards</Lead> watch the screen while a macro plays. When something goes wrong — a
              disconnect, a popup, a dialog — a guard pauses the macro, handles it (click a button,
              press a key), then resumes exactly where it left off. Add guards from any macro card.
            </li>
            <li>
              <Lead>Vision</Lead> is a fully autonomous watcher. Add triggers for things Clawmation
              should react to — a reconnect button, an OK dialog, a reward popup — and it watches the
              whole screen forever, acting the instant they appear. Triggers use Color, Image, or OCR
              detection.
            </li>
            <li>
              Both scan the whole screen by default. You can limit the search area to speed up
              detection, but it's optional.
            </li>
          </ul>
        </Section>

        <Section title="Chains & Scheduling">
          <ul className="t-body" style={listStyle}>
            <li>
              <Lead>Chains</Lead> let you run multiple macros in sequence. Build a chain, add macros
              in order, set a delay between them, and run the whole routine with one click. Chains can
              repeat infinitely or a fixed number of times.
            </li>
            <li>
              <Lead>Scheduling</Lead> runs macros or chains automatically on a timer. Set an interval
              (every N minutes) or a daily time (e.g. 08:00), and Clawmation will start the
              macro/chain for you — even if you're away. Schedules can be paused/resumed anytime.
            </li>
          </ul>
        </Section>

        <Section title="Organizing macros">
          <ul className="t-body" style={listStyle}>
            <li>
              <Lead>Categories & notes</Lead> — tag macros with categories (e.g. "Farming",
              "Dailies") and add notes to remember what each one does. Filter by category or search
              across names, notes, and categories.
            </li>
            <li>
              <Lead>Templates</Lead> — save a macro as a reusable template, then create new macros
              from it. Great for common patterns you tweak slightly each time.
            </li>
            <li>
              <Lead>Bulk operations</Lead> — select multiple macros with checkboxes, then delete or
              export them all at once. Export creates a .zip bundle you can share or back up.
            </li>
          </ul>
        </Section>

        <Section title="Keyboard shortcuts">
          <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
            {shortcuts.map(([label, key]) => (
              <div
                key={label}
                style={{
                  display: "flex",
                  alignItems: "center",
                  justifyContent: "space-between",
                  padding: "8px 0",
                  borderBottom: "1px solid var(--border-faint)",
                }}
              >
                <span className="t-body-sm" style={{ color: "var(--text-78)" }}>
                  {label}
                </span>
                <kbd
                  style={{
                    fontFamily: "var(--font-mono)",
                    fontSize: "var(--text-sm)",
                    color: "var(--text-68)",
                    border: "1px solid var(--border-strong)",
                    borderRadius: "var(--radius-xs)",
                    padding: "2px 8px",
                    background: "var(--surface-card-strong)",
                  }}
                >
                  {key || "—"}
                </kbd>
              </div>
            ))}
          </div>
        </Section>
      </div>
    </Page>
  );
}
