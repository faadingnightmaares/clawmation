import { describe, expect, it } from "vitest";

import {
  MAX_IMAGE_CANDIDATES,
  appendImageCandidate,
  imageCandidates,
  removeImageCandidate,
  splitImageCandidates,
} from "./visionImages";

describe("vision image candidates", () => {
  it("keeps the legacy image first and removes empty duplicates", () => {
    expect(imageCandidates("normal.png", ["hovered.png", "", "normal.png"]))
      .toEqual(["normal.png", "hovered.png"]);
  });

  it("promotes the next image when the primary is removed", () => {
    const remaining = removeImageCandidate(["normal.png", "hovered.png"], 0);
    expect(splitImageCandidates(remaining)).toEqual({
      primary: "hovered.png",
      alternatives: [],
    });
  });

  it("ignores duplicate additions and enforces the eight-image cap", () => {
    expect(appendImageCandidate(["normal.png"], "normal.png")).toMatchObject({
      added: false,
      full: false,
    });
    const full = Array.from(
      { length: MAX_IMAGE_CANDIDATES },
      (_, index) => `${index}.png`,
    );
    expect(appendImageCandidate(full, "extra.png")).toEqual({
      candidates: full,
      added: false,
      full: true,
    });
  });
});
