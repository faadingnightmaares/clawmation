import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn<(cmd: string, args?: Record<string, unknown>) => Promise<unknown>>();
let captureCalls = 0;
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (command: string, args?: Record<string, unknown>) => invoke(command, args),
}));

import { NodeGraphEditor } from "./NodeGraphEditor";

describe("NodeGraphEditor canvas menu", () => {
  beforeEach(() => {
    localStorage.clear();
    captureCalls = 0;
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
        captureCalls += 1;
        return {
          ok: true,
          path:
            captureCalls === 1
              ? "C:\\templates\\magic.png"
              : "C:\\templates\\magic-hovered.png",
          w: 24,
          h: 24,
          thumb: captureCalls === 1 ? "bWFnaWM=" : "aG92ZXJlZA==",
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

  it("repairs a Forever Repeat automatically and only shows a count for Custom", async () => {
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
                id: "repeat",
                type: "loop",
                label: "Repeat",
                enabled: true,
                position: { x: 220, y: 0 },
                config: { count: 0 },
              },
              {
                id: "vision",
                type: "vision",
                label: "Wait for image",
                enabled: true,
                position: { x: 440, y: 0 },
                config: {
                  step: {
                    id: "wait",
                    type: "wait_for",
                    enabled: true,
                    detect_mode: "template",
                    template: "",
                    templates: [],
                  },
                },
              },
            ],
            edges: [
              { id: "enter", from: "start", output: "next", to: "repeat" },
              { id: "body", from: "repeat", output: "body", to: "vision" },
            ],
          },
        };
      }
      if (command === "node_graph_validate") {
        return { ok: true, errors: [], warnings: [] };
      }
      return { ok: true };
    });

    render(
      <div style={{ width: 1000, height: 700 }}>
        <NodeGraphEditor
          loopName="demo"
          loops={[{ name: "demo", nodes: 3, valid_file: true }]}
          macros={[]}
          chains={[]}
          status={null}
          onSelectLoop={vi.fn()}
          onCreateLoop={vi.fn()}
          onRenameLoop={vi.fn(async () => true)}
          onDeleteLoop={vi.fn()}
        />
      </div>,
    );

    fireEvent.click(await screen.findByText("Repeat"));
    expect(screen.getByRole("button", { name: "Forever" })).toBeInTheDocument();
    expect(screen.queryByLabelText(/Exact count/i)).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Custom" }));
    expect(screen.getByLabelText("Count")).toHaveValue(1);

    fireEvent.click(screen.getByRole("button", { name: "Forever" }));
    fireEvent.click(screen.getByRole("button", { name: "Run" }));
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith(
        "node_graph_run",
        expect.objectContaining({
          graph: expect.objectContaining({
            edges: expect.arrayContaining([
              expect.objectContaining({
                from: "vision",
                output: "found",
                to: "repeat",
              }),
            ]),
          }),
        }),
      );
    });
  });

  it("shows contextual paths and keeps recovery ports hidden until enabled", async () => {
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
    expect(within(clickCard as HTMLElement).getByLabelText("Continue")).toBeInTheDocument();
    expect(within(clickCard as HTMLElement).queryByLabelText("On failure")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("combobox", { name: "On failure" }));
    fireEvent.click(await screen.findByRole("option", { name: "Recovery path" }));
    expect(within(clickCard as HTMLElement).getByLabelText("On failure")).toBeInTheDocument();

    fireEvent.contextMenu(pane!, { clientX: 560, clientY: 360 });
    fireEvent.click(await screen.findByRole("menuitem", { name: "Branch" }));
    const branchCard = (await screen.findByText("Branch")).closest(".node-card");
    expect(branchCard).not.toBeNull();
    expect(within(branchCard as HTMLElement).getByText("Matches")).toBeInTheDocument();
    expect(within(branchCard as HTMLElement).getByText("Otherwise")).toBeInTheDocument();
    expect(within(branchCard as HTMLElement).getByLabelText("Matches")).toBeInTheDocument();
    expect(within(branchCard as HTMLElement).getByLabelText("Otherwise")).toBeInTheDocument();

    expect(screen.queryByRole("button", { name: "Check" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Arrange graph" })).toBeInTheDocument();
  });

  it("uses the compact menu when right-clicking a node and continues from it", async () => {
    render(
      <div style={{ width: 1000, height: 700 }}>
        <NodeGraphEditor
          loopName="demo"
          loops={[{ name: "demo", nodes: 2, valid_file: true }]}
          macros={[]}
          chains={[]}
          status={null}
          onSelectLoop={vi.fn()}
          onCreateLoop={vi.fn()}
          onRenameLoop={vi.fn(async () => true)}
          onDeleteLoop={vi.fn()}
        />
      </div>,
    );

    const startCard = (await screen.findByText("Start")).closest(".node-card");
    fireEvent.contextMenu(startCard!, { clientX: 260, clientY: 180 });

    const menu = await screen.findByRole("menu", { name: "Add node" });
    expect(
      within(menu).queryByRole("menuitem", { name: "Start" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("dialog", { name: "Add connected node" }),
    ).not.toBeInTheDocument();
    fireEvent.click(within(menu).getByRole("menuitem", { name: "Find & click" }));
    expect(await screen.findByText("Find image & click")).toBeInTheDocument();
  });

  it("opens Loop templates from the compact canvas menu", async () => {
    const onCreateLoop = vi.fn();
    const { container } = render(
      <div style={{ width: 1000, height: 700 }}>
        <NodeGraphEditor
          loopName="demo"
          loops={[{ name: "demo", nodes: 2, valid_file: true }]}
          macros={[]}
          chains={[]}
          status={null}
          onSelectLoop={vi.fn()}
          onCreateLoop={onCreateLoop}
          onRenameLoop={vi.fn(async () => true)}
          onDeleteLoop={vi.fn()}
        />
      </div>,
    );

    expect(await screen.findByText("Stop")).toBeInTheDocument();
    fireEvent.contextMenu(container.querySelector(".react-flow__pane")!, {
      clientX: 420,
      clientY: 260,
    });
    fireEvent.click(
      await screen.findByRole("menuitem", { name: "Templates" }),
    );
    const templates = await screen.findByRole("menu", {
      name: "Loop templates",
    });
    fireEvent.click(
      within(templates).getByRole("menuitem", { name: /Learn Loops/ }),
    );

    expect(onCreateLoop).toHaveBeenCalledWith("learn-loops");
  });

  it("offers direct screen targeting for Click nodes", async () => {
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
              { id: "start", type: "start", label: "Start", enabled: true, position: { x: 0, y: 0 }, config: {} },
              {
                id: "click",
                type: "action",
                label: "Click",
                enabled: true,
                position: { x: 220, y: 0 },
                config: {
                  step: {
                    id: "click-1", type: "click", enabled: true, label: "Click",
                    x: 0, y: 0, key: "", text: "", delay: 0, scroll_amount: 0,
                    detect_mode: "color", hsv_low: [0, 0, 0], hsv_high: [179, 255, 255],
                    template: "", templates: [], region: [0, 0, 100, 100],
                    min_area: 40, timeout: 10, confidence: 0.8,
                  },
                },
              },
            ],
            edges: [{ id: "edge", from: "start", output: "next", to: "click" }],
          },
        };
      }
      if (command === "pick_screen_point") {
        return { ok: true, x: -420, y: 780, monitor: "Display 2" };
      }
      return { ok: true, errors: [], warnings: [] };
    });

    render(
      <div style={{ width: 1000, height: 700 }}>
        <NodeGraphEditor
          loopName="demo"
          loops={[{ name: "demo", nodes: 2, valid_file: true }]}
          macros={[]}
          chains={[]}
          status={null}
          onSelectLoop={vi.fn()}
          onCreateLoop={vi.fn()}
          onRenameLoop={vi.fn(async () => true)}
          onDeleteLoop={vi.fn()}
        />
      </div>,
    );

    fireEvent.click(await screen.findByText("Click"));
    fireEvent.click(screen.getByRole("button", { name: "Pick on screen" }));
    await waitFor(() => {
      expect(screen.getByLabelText("X")).toHaveValue(-420);
      expect(screen.getByLabelText("Y")).toHaveValue(780);
    });
    expect(screen.getByText("Display 2")).toBeInTheDocument();
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

    fireEvent.click(screen.getByRole("button", { name: "Magic select from screen" }));
    expect(await screen.findByText("magic-hovered.png")).toBeInTheDocument();
    fireEvent.click(
      screen.getByRole("button", { name: "Remove magic-hovered.png" }),
    );
    expect(screen.getByText("magic.png")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Remove magic.png" }));
    expect(await screen.findByText("Drag and drop an image")).toBeInTheDocument();
    expect(screen.queryByAltText("Wait for image template")).not.toBeInTheDocument();

    const dropzone = screen.getByRole("button", { name: "Choose image" });
    const image = new File(["image-bytes"], "enemy.png", { type: "image/png" });
    fireEvent.drop(dropzone, { dataTransfer: { files: [image] } });

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith(
        "save_template_upload",
        expect.objectContaining({ dataBase64: expect.any(String) }),
      ),
    );
    expect(await screen.findByText("enemy.png")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Remove enemy.png" })).toBeInTheDocument();
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

    expect(
      screen.getByRole("combobox", { name: "Current Loop" }).closest("[data-loop-picker]"),
    ).toContainElement(
      screen.getByRole("combobox", { name: "Current Loop" }),
    );
    expect(screen.getByRole("button", { name: "Import" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Export" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "More" }));
    expect(await screen.findByRole("menu", { name: "Loop actions" })).toBeInTheDocument();
    expect(screen.queryByRole("menuitem", { name: "Import Loop" })).not.toBeInTheDocument();
    expect(screen.queryByRole("menuitem", { name: "Export Loop" })).not.toBeInTheDocument();
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

  it("saves unsaved changes before exporting and delegates Loop imports", async () => {
    const onImportLoop = vi.fn(async () => {});
    render(
      <div style={{ width: 1000, height: 700 }}>
        <NodeGraphEditor
          loopName="demo"
          loops={[{ name: "demo", nodes: 2, valid_file: true }]}
          macros={[]}
          chains={[]}
          status={null}
          onSelectLoop={vi.fn()}
          onCreateLoop={vi.fn()}
          onImportLoop={onImportLoop}
          onRenameLoop={vi.fn(async () => true)}
          onDeleteLoop={vi.fn()}
        />
      </div>,
    );

    fireEvent.click(await screen.findByText("Start"));
    fireEvent.change(screen.getByRole("textbox", { name: "Label" }), {
      target: { value: "Begin" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Export" }));
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("export_loop", { loopName: "demo" }),
    );
    const commands = invoke.mock.calls.map(([command]) => command);
    expect(commands.indexOf("node_graph_save")).toBeLessThan(
      commands.indexOf("export_loop"),
    );

    fireEvent.click(screen.getByRole("button", { name: "Import" }));
    expect(onImportLoop).toHaveBeenCalledTimes(1);
  });
});
