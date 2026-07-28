import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const appMetrics = vi.hoisted(() => ({
  macroMounts: 0,
  suspendMacros: false,
  macroPromise: Promise.resolve(),
  releaseMacros: (() => {}) as () => void,
}));

vi.mock("@/api", () => ({
  onUpdateAvailable: vi.fn(async () => () => {}),
}));

vi.mock("@/useStatus", () => ({
  useStatus: () => null,
}));

vi.mock("@/components/CommandBar", () => ({
  CommandBar: ({
    view,
    navigate,
  }: {
    view: string;
    navigate: (view: string) => void;
  }) => (
    <nav aria-label="Test navigation">
      {[
        ["dashboard", "Home"],
        ["macros", "Macros"],
        ["nodes", "Loops"],
        ["vision", "Watch"],
        ["settings", "Settings"],
      ].map(([id, label]) => (
        <button
          key={id}
          type="button"
          aria-current={view === id ? "page" : undefined}
          onClick={() => navigate(id)}
        >
          {label}
        </button>
      ))}
    </nav>
  ),
}));

vi.mock("@/components/ui/sonner", () => ({
  Toaster: () => null,
}));

vi.mock("@/views/Home", () => ({
  Home: () => <div>Home content</div>,
}));

vi.mock("@/views/Macros", async () => {
  const React = await import("react");
  return {
    Macros: () => {
      React.useState(() => {
        appMetrics.macroMounts += 1;
        return 0;
      });
      if (appMetrics.suspendMacros) throw appMetrics.macroPromise;
      return <div>Macros content</div>;
    },
  };
});

vi.mock("@/views/Nodes", () => ({
  Nodes: () => <div>Loops content</div>,
}));

vi.mock("@/views/Watch", () => ({
  Watch: () => <div>Watch content</div>,
}));

vi.mock("@/views/Settings", () => ({
  Settings: () => <div>Settings content</div>,
}));

import App, { updateViewCache } from "./App";

describe("App navigation performance", () => {
  beforeEach(() => {
    appMetrics.macroMounts = 0;
    appMetrics.suspendMacros = false;
    appMetrics.macroPromise = Promise.resolve();
    appMetrics.releaseMacros = () => {};
  });

  it("keeps only the three most recent views cached", () => {
    expect(
      updateViewCache(
        ["dashboard", "macros", "nodes"],
        "vision",
      ),
    ).toEqual(["macros", "nodes", "vision"]);
    expect(updateViewCache(["macros", "nodes", "vision"], "macros")).toEqual([
      "nodes",
      "vision",
      "macros",
    ]);
  });

  it("survives rapid switching and reuses a cached heavy workspace", async () => {
    const { container } = render(<App />);

    fireEvent.click(screen.getByRole("button", { name: "Macros" }));
    fireEvent.click(screen.getByRole("button", { name: "Loops" }));
    fireEvent.click(screen.getByRole("button", { name: "Watch" }));

    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Watch" })).toHaveAttribute(
        "aria-current",
        "page",
      ),
    );
    expect(container.querySelectorAll("[data-view-surface]")).toHaveLength(3);

    fireEvent.click(screen.getByRole("button", { name: "Macros" }));
    await screen.findByText("Macros content");
    expect(appMetrics.macroMounts).toBe(1);
  });

  it("updates navigation immediately while a slow view keeps the previous frame visible", async () => {
    appMetrics.suspendMacros = true;
    appMetrics.macroPromise = new Promise<void>((resolve) => {
      appMetrics.releaseMacros = resolve;
    });
    const { container } = render(<App />);

    fireEvent.click(screen.getByRole("button", { name: "Macros" }));

    expect(screen.getByRole("button", { name: "Macros" })).toHaveAttribute(
      "aria-current",
      "page",
    );
    expect(
      container.querySelector('[data-view-surface="dashboard"]'),
    ).toHaveAttribute("data-active", "true");
    expect(screen.getByText("Home content")).toBeVisible();

    appMetrics.suspendMacros = false;
    await act(async () => {
      appMetrics.releaseMacros();
      await appMetrics.macroPromise;
    });

    await waitFor(() =>
      expect(
        container.querySelector('[data-view-surface="macros"]'),
      ).toHaveAttribute("data-active", "true"),
    );
  });
});
