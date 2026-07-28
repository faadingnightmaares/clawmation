import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn<
  (command: string, args?: Record<string, unknown>) => Promise<unknown>
>();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (command: string, args?: Record<string, unknown>) =>
    invoke(command, args),
}));

import { Settings } from "./Settings";

const config = {
  capture_backend: "auto",
  hotkey_record: "",
  hotkey_play: "",
  hotkey_stop: "",
  indicator_on_top: true,
  humanize_clicks: false,
  notify_on_schedule: false,
  notify_on_complete: true,
};

function renderSettings() {
  return render(<Settings status={null} navigate={vi.fn()} />);
}

function installDefaultBackend() {
  invoke.mockImplementation(async (command) => {
    switch (command) {
      case "get_config":
        return config;
      case "get_data_paths":
        return {
          ok: true,
          root: "C:\\Clawmation",
          macros_dir: "C:\\Clawmation\\macros",
          templates_dir: "C:\\Clawmation\\templates",
          snapshots_dir: "C:\\Clawmation\\snapshots",
          config_dir: "C:\\Clawmation\\config",
          macro_count: 12,
          template_count: 5,
          snapshot_count: 3,
        };
      case "get_version":
        return { version: "1.1.9" };
      case "check_update":
        return {
          update_available: true,
          current: "1.1.9",
          latest: "1.2.0",
          notes: "A polished release.\n\n## Improvements\n- **Smooth UI:** Refined every workspace.",
        };
      default:
        return { ok: true, unbound: [] };
    }
  });
}

describe("Settings workbench", () => {
  beforeEach(() => {
    invoke.mockReset();
    installDefaultBackend();
  });

  it("holds a stable loading surface until the initial settings requests settle", async () => {
    let resolveConfig!: (value: unknown) => void;
    let resolvePaths!: (value: unknown) => void;
    let resolveVersion!: (value: unknown) => void;
    const pending = {
      get_config: new Promise((resolve) => {
        resolveConfig = resolve;
      }),
      get_data_paths: new Promise((resolve) => {
        resolvePaths = resolve;
      }),
      get_version: new Promise((resolve) => {
        resolveVersion = resolve;
      }),
    };
    invoke.mockImplementation(
      async (command) =>
        pending[command as keyof typeof pending] ?? { ok: true },
    );

    renderSettings();
    expect(screen.getByLabelText("Loading settings")).toBeInTheDocument();

    await act(async () => {
      resolveConfig(config);
      resolvePaths({
        ok: true,
        root: "C:\\Clawmation",
        macro_count: 0,
        template_count: 0,
        snapshot_count: 0,
      });
      resolveVersion({ version: "1.1.9" });
      await Promise.all(Object.values(pending));
    });

    expect(
      await screen.findByRole("heading", { name: "General" }),
    ).toBeInTheDocument();
    expect(screen.queryByLabelText("Loading settings")).not.toBeInTheDocument();
  });

  it("opens each documentation topic inside the same workbench", async () => {
    renderSettings();
    await screen.findByRole("heading", { name: "General" });

    fireEvent.click(
      screen.getByRole("button", { name: "Loops & chains" }),
    );

    expect(
      screen.getByRole("heading", { level: 1, name: "Loops & chains" }),
    ).toBeInTheDocument();
    expect(screen.getByText("Compose a chain inside a Loop")).toBeInTheDocument();
  });

  it("persists a captured global shortcut immediately", async () => {
    renderSettings();
    await screen.findByRole("heading", { name: "General" });
    fireEvent.click(screen.getByRole("button", { name: "Shortcuts" }));

    fireEvent.click(
      screen.getByRole("button", {
        name: "Start or stop recording: not set",
      }),
    );
    fireEvent.keyDown(window, {
      code: "KeyR",
      key: "r",
      ctrlKey: true,
      shiftKey: true,
    });

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("update_config", {
        patch: { hotkey_record: expect.any(String) },
      }),
    );
  });

  it("checks for an update and opens the full release notes", async () => {
    renderSettings();
    await screen.findByRole("heading", { name: "General" });
    expect(
      screen.queryByRole("button", { name: "Notifications" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Updates" }),
    ).not.toBeInTheDocument();
    fireEvent.click(
      screen.getByRole("button", { name: "Check for updates" }),
    );

    expect(
      await screen.findByRole("alertdialog", {
        name: "Clawmation 1.2.0 is ready",
      }),
    ).toBeInTheDocument();
    expect(screen.getByText("Smooth UI")).toBeInTheDocument();
  });
});
