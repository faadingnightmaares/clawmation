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

function view(status: Status | null = null, navigate = () => {}) {
  return render(<Home status={status} navigate={navigate} />);
}

describe("Home Anti-AFK controls", () => {
  beforeEach(() => invoke.mockReset());
  afterEach(() => vi.useRealTimers());

  it("lists distinguishable game instances and requires a target before enabling", async () => {
    mockBackend();
    view();

    expect(
      await screen.findByRole("heading", { name: "Anti-AFK" }),
    ).toBeInTheDocument();
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

    const actionSelect = await screen.findByRole("combobox", {
      name: "Anti-AFK action",
    });
    await waitFor(() => expect(actionSelect).toBeEnabled());
    fireEvent.click(actionSelect);

    expect(await screen.findByRole("option", { name: "Jump" })).toBeInTheDocument();
    expect(screen.getByRole("option", { name: "Walk" })).toBeInTheDocument();
    expect(screen.getByRole("option", { name: "Camera nudge" })).toBeInTheDocument();
    expect(screen.getByRole("option", { name: "Random mix" })).toBeInTheDocument();
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

describe("Home workspace", () => {
  beforeEach(() => {
    invoke.mockReset();
    mockBackend();
  });

  it("keeps one quick action area and opens Loops from both entry points", async () => {
    const navigate = vi.fn();
    view(null, navigate);

    expect(await screen.findAllByLabelText("Quick actions")).toHaveLength(1);
    fireEvent.click(
      screen.getByRole("button", { name: /LoopsBuild reusable workflows/i }),
    );
    expect(navigate).toHaveBeenCalledWith("nodes");

    fireEvent.click(
      screen.getByRole("button", { name: /Build with Loops/i }),
    );
    expect(navigate).toHaveBeenCalledTimes(2);
  });

  it("keeps the Home hero free of character artwork", async () => {
    const rendered = view();
    await screen.findByRole("heading", { name: /Good/ });

    expect(
      rendered.container.querySelector('img[src*="jester"]'),
    ).not.toBeInTheDocument();
    expect(
      rendered.container.querySelector('img[src*="cat-"]'),
    ).not.toBeInTheDocument();
  });

  it("keeps the primary and activity columns on the same desktop baseline", async () => {
    view();

    expect(await screen.findByTestId("home-primary-column")).toHaveClass(
      "flex",
      "h-full",
    );
    expect(screen.getByTestId("home-sidebar")).toHaveClass("grid", "h-full");
    expect(
      screen.getByRole("region", { name: "Recent activity" }),
    ).toHaveClass("h-full");
  });

  it("refreshes activity when the cached Home view becomes active again", async () => {
    let historyLoads = 0;
    invoke.mockImplementation(async (cmd) => {
      if (cmd === "get_stats_summary") {
        return {
          total_macros: 0,
          total_guards: 0,
          total_chains: 0,
          total_schedules: 0,
          total_play_seconds: 0,
          most_played: "",
          most_played_count: 0,
        };
      }
      if (cmd === "get_run_history") {
        historyLoads += 1;
        return historyLoads === 1
          ? [
              {
                name: "Deleted macro",
                timestamp: 1,
                duration: 1,
                status: "completed",
              },
            ]
          : [];
      }
      if (cmd === "anti_afk_list_windows") return [];
      if (cmd === "anti_afk_get") {
        return {
          enabled: false,
          target_id: null,
          interval_min: 15,
          action: "random",
          status: "off",
          error: null,
        };
      }
      return { ok: true };
    });

    const rendered = render(
      <Home status={null} navigate={() => {}} active />,
    );
    expect(await screen.findByText("Deleted macro")).toBeInTheDocument();

    rendered.rerender(
      <Home status={null} navigate={() => {}} active={false} />,
    );
    rendered.rerender(
      <Home status={null} navigate={() => {}} active />,
    );

    expect(await screen.findByText("No activity yet")).toBeInTheDocument();
    expect(screen.queryByText("Deleted macro")).not.toBeInTheDocument();
  });
});
