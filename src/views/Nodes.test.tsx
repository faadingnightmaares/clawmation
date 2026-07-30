import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn<(command: string, args?: Record<string, unknown>) => Promise<unknown>>();
let savedLoops: Array<{ name: string; nodes: number; valid_file: boolean }> = [];

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (command: string, args?: Record<string, unknown>) => invoke(command, args),
}));

vi.mock("@/components/nodes/NodeGraphEditor", () => ({
  NodeGraphEditor: ({
    loopName,
    onCreateLoop,
  }: {
    loopName: string;
    onCreateLoop: (templateId?: string) => void;
  }) => (
    <div data-testid="loop-editor">
      Editing {loopName}
      <button type="button" onClick={() => onCreateLoop("learn-loops")}>
        Create tutorial
      </button>
    </div>
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
      if (command === "import_loop") {
        savedLoops = [{ name: "Imported Loop", nodes: 4, valid_file: true }];
        return { ok: true, name: "Imported Loop" };
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

  it("imports a portable Loop from the empty workspace and selects it", async () => {
    render(<Nodes status={null} navigate={vi.fn()} />);
    expect(await screen.findByText("Create your first Loop")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Import" }));

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("import_loop", undefined),
    );
    expect(await screen.findByTestId("loop-editor")).toHaveTextContent(
      "Editing Imported Loop",
    );
  });

  it("creates, saves, and selects a Loop template", async () => {
    savedLoops = [{ name: "Existing", nodes: 2, valid_file: true }];
    invoke.mockImplementation(async (command, args) => {
      if (command === "node_graph_list") return savedLoops;
      if (command === "list_macros" || command === "list_chains") return [];
      if (command === "node_graph_load") {
        return {
          ok: true,
          source: "saved",
          graph: {
            version: 1,
            name: "Existing",
            entry: "start",
            nodes: [],
            edges: [],
          },
        };
      }
      if (command === "node_graph_create") {
        expect(args).toEqual({ name: "Learn Loops" });
        savedLoops = [
          ...savedLoops,
          { name: "Learn Loops", nodes: 7, valid_file: true },
        ];
        return { ok: true, name: "Learn Loops" };
      }
      return { ok: true };
    });

    render(<Nodes status={null} navigate={vi.fn()} />);
    fireEvent.click(await screen.findByRole("button", { name: "Create tutorial" }));

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith(
        "node_graph_save",
        expect.objectContaining({
          loopName: "Learn Loops",
          graph: expect.objectContaining({
            name: "Learn Loops",
            entry: "start",
          }),
        }),
      ),
    );
    expect(await screen.findByTestId("loop-editor")).toHaveTextContent(
      "Editing Learn Loops",
    );
  });

  it("removes a reserved Loop when its template cannot be saved", async () => {
    savedLoops = [{ name: "Existing", nodes: 2, valid_file: true }];
    invoke.mockImplementation(async (command) => {
      if (command === "node_graph_list") return savedLoops;
      if (command === "list_macros" || command === "list_chains") return [];
      if (command === "node_graph_create") {
        return { ok: true, name: "Learn Loops" };
      }
      if (command === "node_graph_save") {
        return { ok: false, error: "save failed" };
      }
      if (command === "node_graph_delete") return { ok: true };
      return { ok: true };
    });

    render(<Nodes status={null} navigate={vi.fn()} />);
    fireEvent.click(await screen.findByRole("button", { name: "Create tutorial" }));

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("node_graph_delete", {
        name: "Learn Loops",
      }),
    );
  });
});
