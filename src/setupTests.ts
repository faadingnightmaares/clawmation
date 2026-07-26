// Registers @testing-library/jest-dom matchers (toBeInTheDocument, etc.) on
// Vitest's `expect`, plus RTL auto-cleanup between tests.
import "@testing-library/jest-dom/vitest";

class TestResizeObserver implements ResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
}

globalThis.ResizeObserver = TestResizeObserver;
HTMLElement.prototype.scrollIntoView = () => {};
