import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { ViewSwitch } from "./ViewSwitch";

describe("ViewSwitch", () => {
  it("shows Nodes as Soon and never navigates to it", () => {
    const navigate = vi.fn();
    render(<ViewSwitch view="macros" navigate={navigate} />);

    const nodes = screen.getByRole("button", { name: "Nodes" });
    expect(nodes).toBeDisabled();
    expect(screen.getByText("Soon")).toBeInTheDocument();
    fireEvent.click(nodes);
    expect(navigate).not.toHaveBeenCalled();
  });
});
