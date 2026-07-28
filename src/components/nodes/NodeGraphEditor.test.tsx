import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn<(cmd: string, args?: Record<string, unknown>) => Promise<unknown>>();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (command: string, args?: Record<string, unknown>) => invoke(command, args),
}));

import { NodeGraphEditor } from "./NodeGraphEditor";

describe("NodeGraphEditor canvas menu", () => {
  beforeEach(() => {
    localStorage.clear();
    invoke.mockReset();
    invoke.mockImplementation(async (command) => {
      if (command === "node_graph_load") {
        return {
          ok: true,
          source: "saved",
          graph: {
            version: 1,
            name: "demo",
            entry: "start",
            nodes: [
              {
                id: "start",
                type: "start",
                label: "Start",
                enabled: true,
                position: { x: 0, y: 0 },
                config: {},
              },
              {
                id: "stop",
                type: "stop",
                label: "Finish",
                enabled: true,
                position: { x: 300, y: 0 },
                config: { success: true },
              },
            ],
            edges: [{ id: "edge", from: "start", output: "next", to: "stop" }],
          },
        };
      }
      if (command === "node_graph_validate") {
        return { ok: true, errors: [], warnings: [] };
      }
      if (command === "save_template_upload") {
        return {
          ok: true,
          path: "C:\\templates\\enemy.png",
          w: 16,
          h: 16,
          thumb: "dGh1bWI=",
        };
      }
      if (command === "capture_template") {
        return {
          ok: true,
          path: "C:\\templates\\magic.png",
          w: 24,
          h: 24,
          thumb: "bWFnaWM=",
        };
      }
      if (command === "macro_to_steps") {
        return {
          ok: true,
          count: 1,
          steps: [
            {
              id: "click-1",
              type: "click",
              enabled: true,
              label: "Click",
              x: 100,
              y: 200,
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
            },
          ],
        };
      }
      return { ok: true };
    });
  });

  it("opens on canvas right-click and adds the chosen node at that point", async () => {
    const { container } = render(
      <div style={{ width: 1000, height: 700 }}>
        <NodeGraphEditor
          loopName="demo"
          loops={[{ name: "demo", nodes: 2, valid_file: true }]}
          macros={[{ name: "demo", events: 1 }]}
          chains={[{ id: "daily", name: "Daily", macro_names: [] }]}
          status={null}
          onSelectLoop={vi.fn()}
          onCreateLoop={vi.fn()}
          onRenameLoop={vi.fn(async () => true)}
          onDeleteLoop={vi.fn()}
        />
      </div>,
    );

    expect(await screen.findByText("Stop")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Add node" })).not.toBeInTheDocument();

    const pane = container.querySelector(".react-flow__pane");
    expect(pane).not.toBeNull();
    fireEvent.contextMenu(pane!, { clientX: 420, clientY: 260 });

    expect(await screen.findByRole("menu", { name: "Add node" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("menuitem", { name: "Chain" }));

    await waitFor(() => expect(screen.queryByRole("menu", { name: "Add node" })).not.toBeInTheDocument());
    expect(screen.getAllByText("Chain").length).toBeGreaterThan(0);

    fireEvent.contextMenu(pane!, { clientX: 520, clientY: 360 });
    fireEvent.click(await screen.findByRole("menuitem", { name: "Add note" }));
    expect(await screen.findByText("Add context")).toBeInTheDocument();
    fireEvent.change(screen.getByPlaceholderText("Add context for this workflow"), {
      target: { value: "Only runs after reconnecting" },
    });
    expect(await screen.findByText("Only runs after reconnecting")).toBeInTheDocument();
  });

  it("shows the complete node palette directly without a nested menu", async () => {
    const { container } = render(
      <div style={{ width: 1000, height: 700 }}>
        <NodeGraphEditor
          loopName="demo"
          loops={[{ name: "demo", nodes: 2, valid_file: true }]}
          macros={[{ name: "demo", events: 1 }]}
          chains={[]}
          status={null}
          onSelectLoop={vi.fn()}
          onCreateLoop={vi.fn()}
          onRenameLoop={vi.fn(async () => true)}
          onDeleteLoop={vi.fn()}
        />
      </div>,
    );

    expect(await screen.findByText("Stop")).toBeInTheDocument();
    const pane = container.querySelector(".react-flow__pane");
    fireEvent.contextMenu(pane!, { clientX: 420, clientY: 260 });

    expect(screen.queryByRole("menuitem", { name: "More nodes" })).not.toBeInTheDocument();
    for (const label of [
      "Type",
      "Scroll",
      "Find & click",
      "Wait for image",
      "Repeat",
      "Add note",
      "New Loop",
    ]) {
      expect(screen.getByRole("menuitem", { name: label })).toBeVisible();
    }
  });

  it("keeps multi-path ports stable and labels both branch outcomes", async () => {
    const { container } = render(
      <div style={{ width: 1000, height: 700 }}>
        <NodeGraphEditor
          loopName="demo"
          loops={[{ name: "demo", nodes: 2, valid_file: true }]}
          macros={[{ name: "demo", events: 1 }]}
          chains={[]}
          status={null}
          onSelectLoop={vi.fn()}
          onCreateLoop={vi.fn()}
          onRenameLoop={vi.fn(async () => true)}
          onDeleteLoop={vi.fn()}
        />
      </div>,
    );

    expect(await screen.findByText("Stop")).toBeInTheDocument();
    const pane = container.querySelector(".react-flow__pane");

    fireEvent.contextMenu(pane!, { clientX: 420, clientY: 260 });
    fireEvent.click(await screen.findByRole("menuitem", { name: "Click" }));
    const clickCard = (await screen.findByText("Click")).closest(".node-card");
    expect(clickCard).not.toBeNull();
    expect(clickCard).not.toHaveClass("node-card--collapsible-outputs");
    expect(within(clickCard as HTMLElement).getByText("If works")).toBeInTheDocument();
    expect(within(clickCard as HTMLElement).getByText("If fails")).toBeInTheDocument();
    expect(within(clickCard as HTMLElement).getByLabelText("If works")).toBeInTheDocument();
    expect(within(clickCard as HTMLElement).getByLabelText("If fails")).toBeInTheDocument();

    fireEvent.contextMenu(pane!, { clientX: 560, clientY: 360 });
    fireEvent.click(await screen.findByRole("menuitem", { name: "Branch" }));
    const branchCard = (await screen.findByText("Branch")).closest(".node-card");
    expect(branchCard).not.toBeNull();
    expect(within(branchCard as HTMLElement).getByText("If works")).toBeInTheDocument();
    expect(within(branchCard as HTMLElement).getByText("If fails")).toBeInTheDocument();
    expect(within(branchCard as HTMLElement).getByLabelText("If works")).toBeInTheDocument();
    expect(within(branchCard as HTMLElement).getByLabelText("If fails")).toBeInTheDocument();

    expect(screen.queryByRole("button", { name: "Check" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Arrange graph" })).toBeInTheDocument();
  });

  it("imports a dropped image into a wait guard", async () => {
    const { container } = render(
      <div style={{ width: 1000, height: 700 }}>
        <NodeGraphEditor
          loopName="demo"
          loops={[{ name: "demo", nodes: 2, valid_file: true }]}
          macros={[{ name: "demo", events: 1 }]}
          chains={[]}
          status={null}
          onSelectLoop={vi.fn()}
          onCreateLoop={vi.fn()}
          onRenameLoop={vi.fn(async () => true)}
          onDeleteLoop={vi.fn()}
        />
      </div>,
    );

    expect(await screen.findByText("Stop")).toBeInTheDocument();
    const pane = container.querySelector(".react-flow__pane");
    fireEvent.contextMenu(pane!, { clientX: 420, clientY: 260 });
    fireEvent.click(await screen.findByRole("menuitem", { name: "Wait for image" }));

    expect(await screen.findByText("Drag and drop an image")).toBeInTheDocument();
    expect(screen.getByText("or click to choose one")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Magic select from screen" }));
    expect(await screen.findByText("magic.png")).toBeInTheDocument();
    expect(invoke).toHaveBeenCalledWith("capture_template", undefined);
    expect(await screen.findByAltText("Wait for image template")).toHaveAttribute(
      "src",
      "data:image/png;base64,bWFnaWM=",
    );
    expect(screen.getByRole("combobox", { name: "Watch for" }).tagName).toBe("BUTTON");

    fireEvent.click(screen.getByRole("button", { name: "Remove image" }));
    expect(await screen.findByText("Drag and drop an image")).toBeInTheDocument();
    expect(screen.queryByAltText("Wait for image template")).not.toBeInTheDocument();

    const dropzone = screen.getByRole("button", { name: "Image template" });
    const image = new File(["image-bytes"], "enemy.png", { type: "image/png" });
    fireEvent.drop(dropzone, { dataTransfer: { files: [image] } });

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith(
        "save_template_upload",
        expect.objectContaining({ dataBase64: expect.any(String) }),
      ),
    );
    expect(await screen.findByText("enemy.png")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Remove image" })).toBeInTheDocument();
    expect(screen.getByAltText("Wait for image template")).toHaveAttribute(
      "src",
      "data:image/png;base64,dGh1bWI=",
    );

    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith(
        "node_graph_save",
        expect.objectContaining({
          graph: expect.objectContaining({
            nodes: expect.arrayContaining([
              expect.objectContaining({
                config: expect.objectContaining({
                  template_thumb: "data:image/png;base64,dGh1bWI=",
                }),
              }),
            ]),
          }),
        }),
      ),
    );
  });

  it("imports a macro snapshot into a standalone node", async () => {
    const { container } = render(
      <div style={{ width: 1000, height: 700 }}>
        <NodeGraphEditor
          loopName="demo"
          loops={[{ name: "demo", nodes: 2, valid_file: true }]}
          macros={[
            { name: "demo", events: 1 },
            { name: "Farm", events: 42, duration: 9.5, resolution: "1920x1080" },
          ]}
          chains={[]}
          status={null}
          onSelectLoop={vi.fn()}
          onCreateLoop={vi.fn()}
          onRenameLoop={vi.fn(async () => true)}
          onDeleteLoop={vi.fn()}
        />
      </div>,
    );

    expect(await screen.findByText("Stop")).toBeInTheDocument();
    const pane = container.querySelector(".react-flow__pane");
    fireEvent.contextMenu(pane!, { clientX: 420, clientY: 260 });
    fireEvent.click(await screen.findByRole("menuitem", { name: "Macro" }));

    fireEvent.click(await screen.findByRole("combobox", { name: "Macro to import" }));
    fireEvent.click(await screen.findByRole("option", { name: "Farm" }));

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("macro_to_steps", { macroName: "Farm" }),
    );
    expect(await screen.findByText(/1 action embedded/)).toBeInTheDocument();
    expect(screen.getByText("Independent copy")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Re-import latest" })).toBeInTheDocument();
    expect(screen.getByLabelText("Repeat count")).toHaveValue(1);
  });

  it("creates and edits a complete macro chain inside a Loop node", async () => {
    const { container } = render(
      <div style={{ width: 1000, height: 700 }}>
        <NodeGraphEditor
          loopName="demo"
          loops={[{ name: "demo", nodes: 2, valid_file: true }]}
          macros={[
            { name: "Farm", events: 20, duration: 10 },
            { name: "Raid", events: 40, duration: 20 },
          ]}
          chains={[
            {
              id: "daily",
              name: "Daily",
              macro_names: ["Farm"],
              delay_between: 1,
              repeat: 1,
            },
          ]}
          status={null}
          onSelectLoop={vi.fn()}
          onCreateLoop={vi.fn()}
          onRenameLoop={vi.fn(async () => true)}
          onDeleteLoop={vi.fn()}
        />
      </div>,
    );

    expect(await screen.findByText("Stop")).toBeInTheDocument();
    const pane = container.querySelector(".react-flow__pane");
    fireEvent.contextMenu(pane!, { clientX: 420, clientY: 260 });
    fireEvent.click(await screen.findByRole("menuitem", { name: "Chain" }));

    const floatingInspector = await screen.findByRole("complementary", {
      name: "Node inspector",
    });
    expect(container.querySelector(".node-canvas")).toContainElement(
      floatingInspector,
    );
    expect(screen.queryByLabelText("Loop library")).not.toBeInTheDocument();

    fireEvent.click(await screen.findByRole("combobox", { name: "Chain" }));
    fireEvent.click(await screen.findByRole("option", { name: "Daily" }));

    expect(
      await screen.findByRole("region", { name: "Chain sequence" }),
    ).toHaveTextContent("Farm");
    fireEvent.click(
      screen.getByRole("combobox", { name: "Add macro to chain" }),
    );
    fireEvent.click(await screen.findByRole("option", { name: "Raid" }));
    expect(
      screen.getByRole("region", { name: "Chain sequence" }),
    ).toHaveTextContent("Raid");

    fireEvent.click(screen.getByRole("button", { name: "Save chain" }));
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("update_chain", {
        chainId: "daily",
        name: "Daily",
        macroNames: ["Farm", "Raid"],
        delayBetween: 1,
        repeat: 1,
      }),
    );
  });

  it("can rebuild the entry node and undo or redo canvas edits", async () => {
    const { container } = render(
      <div style={{ width: 1000, height: 700 }}>
        <NodeGraphEditor
          loopName="demo"
          loops={[{ name: "demo", nodes: 2, valid_file: true }]}
          macros={[{ name: "demo", events: 1 }]}
          chains={[]}
          status={null}
          onSelectLoop={vi.fn()}
          onCreateLoop={vi.fn()}
          onRenameLoop={vi.fn(async () => true)}
          onDeleteLoop={vi.fn()}
        />
      </div>,
    );

    expect(await screen.findByText("Start")).toBeInTheDocument();
    fireEvent.click(screen.getByText("Start"));
    fireEvent.click(screen.getByRole("button", { name: "Delete node" }));
    expect(screen.queryByText("Start")).not.toBeInTheDocument();

    const pane = container.querySelector(".react-flow__pane");
    fireEvent.contextMenu(pane!, { clientX: 300, clientY: 240 });
    fireEvent.click(await screen.findByRole("menuitem", { name: "Start" }));
    expect(await screen.findByText("Start")).toBeInTheDocument();

    fireEvent.contextMenu(pane!, { clientX: 460, clientY: 260 });
    fireEvent.click(await screen.findByRole("menuitem", { name: "Chain" }));
    expect((await screen.findAllByText("Chain")).length).toBeGreaterThan(0);

    fireEvent.keyDown(window, { key: "z", ctrlKey: true });
    await waitFor(() => expect(screen.queryAllByText("Chain")).toHaveLength(0));
    fireEvent.keyDown(window, { key: "z", ctrlKey: true, shiftKey: true });
    expect((await screen.findAllByText("Chain")).length).toBeGreaterThan(0);
  });

  it("creates a Loop and supports direct or contextual inline renaming", async () => {
    const onCreateLoop = vi.fn();
    const onRenameLoop = vi.fn(async () => true);
    const onDeleteLoop = vi.fn();
    const { container } = render(
      <div style={{ width: 1000, height: 700 }}>
        <NodeGraphEditor
          loopName="demo"
          loops={[{ name: "demo", nodes: 2, valid_file: true }]}
          macros={[{ name: "Farm", events: 20 }]}
          chains={[]}
          status={null}
          onSelectLoop={vi.fn()}
          onCreateLoop={onCreateLoop}
          onRenameLoop={onRenameLoop}
          onDeleteLoop={onDeleteLoop}
        />
      </div>,
    );

    expect(await screen.findByText("Stop")).toBeInTheDocument();
    const pane = container.querySelector(".react-flow__pane");
    fireEvent.contextMenu(pane!, { clientX: 420, clientY: 260 });
    fireEvent.click(await screen.findByRole("menuitem", { name: "New Loop" }));
    expect(onCreateLoop).toHaveBeenCalledOnce();
    expect(
      screen.queryByRole("button", { name: "More actions for demo" }),
    ).not.toBeInTheDocument();

    const loopControls = screen.getByRole("button", { name: "Loop controls" });
    expect(loopControls.closest("[data-loop-picker]")).toContainElement(
      screen.getByRole("combobox", { name: "Current Loop" }),
    );
    fireEvent.click(loopControls);
    expect(await screen.findByRole("menu", { name: "Loop actions" })).toBeInTheDocument();
    expect(screen.getByRole("menuitem", { name: "Rename" })).toBeInTheDocument();
    expect(screen.getByRole("menuitem", { name: "Delete" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("menuitem", { name: "Rename" }));
    expect(screen.getByRole("textbox", { name: "Rename demo" })).toBeInTheDocument();
    fireEvent.keyDown(screen.getByRole("textbox", { name: "Rename demo" }), {
      key: "Escape",
    });

    const loop = screen.getByRole("combobox", { name: "Current Loop" });
    fireEvent.doubleClick(loop);
    const directName = screen.getByRole("textbox", { name: "Rename demo" });
    fireEvent.keyDown(directName, { key: "Escape" });
    expect(screen.queryByRole("textbox", { name: "Rename demo" })).not.toBeInTheDocument();

    fireEvent.keyDown(window, { key: "F2" });
    expect(screen.getByRole("textbox", { name: "Rename demo" })).toBeInTheDocument();
    fireEvent.keyDown(screen.getByRole("textbox", { name: "Rename demo" }), { key: "Escape" });

    fireEvent.contextMenu(screen.getByRole("combobox", { name: "Current Loop" }), {
      clientX: 760,
      clientY: 240,
    });
    fireEvent.click(await screen.findByRole("menuitem", { name: "Rename" }));
    const name = screen.getByRole("textbox", { name: "Rename demo" });
    fireEvent.change(name, { target: { value: "Daily Farm" } });
    fireEvent.keyDown(name, { key: "Enter" });
    await waitFor(() => expect(onRenameLoop).toHaveBeenCalledWith("demo", "Daily Farm"));
    await waitFor(() =>
      expect(screen.getByRole("combobox", { name: "Current Loop" })).toBeInTheDocument(),
    );

    fireEvent.contextMenu(screen.getByRole("combobox", { name: "Current Loop" }), {
      clientX: 760,
      clientY: 240,
    });
    fireEvent.click(await screen.findByRole("menuitem", { name: "Delete" }));
    expect(onDeleteLoop).toHaveBeenCalledWith("demo");
  });

  it("keeps Loop selection in the toolbar and floats contextual editing over the canvas", async () => {
    const onSelectLoop = vi.fn();
    const { container } = render(
      <div style={{ width: 1200, height: 760 }}>
        <NodeGraphEditor
          loopName="demo"
          loops={[
            { name: "demo", nodes: 2, valid_file: true },
            { name: "Daily Farm", nodes: 8, valid_file: true },
            { name: "Broken Guard", nodes: 4, valid_file: false },
          ]}
          macros={[{ name: "Farm", events: 20 }]}
          chains={[]}
          status={null}
          onSelectLoop={onSelectLoop}
          onCreateLoop={vi.fn()}
          onRenameLoop={vi.fn(async () => true)}
          onDeleteLoop={vi.fn()}
        />
      </div>,
    );

    expect(await screen.findByText("Start")).toBeInTheDocument();
    expect(screen.queryByLabelText("Loop library")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("Overview for demo")).not.toBeInTheDocument();

    const currentLoop = screen.getByRole("combobox", { name: "Current Loop" });
    fireEvent.click(currentLoop);
    fireEvent.click(await screen.findByRole("option", { name: "Daily Farm" }));
    expect(onSelectLoop).toHaveBeenCalledWith("Daily Farm");

    fireEvent.click(screen.getByText("Start"));
    const inspector = await screen.findByRole("complementary", {
      name: "Node inspector",
    });
    expect(container.querySelector(".node-canvas")).toContainElement(inspector);
    expect(inspector).toHaveTextContent("Inspector");
    expect(inspector).toHaveTextContent("start");
    expect(inspector).toHaveClass(
      "node-floating-inspector",
    );
  });
});
