import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn<(command: string, args?: unknown) => Promise<unknown>>();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (command: string, args?: unknown) => invoke(command, args),
}));

import { Macros } from "./Macros";

const raid = {
  name: "raid",
  events: 4573,
  duration: 371,
  resolution: "1920x1080",
  loop: true,
  loop_count: 1,
  category: "Combat",
  notes: "Full raid routine",
  play_count: 12,
  last_played: 1_700_000_000,
  played: 4452,
};

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

function mockBackend() {
  invoke.mockImplementation(async (command) => {
    switch (command) {
      case "list_macros":
        return [
          raid,
          {
            ...raid,
            name: "farming loop",
            events: 2913,
            category: "Farming",
            notes: "Daily farming route",
          },
        ];
      case "list_templates":
        return [];
      case "get_all_guard_counts":
        return { ok: true, counts: { raid: 2 } };
      case "play_macro":
        return { ok: true };
      case "set_repeat":
        return { ok: true };
      default:
        return { ok: true };
    }
  });
}

function view(status: Status | null = null) {
  return render(<Macros status={status} navigate={() => {}} />);
}

describe("Macros workspace", () => {
  beforeEach(() => {
    invoke.mockReset();
    localStorage.clear();
    mockBackend();
  });

  it("loads macros and filters them through search", async () => {
    view();

    expect(await screen.findAllByText("raid")).not.toHaveLength(0);
    expect(screen.getAllByText("farming loop")).not.toHaveLength(0);

    fireEvent.change(screen.getByPlaceholderText(/Search macros/), {
      target: { value: "farming" },
    });

    expect(screen.queryByText("raid")).not.toBeInTheDocument();
    expect(screen.getAllByText("farming loop")).not.toHaveLength(0);
  });

  it("keeps the final two-column structure while the macro library loads", async () => {
    const pendingMacros = deferred<typeof raid[]>();
    invoke.mockImplementation(async (command) => {
      if (command === "list_macros") return pendingMacros.promise;
      if (command === "list_templates") return [];
      if (command === "get_all_guard_counts") {
        return { ok: true, counts: {} };
      }
      return { ok: true };
    });

    view();

    expect(
      screen.getByRole("status", { name: "Loading macro workspace" }),
    ).toBeInTheDocument();
    expect(screen.queryByText("No macros yet")).not.toBeInTheDocument();

    pendingMacros.resolve([raid]);

    expect(
      await screen.findByRole("complementary", { name: "Edit raid" }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("status", { name: "Loading macro workspace" }),
    ).not.toBeInTheDocument();
  });

  it("shows a retry state instead of a false empty library when loading fails", async () => {
    let shouldFail = true;
    invoke.mockImplementation(async (command) => {
      if (command === "list_macros") {
        if (shouldFail) throw new Error("local library unavailable");
        return [raid];
      }
      if (command === "list_templates") return [];
      if (command === "get_all_guard_counts") {
        return { ok: true, counts: {} };
      }
      return { ok: true };
    });

    view();

    expect(
      await screen.findByRole("alert"),
    ).toHaveTextContent("Couldn't load macros");
    expect(screen.queryByText("No macros yet")).not.toBeInTheDocument();

    shouldFail = false;
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));

    expect(
      await screen.findByRole("complementary", { name: "Edit raid" }),
    ).toBeInTheDocument();
  });

  it("runs a macro with its persisted repeat setting", async () => {
    view();

    const runButtons = await screen.findAllByRole("button", { name: /^Run/ });
    fireEvent.click(runButtons[0]);

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("play_macro", {
        name: "raid",
        repeat: 1,
        speed: 1,
      }),
    );
  });

  it("keeps the selected macro editor visible without an extra click", async () => {
    view();

    expect(
      await screen.findByRole("complementary", { name: "Edit raid" }),
    ).toBeInTheDocument();
    expect(
      await screen.findByRole("region", { name: "Screen safeguards for raid" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("tab", { name: "Safety · 2" }),
    ).toBeInTheDocument();
  });

  it("fuses guards and checkpoints into one persistent workspace", async () => {
    view();

    const workspace = await screen.findByRole("region", {
      name: "Screen safeguards for raid",
    });
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(
      within(workspace).getByRole("tab", { name: "Safety · 2" }),
    ).toHaveAttribute("aria-selected", "true");

    fireEvent.click(
      within(workspace).getByRole("tab", { name: "Vision" }),
    );
    expect(
      within(workspace).getByRole("tab", { name: "Vision" }),
    ).toHaveAttribute("aria-selected", "true");
    expect(
      await within(workspace).findByText("No checkpoints yet"),
    ).toBeInTheDocument();
    expect(
      within(workspace).queryByText("What should it watch for?"),
    ).not.toBeInTheDocument();

    fireEvent.click(
      within(workspace).getByRole("button", { name: "Add a checkpoint" }),
    );
    expect(
      within(workspace).getByText("What should it watch for?"),
    ).toBeInTheDocument();
  });

  it("keeps an empty safeguards panel compact until a guard is requested", async () => {
    view();

    const workspace = await screen.findByRole("region", {
      name: "Screen safeguards for raid",
    });
    expect(
      await within(workspace).findByText("No guards yet.", { exact: false }),
    ).toBeInTheDocument();
    expect(
      within(workspace).getByRole("button", { name: "Add a guard" }),
    ).toBeInTheDocument();
    expect(
      within(workspace).queryByPlaceholderText("Name this trigger"),
    ).not.toBeInTheDocument();
  });

  it("keeps the macro library in a six-row scroll region", async () => {
    invoke.mockImplementation(async (command) => {
      if (command === "list_macros") {
        return Array.from({ length: 8 }, (_, index) => ({
          ...raid,
          name: `macro ${index + 1}`,
        }));
      }
      if (command === "list_templates") return [];
      if (command === "get_all_guard_counts") {
        return { ok: true, counts: {} };
      }
      return { ok: true };
    });

    view();

    const list = await screen.findByRole("region", { name: "Saved macros" });
    expect(list).toHaveAttribute("data-visible-rows", "6");
    expect(within(list).getAllByRole("article")).toHaveLength(8);
  });

  it("keeps compact labels intact and uses an icon-only continuous run action", async () => {
    view({
      mode: "idle",
      config: { hotkey_record: "F6" },
    } as Status);

    expect(
      await screen.findByText(
        "Create, manage, and run your macros with ease.",
      ),
    ).toBeInTheDocument();
    expect(screen.queryByText("F6")).not.toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Run continuously" }),
    ).toBeInTheDocument();
    expect(screen.queryByText("Continuous")).not.toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Ready" })).toBeInTheDocument();
  });

  it("summarizes the visible macro library with real totals", async () => {
    view();

    const overview = await screen.findByRole("region", {
      name: "Library overview",
    });
    expect(within(overview).getByText("2 macros at a glance")).toBeInTheDocument();
    expect(within(overview).getByText("7,486")).toBeInTheDocument();
    expect(within(overview).getByText("12:22")).toBeInTheDocument();
    expect(within(overview).getByText("24")).toBeInTheDocument();
  });
});
