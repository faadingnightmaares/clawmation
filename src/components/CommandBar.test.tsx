import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const theme = vi.hoisted(() => ({
  setTheme: vi.fn(),
}));

vi.mock("next-themes", () => ({
  useTheme: () => ({
    resolvedTheme: "dark",
    setTheme: theme.setTheme,
  }),
}));

vi.mock("@/components/WindowControls", () => ({
  WindowControls: () => null,
}));

import { CommandBar } from "./CommandBar";
import type { Status } from "@/api";

describe("CommandBar theme motion", () => {
  const originalMatchMedia = window.matchMedia;
  const originalStartViewTransition = (
    document as Document & { startViewTransition?: unknown }
  ).startViewTransition;

  beforeEach(() => {
    theme.setTheme.mockReset();
    window.matchMedia = vi.fn().mockReturnValue({
      matches: false,
      media: "",
      onchange: null,
      addListener: vi.fn(),
      removeListener: vi.fn(),
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      dispatchEvent: vi.fn(),
    });
  });

  afterEach(() => {
    window.matchMedia = originalMatchMedia;
    Object.defineProperty(document, "startViewTransition", {
      configurable: true,
      value: originalStartViewTransition,
    });
  });

  it("uses a compositor view transition when changing theme", async () => {
    const finished = Promise.resolve();
    const startViewTransition = vi.fn(
      (update: () => void | Promise<void>) => {
        const updateCallbackDone = Promise.resolve(update());
        return {
          finished,
          ready: Promise.resolve(),
          updateCallbackDone,
          skipTransition: vi.fn(),
        };
      },
    );
    Object.defineProperty(document, "startViewTransition", {
      configurable: true,
      value: startViewTransition,
    });

    render(
      <CommandBar
        status={null}
        view="dashboard"
        navigate={vi.fn()}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Switch to light" }));

    expect(startViewTransition).toHaveBeenCalledOnce();
    await waitFor(() => expect(theme.setTheme).toHaveBeenCalledWith("light"));
  });

  it("keeps recording status and stop controls out of the top bar", () => {
    render(
      <CommandBar
        status={{ mode: "recording" } as Status}
        view="macros"
        navigate={vi.fn()}
      />,
    );

    expect(screen.queryByText("Recording")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Stop" })).not.toBeInTheDocument();
  });
});
