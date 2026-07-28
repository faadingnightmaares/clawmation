import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "./dropdown-menu";
import { Popover, PopoverContent, PopoverTrigger } from "./popover";
import {
  AlertDialog,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogTitle,
} from "./alert-dialog";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "./select";

describe("floating UI surfaces", () => {
  it("uses one placement-safe motion surface for menus and popovers", () => {
    render(
      <>
        <DropdownMenu open modal={false}>
          <DropdownMenuTrigger>Menu</DropdownMenuTrigger>
          <DropdownMenuContent data-testid="menu-surface">
            <DropdownMenuItem>Action</DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
        <Popover open>
          <PopoverTrigger>Details</PopoverTrigger>
          <PopoverContent data-testid="popover-surface">
            Content
          </PopoverContent>
        </Popover>
      </>,
    );

    expect(screen.getByTestId("menu-surface")).toHaveClass(
      "ui-floating-surface",
    );
    expect(screen.getByTestId("popover-surface")).toHaveClass(
      "ui-floating-surface",
    );
  });

  it("opens selects in stable Popper mode instead of selected-item alignment", () => {
    render(
      <Select open value="recent">
        <SelectTrigger aria-label="Sort">
          <SelectValue />
        </SelectTrigger>
        <SelectContent data-testid="select-surface">
          <SelectItem value="recent">Recent</SelectItem>
          <SelectItem value="name">Name</SelectItem>
        </SelectContent>
      </Select>,
    );

    const surface = screen.getByTestId("select-surface");
    expect(surface).toHaveClass("ui-floating-surface");
    expect(
      surface.closest("[data-radix-popper-content-wrapper]"),
    ).not.toBeNull();
  });

  it("centers alert dialogs with one transform instead of translating twice", () => {
    render(
      <AlertDialog open>
        <AlertDialogContent>
          <AlertDialogTitle>Delete macro?</AlertDialogTitle>
          <AlertDialogDescription>
            This action cannot be undone.
          </AlertDialogDescription>
        </AlertDialogContent>
      </AlertDialog>,
    );

    const dialog = screen.getByRole("alertdialog", {
      name: "Delete macro?",
    });
    expect(dialog).toHaveStyle({
      transform: "translate(-50%, -50%)",
    });
    expect(dialog.className).not.toContain("translate-x-[-50%]");
    expect(dialog.className).not.toContain("translate-y-[-50%]");
  });
});
