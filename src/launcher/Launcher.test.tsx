import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn<(command: string, args?: unknown) => Promise<unknown>>();
const hide = vi.fn<() => Promise<void>>();
const onFocusChanged = vi.fn(() => Promise.resolve(() => {}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (command: string, args?: unknown) => invoke(command, args),
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    hide,
    onFocusChanged,
  }),
}));

import { Launcher } from "./Launcher";

const graph = {
  version: 1,
  name: "Daily rewards",
  entry: "start",
  nodes: [
    {
      id: "start",
      type: "start",
      label: "Start",
      position: { x: 0, y: 0 },
      enabled: true,
      config: {},
    },
  ],
  edges: [],
};

describe("F4 launcher", () => {
  beforeEach(() => {
    invoke.mockReset();
    hide.mockReset();
    onFocusChanged.mockClear();
    invoke.mockImplementation(async (command) => {
      switch (command) {
        case "list_macros":
          return [];
        case "list_chains":
          return [
            {
              id: "legacy-chain",
              name: "Legacy chain",
              macro_names: ["A"],
              repeat: 1,
            },
          ];
        case "node_graph_list":
          return [
            {
              name: "Daily rewards",
              nodes: 1,
              valid_file: true,
              updated_at: 123,
            },
          ];
        case "node_graph_load":
          return { ok: true, graph, source: "saved" };
        case "node_graph_run":
          return { ok: true };
        case "vision_load":
          return { ok: true, triggers: [] };
        default:
          return { ok: true };
      }
    });
  });

  it("shows Loops and excludes legacy Chains", async () => {
    render(<Launcher />);

    expect(await screen.findByText("Daily rewards")).toBeInTheDocument();
    expect(screen.getByText("loop")).toBeInTheDocument();
    expect(screen.queryByText("Legacy chain")).not.toBeInTheDocument();
    expect(invoke).not.toHaveBeenCalledWith("list_chains");
    expect(
      screen.getByPlaceholderText("Search macros, loops and watch…"),
    ).toBeInTheDocument();
  });

  it("loads and runs a selected Loop", async () => {
    render(<Launcher />);

    fireEvent.click(await screen.findByText("Daily rewards"));

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("node_graph_load", {
        loopName: "Daily rewards",
      });
      expect(invoke).toHaveBeenCalledWith("node_graph_run", { graph });
      expect(hide).toHaveBeenCalled();
    });
  });
});
