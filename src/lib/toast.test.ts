import { beforeEach, describe, expect, it, vi } from "vitest";

const sonner = vi.hoisted(() => {
  const base = vi.fn();
  return {
    toast: Object.assign(base, {
      success: vi.fn(),
      error: vi.fn(),
      warning: vi.fn(),
      info: vi.fn(),
    }),
  };
});

vi.mock("sonner", () => ({ toast: sonner.toast }));

import { notify, notifyAction, notifyUndo } from "./toast";

describe("toast policy", () => {
  beforeEach(() => {
    sonner.toast.mockClear();
    sonner.toast.success.mockClear();
    sonner.toast.error.mockClear();
    sonner.toast.warning.mockClear();
    sonner.toast.info.mockClear();
  });

  it("does not interrupt routine success, info, or announcement actions", () => {
    notify("success", "Saved.");
    notify("info", "Deleted.");
    notifyAction("Update available.", "Open", vi.fn());

    expect(sonner.toast.success).not.toHaveBeenCalled();
    expect(sonner.toast.info).not.toHaveBeenCalled();
  });

  it("still surfaces errors, warnings, and destructive undo actions", () => {
    notify("error", "Save failed.");
    notify("warning", "Connection is unstable.");
    notifyUndo("Deleted guard.", vi.fn());

    expect(sonner.toast.error).toHaveBeenCalledWith("Save failed.");
    expect(sonner.toast.warning).toHaveBeenCalledWith(
      "Connection is unstable.",
    );
    expect(sonner.toast).toHaveBeenCalledWith(
      "Deleted guard.",
      expect.objectContaining({
        action: expect.objectContaining({ label: "Undo" }),
      }),
    );
  });
});
