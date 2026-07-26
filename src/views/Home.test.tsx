import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn<(cmd: string, args?: unknown) => Promise<unknown>>();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (c: string, a?: unknown) => invoke(c, a) }));

import { Home } from "./Home";

function mockBackend(targetId: string | null = null) {
  invoke.mockImplementation(async (cmd, rawArgs) => {
    switch (cmd) {
      case "get_stats_summary":
        return {
          total_macros: 0,
          total_guards: 0,
          total_chains: 0,
          total_schedules: 0,
          total_play_seconds: 0,
          most_played: "",
          most_played_count: 0,
        };
      case "get_run_history":
        return [];
      case "anti_afk_list_windows":
        return [
          { id: "A10", title: "Roblox", pid: 101 },
          { id: "B20", title: "Roblox", pid: 202 },
        ];
      case "anti_afk_get":
        return {
          enabled: false,
          target_id: targetId,
          interval_min: 15,
          action: "random",
          status: "off",
          error: null,
        };
      case "anti_afk_update": {
        const args = rawArgs as
          | {
              targetId?: string;
              intervalMin?: number;
              action?: "jump" | "walk" | "camera" | "random";
              enabled?: boolean;
            }
          | undefined;
        return {
          ok: true,
          state: {
            enabled: args?.enabled ?? false,
            target_id: args?.targetId ?? targetId,
            interval_min: args?.intervalMin ?? 15,
            action: args?.action ?? "random",
            status: args?.enabled ? "active" : "off",
            error: null,
          },
        };
      }
      default:
        return { ok: true };
    }
  });
}

function view() {
  return render(<Home status={null} navigate={() => {}} />);
}

describe("Home Anti-AFK controls", () => {
  beforeEach(() => invoke.mockReset());
  afterEach(() => vi.useRealTimers());

  it("lists distinguishable game instances and requires a target before enabling", async () => {
    mockBackend();
    view();

    expect(await screen.findByText("Anti-AFK")).toBeInTheDocument();
    expect(screen.getByText("15 minutes")).toBeInTheDocument();
    expect(screen.getByRole("switch", { name: "Anti-AFK" })).toBeDisabled();
    expect(screen.queryByRole("button", { name: "Refresh open windows" })).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("combobox", { name: "Game or app" }));
    expect(await screen.findByText("Roblox · PID 101")).toBeInTheDocument();
    expect(screen.getByText("Roblox · PID 202")).toBeInTheDocument();
  });

  it("refreshes the available windows automatically every 10 seconds", async () => {
    vi.useFakeTimers();
    mockBackend();
    view();

    await act(async () => {
      await Promise.resolve();
    });
    expect(
      invoke.mock.calls.filter(([command]) => command === "anti_afk_list_windows"),
    ).toHaveLength(1);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(10_000);
    });
    expect(
      invoke.mock.calls.filter(([command]) => command === "anti_afk_list_windows"),
    ).toHaveLength(2);
  });

  it("offers multiple anti-AFK actions including a random mix", async () => {
    mockBackend("A10");
    view();

    fireEvent.click(await screen.findByRole("combobox", { name: "Anti-AFK action" }));
    expect(await screen.findByText("Jump")).toBeInTheDocument();
    expect(screen.getByText("Walk")).toBeInTheDocument();
    expect(screen.getByText("Camera nudge")).toBeInTheDocument();
    expect(screen.getAllByText("Random mix")).toHaveLength(2);
  });

  it("enables the selected target and delegates the immediate jump to the backend", async () => {
    mockBackend("B20");
    view();

    const toggle = await screen.findByRole("switch", { name: "Anti-AFK" });
    fireEvent.click(toggle);

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("anti_afk_update", {
        targetId: undefined,
        intervalMin: undefined,
        action: undefined,
        enabled: true,
      }),
    );
  });

  it("persists interval changes from the slider", async () => {
    mockBackend("A10");
    view();

    const slider = await screen.findByRole("slider", { name: "Anti-AFK interval" });
    fireEvent.input(slider, { target: { value: "14" } });
    fireEvent.keyUp(slider, { key: "ArrowLeft" });

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith(
        "anti_afk_update",
        expect.objectContaining({ intervalMin: 14 }),
      ),
    );
  });
});
