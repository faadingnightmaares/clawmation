import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

// The Tauri seam. Every command Watch reaches for answers from here.
const invoke = vi.fn<(cmd: string, args?: unknown) => Promise<unknown>>();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (c: string, a?: unknown) => invoke(c, a) }));

import { Watch } from "./Watch";

/** Commands issued so far, in order — what these tests actually assert on. */
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
      default:
        return { ok: true };
    }
  });
}

const view = () => render(<Watch status={null} navigate={() => {}} />);

describe("Watch — saving a trigger", () => {
  beforeEach(() => invoke.mockReset());

  it("starts watching in the same click when nothing is running", async () => {
    mockBackend(false);
    view();

    fireEvent.click(await screen.findByRole("button", { name: /add the first thing/i }));
    // A fresh draft watches for a colour, which is savable straight away, so the
    // save button is live without picking anything.
    fireEvent.click(await screen.findByRole("button", { name: /save & start watching/i }));

    await waitFor(() => expect(calls()).toContain("vision_start"));
    expect(calls().indexOf("vision_save")).toBeLessThan(calls().indexOf("vision_start"));
  });

  it("only saves when the watcher is already running", async () => {
    mockBackend(true);
    view();

    await screen.findByRole("button", { name: /stop watching/i });
    fireEvent.click(await screen.findByRole("button", { name: /add the first thing/i }));
    fireEvent.click(await screen.findByRole("button", { name: /^save trigger$/i }));

    await waitFor(() => expect(calls()).toContain("vision_save"));
    expect(calls()).not.toContain("vision_start");
  });
});
