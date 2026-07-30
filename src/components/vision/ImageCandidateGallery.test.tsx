import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { ImageCandidateGallery } from "./ImageCandidateGallery";

function renderGallery(
  overrides: Partial<React.ComponentProps<typeof ImageCandidateGallery>> = {},
) {
  const onDrop = vi.fn();
  render(
    <ImageCandidateGallery
      candidates={[]}
      onChoose={vi.fn()}
      onMagicSelect={vi.fn()}
      onRemove={vi.fn()}
      onDrop={onDrop}
      {...overrides}
    />,
  );
  return { onDrop };
}

function paste(files: File[]) {
  fireEvent.paste(window, {
    clipboardData: {
      files,
    },
  });
}

describe("ImageCandidateGallery clipboard images", () => {
  it("imports the first pasted image while the gallery is hovered", () => {
    const { onDrop } = renderGallery();
    const image = new File(["pixels"], "button.png", { type: "image/png" });

    fireEvent.mouseEnter(screen.getByRole("region", { name: "Vision images" }));
    paste([image]);

    expect(onDrop).toHaveBeenCalledOnce();
    expect(onDrop).toHaveBeenCalledWith(image);
  });

  it("ignores pasted images when the pointer is outside the gallery", () => {
    const { onDrop } = renderGallery();
    const image = new File(["pixels"], "button.png", { type: "image/png" });

    paste([image]);

    expect(onDrop).not.toHaveBeenCalled();
  });

  it("ignores text clipboard content and images when the gallery is full", () => {
    const full = Array.from({ length: 8 }, (_, index) => `image-${index}.png`);
    const { onDrop } = renderGallery({ candidates: full });
    const gallery = screen.getByRole("region", { name: "Vision images" });
    fireEvent.mouseEnter(gallery);

    paste([new File(["text"], "note.txt", { type: "text/plain" })]);
    paste([new File(["pixels"], "ninth.png", { type: "image/png" })]);

    expect(onDrop).not.toHaveBeenCalled();
  });
});
