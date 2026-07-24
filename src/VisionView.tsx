import { useEffect, useState } from "react";
import { Play, Stop, Trash, PencilSimple, Eye, Plus } from "@phosphor-icons/react";

import {
  guardTest,
  listMacros,
  visionLoad,
  visionSave,
  visionStart,
  visionStatus,
  visionStop,
  type Guard,
} from "./api";
import { hsvToCss } from "./format";
import { GuardEditor, type GuardDraft } from "./GuardEditor";
import { useToast } from "./components/Toasts";
import {
  AddButton,
  Card,
  CardHead,
  EmptyState,
  GhostButton,
  IconButton,
  Page,
  PrimaryButton,
  Toggle,
} from "./components/ui";

// The Vision view is the autonomous bot: it watches the screen forever and acts
// the instant a trigger appears. This is a faithful port of the Python UI's
// Vision screen — every toast string is preserved character for character.
//
// The trigger editor (Add Trigger, per-row Edit and Test) is the shared guard
// editor mounted with its Vision-only extras enabled — the Surgical-capture
// button, the click-offset chip, and a "Save Trigger" label — via GuardEditor's
// `showSurgical` / `saveLabel` props. Triggers carry the `__vision__` macro id
// the Python source uses, and on save write back to the Vision list (never a
// macro's guards), persisted via `vision_save`. As in Python, `saveVisionEditor`
// drops the `macro_sequence` the orchestration UI edits and hardcodes
// `enabled: true` — a faithful reproduction of that source, quirks included.

// Python polls `vision_status` every 2 main ticks (500ms each) → once a second.
const POLL_MS = 1000;

interface TriggerRow {
  id: string;
  name: string;
  swatch: string;
  desc: string;
  rowOpacity: number;
  enabled: boolean;
}

function toRow(t: Guard): TriggerRow {
  const key = (t.key as string) || "?";
  const enabled = !!t.enabled;
  return {
    id: String(t.id ?? ""),
    name: t.name || "Trigger",
    swatch: hsvToCss(t.hsv_low, t.hsv_high),
    desc: "When it appears, " + (t.action === "key" ? "press " + key : "click it") + ".",
    rowOpacity: enabled ? 1 : 0.45,
    enabled,
  };
}

export function VisionView() {
  const { push: pushToast } = useToast();
  const [triggers, setTriggers] = useState<Guard[]>([]);
  const [running, setRunning] = useState(false);
  const [fired, setFired] = useState(0);
  const [editing, setEditing] = useState<GuardDraft | null>(null);
  const [macros, setMacros] = useState<{ id: string; name: string }[]>([]);

  // Load the saved triggers once on mount (Python loads them at bootstrap).
  useEffect(() => {
    let alive = true;
    (async () => {
      try {
        const res = await visionLoad();
        if (alive && res && res.ok) setTriggers(res.triggers || []);
      } catch {
        // best-effort, matches Python's silent bootstrap load
      }
    })();
    return () => {
      alive = false;
    };
  }, []);

  // Load the macro list once — the trigger editor's orchestration dropdown lists
  // every macro by id (which, as in MacrosView, is the macro's name).
  useEffect(() => {
    let alive = true;
    (async () => {
      try {
        const res = await listMacros();
        if (alive && res) setMacros(res.map((m) => ({ id: m.name, name: m.name })));
      } catch {
        // best-effort; the dropdown just shows no options
      }
    })();
    return () => {
      alive = false;
    };
  }, []);

  // Poll the running bot's status + event feed once a second while mounted.
  useEffect(() => {
    let alive = true;
    const tick = async () => {
      try {
        const res = await visionStatus();
        if (alive && res && res.ok) {
          setRunning(res.running);
          setFired(res.fired || 0);
        }
      } catch {
        // skip this tick
      }
    };
    tick();
    const h = setInterval(tick, POLL_MS);
    return () => {
      alive = false;
      clearInterval(h);
    };
  }, []);

  async function persist(list: Guard[]) {
    try {
      await visionSave(list);
    } catch {
      pushToast("error", "Vision save failed");
    }
  }

  function toggleTrigger(id: string) {
    const next = triggers.map((t) =>
      String(t.id ?? "") === id ? { ...t, enabled: !t.enabled } : t,
    );
    setTriggers(next);
    void persist(next);
  }

  async function deleteTrigger(id: string) {
    const next = triggers.filter((t) => String(t.id ?? "") !== id);
    setTriggers(next);
    await persist(next);
    pushToast("info", "Trigger removed.");
  }

  async function start() {
    await persist(triggers);
    try {
      const res = await visionStart();
      if (res && res.ok) {
        setRunning(true);
        pushToast("success", "Vision started — watching " + res.triggers + " trigger(s)");
      } else {
        pushToast("error", (res && res.error) || "Failed to start");
      }
    } catch (e) {
      pushToast("error", "Start failed: " + e);
    }
  }

  async function stop() {
    try {
      await visionStop();
      setRunning(false);
      pushToast("info", "Vision stopped.");
    } catch (e) {
      pushToast("error", "Stop failed: " + e);
    }
  }

  // ── Trigger editor (the shared guard editor, __vision__ host) ──
  // As in Python's addVisionTrigger, a new trigger is appended to the list in
  // memory only (not persisted) and its editor opened immediately; it reaches
  // disk when the user saves, toggles, deletes, or starts Vision.
  function addTrigger() {
    const t: Guard = {
      id: "v" + Date.now(),
      name: "New Trigger",
      method: "template",
      action: "click",
      key: "",
      hsv_low: [0, 0, 0],
      hsv_high: [179, 255, 255],
      region: [0, 0, 100, 100],
      min_area: 40,
      template_path: "",
      threshold: 0.8,
      ocr_text: "",
      resume_delay: 1,
      cooldown: 2,
      enabled: true,
    };
    setTriggers([...triggers, t]);
    openEditor(t);
  }

  function openEditor(t: Guard) {
    setEditing({ ...(t as GuardDraft), macroId: "__vision__", _isNew: false, _preview: null });
  }

  const closeEditor = () => setEditing(null);

  const updateEditor = (patch: Partial<GuardDraft>) =>
    setEditing((prev) => (prev ? { ...prev, ...patch } : prev));

  async function saveTrigger() {
    const ge = editing;
    if (!ge) return;
    // Faithful to Python's saveVisionEditor: the clean object drops the
    // `macro_sequence` the orchestration UI edits and hardcodes enabled:true.
    const clean: Guard = {
      id: ge.id,
      name: ge.name || "Trigger",
      method: ge.method || "color",
      action: ge.action,
      key: ge.key || "",
      hsv_low: ge.hsv_low,
      hsv_high: ge.hsv_high,
      region: ge.region,
      min_area: ge.min_area,
      template_path: ge.template_path || "",
      threshold: ge.threshold,
      ocr_text: ge.ocr_text || "",
      click_offset: ge.click_offset || [],
      click_line: ge.click_line || [],
      click_lines: ge.click_lines || [],
      resume_delay: ge.resume_delay,
      cooldown: ge.cooldown,
      enabled: true,
    };
    const next = triggers.map((t) => (t.id === clean.id ? clean : t));
    setTriggers(next);
    setEditing(null);
    await persist(next);
    pushToast("success", 'Trigger "' + clean.name + '" saved.');
  }

  async function testTrigger(trigger: Guard) {
    // If testing the trigger currently being edited, use the editor's working copy.
    const target =
      editing && editing.macroId === "__vision__" && editing.id === trigger.id ? editing : trigger;
    try {
      const res = await guardTest({
        id: target.id,
        name: target.name || "Guard",
        method: target.method || "color",
        action: target.action,
        key: target.key || "",
        hsv_low: target.hsv_low,
        hsv_high: target.hsv_high,
        region: target.region,
        min_area: target.min_area,
        template_path: target.template_path || "",
        threshold: target.threshold,
        ocr_text: target.ocr_text || "",
        resume_delay: target.resume_delay,
        cooldown: target.cooldown,
        enabled: true,
      });
      if (res && res.ok) {
        pushToast("success", "Match found at (" + res.found_x + ", " + res.found_y + ")");
      } else {
        pushToast("info", (res && res.message) || "No match");
      }
      // Show the preview in the editor if it's open for this trigger.
      if (editing && editing.macroId === "__vision__" && res && res.preview) {
        updateEditor({
          _preview: { src: "data:image/jpeg;base64," + res.preview, ok: res.ok, msg: res.message },
        });
      }
    } catch (e) {
      pushToast("error", "Test failed: " + e);
    }
  }

  const enabledCount = triggers.filter((t) => t.enabled).length;
  const startDisabled = running || enabledCount === 0;
  const statusColor = running ? "var(--verdict-allowed)" : "var(--text-32)";
  const statusAnim = running ? "vecPulse 1.4s ease-in-out infinite" : "none";

  return (
    <Page
      title="Watch and react."
      subtitle="Tell Clawmation what to look for on screen — a reconnect button, an OK dialog, a reward popup — and it clicks or presses the key for you the moment it shows up. It keeps watching for as long as you leave it on."
    >
      {/* On/off hero — the whole screen exists to flip one switch, so it leads.
          Plain-language state, not a telemetry readout; the cat indicator carries
          the "it's running" cue while you're away from the desk. */}
      <Card
        pad="20px 24px"
        style={{
          marginBottom: 18,
          display: "flex",
          alignItems: "center",
          gap: 18,
          flexWrap: "wrap",
          border: "1px solid " + (running ? "var(--gold-border)" : "var(--border-default)"),
          background: running ? "rgba(194,163,112,0.06)" : "var(--surface-card)",
        }}
      >
        <span
          style={{
            width: 12,
            height: 12,
            borderRadius: "50%",
            background: statusColor,
            animation: statusAnim,
            flexShrink: 0,
          }}
        />
        <div style={{ minWidth: 0, flex: 1 }}>
          <div
            style={{
              fontFamily: "var(--font-sans)",
              fontSize: 15,
              fontWeight: 600,
              color: "var(--color-ink)",
            }}
          >
            {running ? "Clawmation is watching the screen." : "Clawmation isn't watching right now."}
          </div>
          <div
            style={{
              fontFamily: "var(--font-sans)",
              fontSize: 12.5,
              color: "var(--text-50)",
              marginTop: 3,
            }}
          >
            {enabledCount === 0
              ? "Add something to watch for below, then press Start."
              : running
                ? "Watching " +
                  enabledCount +
                  (enabledCount === 1 ? " thing" : " things") +
                  " · reacted " +
                  fired +
                  (fired === 1 ? " time" : " times") +
                  " so far."
                : fired > 0
                  ? "Last watch reacted " +
                    fired +
                    (fired === 1 ? " time" : " times") +
                    " · ready to watch " +
                    enabledCount +
                    (enabledCount === 1 ? " thing" : " things") +
                    " again."
                  : "Ready to watch " +
                    enabledCount +
                    (enabledCount === 1 ? " thing" : " things") +
                    "."}
          </div>
        </div>
        {running ? (
          <GhostButton
            onClick={stop}
            icon={<Stop size={13} />}
            style={{ color: "var(--verdict-blocked)", flexShrink: 0 }}
          >
            Stop
          </GhostButton>
        ) : (
          <PrimaryButton
            onClick={start}
            disabled={startDisabled}
            icon={<Play size={14} />}
            style={{ flexShrink: 0 }}
          >
            Start watching
          </PrimaryButton>
        )}
      </Card>

      {/* The things to watch for */}
      <Card pad={22}>
        <CardHead
          title="What to watch for"
          desc="Each one waits for something to appear on screen, then clicks it or presses a key for you."
          action={
            <AddButton onClick={addTrigger} icon={<Plus size={13} />}>
              Add something
            </AddButton>
          }
        />

        {triggers.length > 0 ? (
          <div style={{ display: "flex", flexDirection: "column", gap: 7 }}>
            {triggers.map((t) => {
              const v = toRow(t);
              return (
                <div
                  key={v.id}
                  style={{
                    display: "grid",
                    gridTemplateColumns: "auto 1fr auto auto auto auto",
                    gap: 10,
                    alignItems: "center",
                    padding: "10px 12px",
                    borderRadius: "var(--radius-md)",
                    border: "1px solid var(--border-faint)",
                    background: "var(--surface-card-strong)",
                    opacity: v.rowOpacity,
                  }}
                >
                  <span
                    style={{
                      width: 18,
                      height: 18,
                      borderRadius: 5,
                      border: "1px solid var(--border-default)",
                      background: v.swatch,
                      flexShrink: 0,
                    }}
                  />
                  <div style={{ minWidth: 0 }}>
                    <div
                      style={{
                        fontFamily: "var(--font-sans)",
                        fontSize: 12.5,
                        fontWeight: 600,
                        color: "var(--color-ink)",
                        whiteSpace: "nowrap",
                        overflow: "hidden",
                        textOverflow: "ellipsis",
                      }}
                    >
                      {v.name}
                    </div>
                    <div
                      style={{
                        fontFamily: "var(--font-sans)",
                        fontSize: 11,
                        color: "var(--text-50)",
                        marginTop: 2,
                      }}
                    >
                      {v.desc}
                    </div>
                  </div>
                  <IconButton title="Edit" onClick={() => openEditor(t)}>
                    <PencilSimple size={14} />
                  </IconButton>
                  <IconButton title="Test now" onClick={() => testTrigger(t)}>
                    <Eye size={15} />
                  </IconButton>
                  <Toggle on={v.enabled} onClick={() => toggleTrigger(v.id)} />
                  <IconButton title="Delete" onClick={() => deleteTrigger(v.id)} color="var(--text-40)">
                    <Trash size={14} />
                  </IconButton>
                </div>
              );
            })}
          </div>
        ) : (
          <EmptyState
            icon={<Eye size={28} weight="thin" />}
            title="Nothing to watch for yet"
            hint="Add one, point it at the button or text on screen, then press Start watching — Clawmation reacts the instant it shows up."
          />
        )}

        {editing && (
          <div style={{ marginTop: 14 }}>
            <GuardEditor
              draft={editing}
              macros={macros}
              onChange={updateEditor}
              onTest={() => testTrigger(editing)}
              onSave={saveTrigger}
              onCancel={closeEditor}
              push={pushToast}
              showSurgical
              saveLabel="Save Trigger"
            />
          </div>
        )}
      </Card>

    </Page>
  );
}
