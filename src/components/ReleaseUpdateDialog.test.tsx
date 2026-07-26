import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { UpdateInfo } from "@/api";
import { ReleaseUpdateDialog } from "./ReleaseUpdateDialog";

const update: UpdateInfo = {
  update_available: true,
  current: "1.1.5",
  latest: "1.1.6",
  notes: `A reliability update for recording and playback.

## Reliable mouse playback
- **Accurate at every display scale:** Recorded mouse paths now land where they were captured.

## Complete recordings
- **Full duration preserved:** The final idle stretch is kept before Stop.`,
};

function renderDialog(overrides: Partial<React.ComponentProps<typeof ReleaseUpdateDialog>> = {}) {
  const props: React.ComponentProps<typeof ReleaseUpdateDialog> = {
    info: update,
    installing: false,
    progress: null,
    onDismiss: vi.fn(),
    onInstall: vi.fn(),
    ...overrides,
  };
  render(<ReleaseUpdateDialog {...props} />);
  return props;
}

describe("ReleaseUpdateDialog", () => {
  it("presents versions, structured highlights, and an accessible notes region", () => {
    renderDialog();

    expect(
      screen.getByRole("alertdialog", { name: "Clawmation 1.1.6 is ready" }),
    ).toBeInTheDocument();
    expect(screen.getByText("1.1.5")).toBeInTheDocument();
    expect(screen.getByText("1.1.6", { selector: "[data-version='available']" })).toBeInTheDocument();

    const notes = screen.getByRole("region", { name: "Release highlights" });
    expect(notes).toHaveTextContent("Reliable mouse playback");
    expect(notes).toHaveTextContent("Accurate at every display scale");
    expect(notes).toHaveTextContent("Complete recordings");
    expect(notes).toHaveTextContent("Full duration preserved");
  });

  it("keeps every item in a long release available to the reader", () => {
    const notes = Array.from(
      { length: 75 },
      (_, i) => `- **Fix ${i + 1}:** Full detail ${i + 1}`,
    ).join("\n");
    renderDialog({ info: { ...update, notes: `Long update.\n\n## Fixes\n${notes}` } });

    expect(screen.getAllByRole("listitem")).toHaveLength(75);
    expect(screen.getByText("Fix 75")).toBeInTheDocument();
    expect(screen.getByText("Full detail 75")).toBeInTheDocument();
  });

  it("keeps both decisions visible and routes them to the owning Settings state", () => {
    const props = renderDialog();

    expect(screen.getByText("Installing restarts the app. Finish any active run first.")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Not now" }));
    expect(props.onDismiss).toHaveBeenCalledOnce();

    fireEvent.click(screen.getByRole("button", { name: "Install and restart" }));
    expect(props.onInstall).toHaveBeenCalledOnce();
  });

  it("shows determinate download progress without the decision actions", () => {
    renderDialog({ installing: true, progress: 68 });

    expect(screen.getByText("Downloading… 68%")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Not now" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Install and restart" })).not.toBeInTheDocument();
  });
});
