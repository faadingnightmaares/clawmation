import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn<(command: string, args?: Record<string, unknown>) => Promise<unknown>>();
let savedLoops: Array<{ name: string; nodes: number; valid_file: boolean }> = [];

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (command: string, args?: Record<string, unknown>) => invoke(command, args),
}));

vi.mock("@/components/nodes/NodeGraphEditor", () => ({
  NodeGraphEditor: ({ loopName }: { loopName: string }) => (
    <div data-testid="loop-editor">Editing {loopName}</div>
  ),
}));

import { Nodes } from "./Nodes";

describe("Nodes Loop workspaces", () => {
  beforeEach(() => {
    savedLoops = [];
    invoke.mockReset();
    invoke.mockImplementation(async (command) => {
      if (command === "node_graph_list") return savedLoops;
      if (command === "list_macros") {
        return [{ name: "Recorded Farm", events: 42, duration: 10, resolution: "1920x1080" }];
      }
      if (command === "list_chains") return [];
      if (command === "node_graph_create") {
        savedLoops = [{ name: "Loop", nodes: 2, valid_file: true }];
        return { ok: true, name: "Loop" };
      }
      return { ok: true };
    });
  });

  it("does not render recorded macros as a Nodes sidebar", async () => {
    render(<Nodes status={null} navigate={vi.fn()} />);

    expect(await screen.findByText("Create your first Loop")).toBeInTheDocument();
    expect(screen.queryByPlaceholderText("Find a macro")).not.toBeInTheDocument();
    expect(screen.queryByText("Recorded Farm")).not.toBeInTheDocument();
  });

  it("creates a Loop from the empty canvas right-click menu", async () => {
    const { container } = render(<Nodes status={null} navigate={vi.fn()} />);
    expect(await screen.findByText("Create your first Loop")).toBeInTheDocument();

    fireEvent.contextMenu(container.querySelector(".node-empty-canvas")!, {
      clientX: 320,
      clientY: 240,
    });
    fireEvent.click(await screen.findByRole("menuitem", { name: "New Loop" }));

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("node_graph_create", { name: "Loop" }),
    );
    expect(await screen.findByTestId("loop-editor")).toHaveTextContent("Editing Loop");
  });
});
