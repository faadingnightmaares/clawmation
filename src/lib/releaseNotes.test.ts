import { describe, expect, it } from "vitest";

import { DEFAULT_RELEASE_NOTES, DEFAULT_RELEASE_SUMMARY, parseReleaseNotes } from "./releaseNotes";

describe("parseReleaseNotes", () => {
  it("turns GitHub-style sections and bullets into ordered highlights", () => {
    const parsed = parseReleaseNotes(`
A reliability update for recording and playback.

### Reliable mouse playback

- **Accurate at every display scale:** Recorded mouse paths land where they were captured.
- Raw Input stays intact

The same coordinates are used from recording through playback.
`);

    expect(parsed.summary).toBe("A reliability update for recording and playback.");
    expect(parsed.sections).toEqual([
      {
        heading: "Reliable mouse playback",
        content: [
          {
            kind: "highlights",
            items: [
              {
                number: 1,
                title: "Accurate at every display scale",
                detail: "Recorded mouse paths land where they were captured.",
              },
              { number: 2, title: "Raw Input stays intact", detail: "" },
            ],
          },
          {
            kind: "paragraph",
            text: "The same coordinates are used from recording through playback.",
          },
        ],
      },
    ]);
  });

  it("keeps a single unstructured note as readable body copy", () => {
    const parsed = parseReleaseNotes("One concise maintenance note.");

    expect(parsed.summary).toBe(DEFAULT_RELEASE_SUMMARY);
    expect(parsed.sections).toEqual([
      {
        heading: null,
        content: [{ kind: "paragraph", text: "One concise maintenance note." }],
      },
    ]);
  });

  it("uses an explicit fallback for missing or whitespace-only notes", () => {
    for (const notes of [null, undefined, "", " \r\n\t "]) {
      const parsed = parseReleaseNotes(notes);
      expect(parsed.summary).toBe(DEFAULT_RELEASE_SUMMARY);
      expect(parsed.sections[0]?.content).toEqual([
        { kind: "paragraph", text: DEFAULT_RELEASE_NOTES },
      ]);
    }
  });

  it("normalizes line endings and preserves wrapped bullet detail", () => {
    const parsed = parseReleaseNotes(
      "Summary.\r\n\r\n## Complete recordings\r\n- **Full duration preserved:** Keeps the final idle stretch\r\n  before Stop is pressed.\r\n1. Works for ordered bullets too.",
    );

    expect(parsed.sections[0]?.content).toEqual([
      {
        kind: "highlights",
        items: [
          {
            number: 1,
            title: "Full duration preserved",
            detail: "Keeps the final idle stretch before Stop is pressed.",
          },
          { number: 2, title: "Works for ordered bullets too.", detail: "" },
        ],
      },
    ]);
  });

  it("does not cap or truncate a long changelog", () => {
    const bullets = Array.from(
      { length: 80 },
      (_, i) => `- **Change ${i + 1}:** Preserved detail token-${i + 1}`,
    ).join("\n");

    const parsed = parseReleaseNotes(`Long release summary.\n\n## Everything\n${bullets}`);
    const highlights = parsed.sections[0]?.content[0];

    expect(highlights?.kind).toBe("highlights");
    if (highlights?.kind !== "highlights") throw new Error("expected highlights");
    expect(highlights.items).toHaveLength(80);
    expect(highlights.items[79]).toEqual({
      number: 80,
      title: "Change 80",
      detail: "Preserved detail token-80",
    });
  });
});
