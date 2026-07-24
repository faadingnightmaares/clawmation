import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { invoke } from "@tauri-apps/api/core";

import App from "./App";
import type { Status } from "./api";

// Mock the Tauri command seam: `invoke` never touches a real backend in tests.
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const mockInvoke = vi.mocked(invoke);

function fakeStatus(overrides: Partial<Status> = {}): Status {
  return {
    mode: "idle",
    elapsed: 0,
    record_paused: false,
    play_iteration: 0,
    play_total_reps: 0,
    window: { found: false, title: "", size: "—" },
    capture: { backend: "dxgi", fps: 0 },
    recorded_count: 0,
    last_macro: "",
    macro_count: 3,
    indicator_alive: false,
    config: {
      resolution: [1920, 1080],
      capture_backend: "dxgi",
      hotkey_record: "f6",
      hotkey_play: "f4",
      hotkey_stop: "f3",
      indicator_on_top: true,
    },
    log: [],
    ...overrides,
  };
}

// The Dashboard (the default view) fires `list_macros` + `get_run_history` on
// mount and expects arrays back; route those to `[]` and let every other
// command — `get_status` above all — resolve to the status payload under test.
function mockBackend(status: Status) {
  mockInvoke.mockImplementation((cmd) =>
    cmd === "list_macros" || cmd === "get_run_history"
      ? Promise.resolve([])
      : Promise.resolve(status),
  );
}

beforeEach(() => mockInvoke.mockReset());
afterEach(() => cleanup());

describe("App shell — Tauri↔React seam", () => {
  it("polls get_status and renders its payload (idle indicator + activity log)", async () => {
    mockBackend(
      fakeStatus({ log: [{ t: "12:00:00", level: "info", msg: "Ready to record" }] }),
    );

    render(<App />);

    // The indicator and the log line both come from the invoked payload.
    expect(await screen.findByText("Ready")).toBeInTheDocument();
    expect(await screen.findByText("Ready to record")).toBeInTheDocument();
    expect(mockInvoke).toHaveBeenCalledWith("get_status");
  });

  it("derives the top-bar mode label from status", async () => {
    mockBackend(fakeStatus({ mode: "recording" }));

    render(<App />);

    // Scope to the top bar: the Dashboard's Mode fact also reads "Recording".
    const banner = screen.getByRole("banner");
    expect(await within(banner).findByText("Recording")).toBeInTheDocument();
  });

  it("swaps the active view when a sidebar item is clicked", async () => {
    mockBackend(fakeStatus());

    render(<App />);

    // Dashboard is the default view.
    expect(await screen.findByText("Activity")).toBeInTheDocument();

    // Clicking "Settings" swaps the main content.
    fireEvent.click(screen.getByRole("button", { name: /Settings/ }));
    expect(await screen.findByText("Make Clawmation yours.")).toBeInTheDocument();
    expect(screen.queryByText("Activity")).not.toBeInTheDocument();
  });
});
