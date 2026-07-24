import { useState } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";

import { StepEditor, type StepEditorTestResult } from "./StepEditor";
import type { Step } from "./api";

afterEach(() => cleanup());

// A complete Step with per-test overrides — the editor's list is always full
// objects (see `makeStep`), so tests build full objects too.
function step(overrides: Partial<Step>): Step {
  return {
    id: "x",
    type: "click",
    enabled: true,
    label: "",
    x: 0,
    y: 0,
    key: "",
    text: "",
    delay: 0,
    scroll_amount: 0,
    detect_mode: "color",
    hsv_low: [0, 0, 0],
    hsv_high: [179, 255, 255],
    template: "",
    region: [0, 0, 100, 100],
    min_area: 40,
    timeout: 10,
    confidence: 0.8,
    ...overrides,
  };
}

// Drive the editor with real list state so the in-memory transforms (insert,
// delete, move, toggle, bulk) actually run and re-render.
function Harness({
  initial,
  testResult = null,
  onSave = () => {},
  onRun = () => {},
  onTest = () => {},
  onPickColor = () => {},
  onClose = () => {},
}: {
  initial: Step[];
  testResult?: StepEditorTestResult | null;
  onSave?: () => void;
  onRun?: () => void;
  onTest?: (idx: number) => void;
  onPickColor?: (idx: number) => void;
  onClose?: () => void;
}) {
  const [steps, setSteps] = useState<Step[]>(initial);
  return (
    <StepEditor
      steps={steps}
      loading={false}
      testResult={testResult}
      setSteps={setSteps}
      onSave={onSave}
      onRun={onRun}
      onTest={onTest}
      onPickColor={onPickColor}
      onClose={onClose}
    />
  );
}

describe("StepEditor", () => {
  it("renders a labeled row per step, and the placeholder when empty", () => {
    render(
      <Harness
        initial={[
          step({ id: "a", type: "click", label: "Open menu" }),
          step({ id: "b", type: "wait_for", label: "Wait boss" }),
        ]}
      />,
    );

    expect(screen.getByDisplayValue("Open menu")).toBeInTheDocument();
    expect(screen.getByDisplayValue("Wait boss")).toBeInTheDocument();
    // The type badges are <span>s; scope past the same-named insert buttons.
    expect(screen.getByText("Click", { selector: "span" })).toBeInTheDocument();
    expect(screen.getByText("Wait For", { selector: "span" })).toBeInTheDocument();

    cleanup();
    render(<Harness initial={[]} />);
    expect(
      screen.getByText(
        "No steps. The recording had no recognizable actions, or it hasn't been converted yet.",
      ),
    ).toBeInTheDocument();
  });

  it("inserts a Wait For step after the clicked row", () => {
    render(<Harness initial={[step({ id: "a", type: "click", label: "A" })]} />);
    expect(screen.getAllByPlaceholderText("Label")).toHaveLength(1);

    // The row's insert bar "Wait For" button (distinct from a type badge).
    fireEvent.click(screen.getByRole("button", { name: "Wait For" }));

    expect(screen.getAllByPlaceholderText("Label")).toHaveLength(2);
    // The inserted step defaults to the "Wait For" label and its detection mode
    // defaults to `template` — the monolith's (non-matching) default.
    expect(screen.getByDisplayValue("Wait For")).toBeInTheDocument();
  });

  it("Set all rewrites every delay's value and label", () => {
    render(
      <Harness
        initial={[
          step({ id: "d1", type: "delay", delay: 1, label: "Wait 1s" }),
          step({ id: "d2", type: "delay", delay: 5, label: "Wait 5s" }),
        ]}
      />,
    );

    // Scope to the bulk bar so we grab its input, not a row's delay field.
    const bulkBar = screen.getByText("Bulk delays").parentElement as HTMLElement;
    const bulkInput = within(bulkBar).getByPlaceholderText("sec");
    fireEvent.change(bulkInput, { target: { value: "2" } });
    fireEvent.click(screen.getByRole("button", { name: "Set all" }));

    expect(screen.getAllByDisplayValue("Wait 2s")).toHaveLength(2);
  });

  it("wires Run, Save, Close, and per-row Test to their callbacks", () => {
    const onRun = vi.fn();
    const onSave = vi.fn();
    const onClose = vi.fn();
    const onTest = vi.fn();
    render(
      <Harness
        initial={[step({ id: "a", type: "click", label: "A" })]}
        onRun={onRun}
        onSave={onSave}
        onClose={onClose}
        onTest={onTest}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Run" }));
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    fireEvent.click(screen.getByTitle("Close step editor"));
    fireEvent.click(screen.getByTitle("Test this step"));

    expect(onRun).toHaveBeenCalledTimes(1);
    expect(onSave).toHaveBeenCalledTimes(1);
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(onTest).toHaveBeenCalledWith(0);
  });

  it("renders the test result banner and preview image", () => {
    render(
      <Harness
        initial={[step({ id: "a", type: "find_click", label: "A" })]}
        testResult={{
          idx: 0,
          ok: true,
          message: "would click (5, 5) — 1 color match(es)",
          preview: "data:image/jpeg;base64,AAAA",
        }}
      />,
    );

    expect(screen.getByText("Match")).toBeInTheDocument();
    expect(screen.getByText("would click (5, 5) — 1 color match(es)")).toBeInTheDocument();
    const img = document.querySelector("img") as HTMLImageElement;
    expect(img.src).toBe("data:image/jpeg;base64,AAAA");
  });

  it("offers Pick Color on a color detection step and fires onPickColor", () => {
    const onPickColor = vi.fn();
    render(
      <Harness
        initial={[step({ id: "a", type: "find_click", label: "A", detect_mode: "color" })]}
        onPickColor={onPickColor}
      />,
    );

    // Full-range default → the "any" hint (no colour picked yet), not a swatch.
    expect(screen.getByText("any")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Pick/ }));
    expect(onPickColor).toHaveBeenCalledWith(0);
  });

  it("shows the swatch (not the 'any' hint) once a colour is set", () => {
    render(
      <Harness
        initial={[
          step({
            id: "a",
            type: "find_click",
            detect_mode: "color",
            hsv_low: [20, 100, 100],
            hsv_high: [40, 255, 255],
          }),
        ]}
      />,
    );

    expect(screen.queryByText("any")).not.toBeInTheDocument();
    // The swatch carries the HSV window as its tooltip.
    expect(document.querySelector('[title^="H 20"]')).toBeTruthy();
  });
});
