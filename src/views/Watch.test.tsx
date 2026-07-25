import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

// The Tauri seam. Every command Watch reaches for answers from here.
const invoke = vi.fn<(cmd: string, args?: unknown) => Promise<unknown>>();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (c: string, a?: unknown) => invoke(c, a) }));

import { Watch } from "./Watch";

/** Commands issued so far, in order. What these tests actually assert on. */
const calls = () => invoke.mock.calls.map(([cmd]) => cmd);

function mockBackend(running: boolean) {
  invoke.mockImplementation(async (cmd) => {
    switch (cmd) {
      case "vision_load":
        return { ok: true, triggers: [] };
      case "vision_status":
        return { ok: true, running, fired: 0, log: [] };
      case "vision_start":
        return { ok: true, triggers: 1 };
      case "guard_pick_color":
        return { ok: true, hsv_low: [20, 80, 80], hsv_high: [40, 255, 255], hex: "#ffcc00" };
      default:
        return { ok: true };
    }
  });
}

const view = () => render(<Watch status={null} navigate={() => {}} />);

/** Open the editor and show it something to watch for. Saving is deliberately
 *  blocked until then: an untouched draft carries the full colour spectrum, which
 *  matches the whole screen as one blob. */
async function openEditorAndPickAColour() {
  fireEvent.click(await screen.findByRole("button", { name: /add the first thing/i }));
  fireEvent.click(await screen.findByRole("button", { name: /pick its colour/i }));
  await screen.findByText(/colour picked/i);
}

describe("Watch, saving a trigger", () => {
  beforeEach(() => invoke.mockReset());

  it("saves without starting the watcher", async () => {
    // Save saves. Arming is Start's job, so describing something to look for
    // never puts the app on the screen clicking things in the same gesture.
    mockBackend(false);
    view();

    await openEditorAndPickAColour();
    fireEvent.click(await screen.findByRole("button", { name: /^save trigger$/i }));

    await waitFor(() => expect(calls()).toContain("vision_save"));
    expect(calls()).not.toContain("vision_start");

    // A Watch trigger is saved with no wait between repeats, so a thing that
    // keeps reappearing gets collected as fast as the screen can be read.
    const saved = invoke.mock.calls.find(([cmd]) => cmd === "vision_save")?.[1] as {
      triggers: { cooldown: number }[];
    };
    expect(saved.triggers[0].cooldown).toBe(0);
  });

  it("saves the same way while the watcher is already running", async () => {
    mockBackend(true);
    view();

    await screen.findByRole("button", { name: /^stop$/i });
    await openEditorAndPickAColour();
    fireEvent.click(await screen.findByRole("button", { name: /^save trigger$/i }));

    await waitFor(() => expect(calls()).toContain("vision_save"));
    expect(calls()).not.toContain("vision_start");
  });

  it("will not save a trigger that was never shown anything", async () => {
    mockBackend(false);
    view();

    fireEvent.click(await screen.findByRole("button", { name: /add the first thing/i }));
    expect(await screen.findByRole("button", { name: /^save trigger$/i })).toBeDisabled();
  });
});

describe("Watch, starting the watcher", () => {
  beforeEach(() => invoke.mockReset());

  it("saves the list before arming, since the backend starts from the file", async () => {
    mockBackend(false);
    view();

    await openEditorAndPickAColour();
    fireEvent.click(await screen.findByRole("button", { name: /^save trigger$/i }));
    await waitFor(() => expect(calls()).toContain("vision_save"));

    fireEvent.click(await screen.findByRole("button", { name: /start watching/i }));

    await waitFor(() => expect(calls()).toContain("vision_start"));
    expect(calls().lastIndexOf("vision_save")).toBeLessThan(calls().indexOf("vision_start"));
  });
});
