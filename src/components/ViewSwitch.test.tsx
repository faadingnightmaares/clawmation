import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const anime = vi.hoisted(() => ({
  animate: vi.fn(),
  spring: vi.fn(() => "spring-ease"),
}));

vi.mock("animejs", () => ({
  animate: anime.animate,
  spring: anime.spring,
}));

import { ViewSwitch } from "./ViewSwitch";

describe("ViewSwitch", () => {
  const originalResizeObserver = globalThis.ResizeObserver;
  const originalMatchMedia = window.matchMedia;

  beforeEach(() => {
    anime.animate.mockReset();
    anime.spring.mockClear();
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
    globalThis.ResizeObserver = originalResizeObserver;
    window.matchMedia = originalMatchMedia;
  });

  it("opens the Loops workspace", () => {
    const navigate = vi.fn();
    render(<ViewSwitch view="macros" navigate={navigate} />);

    const nodes = screen.getByRole("button", { name: "Loops" });
    expect(nodes).toBeEnabled();
    fireEvent.click(nodes);
    expect(navigate).toHaveBeenCalledWith("nodes");
  });

  it("keeps the workspace transition alive when ResizeObserver reports its initial size", () => {
    const cancel = vi.fn();
    anime.animate.mockReturnValue({ cancel });

    class ImmediateResizeObserver {
      constructor(private readonly callback: ResizeObserverCallback) {}

      observe() {
        this.callback(
          [
            {
              contentRect: { width: 1 },
            } as ResizeObserverEntry,
          ],
          this as unknown as ResizeObserver,
        );
      }

      unobserve() {}
      disconnect() {}
    }

    globalThis.ResizeObserver =
      ImmediateResizeObserver as unknown as typeof ResizeObserver;

    const { container, rerender } = render(
      <ViewSwitch view="dashboard" navigate={vi.fn()} />,
    );
    rerender(<ViewSwitch view="macros" navigate={vi.fn()} />);

    expect(anime.animate).toHaveBeenCalled();
    expect(anime.spring).toHaveBeenCalled();
    expect(cancel).not.toHaveBeenCalled();
    expect(
      container.querySelector("[data-view-switch-refraction]"),
    ).toBeInTheDocument();
  });

  it("never scales nav buttons or changes icon geometry while switching", () => {
    anime.animate.mockReturnValue({ cancel: vi.fn() });
    const { rerender } = render(
      <ViewSwitch view="dashboard" navigate={vi.fn()} />,
    );
    const homeIcon = screen
      .getByRole("button", { name: "Home" })
      .querySelector("svg");
    const initialStroke = homeIcon?.getAttribute("stroke-width");

    rerender(<ViewSwitch view="macros" navigate={vi.fn()} />);

    const macrosIcon = screen
      .getByRole("button", { name: "Macros" })
      .querySelector("svg");
    expect(macrosIcon?.getAttribute("stroke-width")).toBe(initialStroke);
    expect(
      anime.animate.mock.calls.some(
        ([target]) => target instanceof HTMLButtonElement,
      ),
    ).toBe(false);
    expect(
      anime.animate.mock.calls.some(
        ([, options]) =>
          typeof options === "object" &&
          options !== null &&
          "duration" in options &&
          options.duration === 360,
      ),
    ).toBe(true);
  });
});
