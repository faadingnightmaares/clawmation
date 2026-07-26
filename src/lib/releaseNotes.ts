export const DEFAULT_RELEASE_SUMMARY = "A reliability and usability update for Clawmation.";
export const DEFAULT_RELEASE_NOTES =
  "This update includes reliability and maintenance improvements.";

export interface ReleaseHighlight {
  number: number;
  title: string;
  detail: string;
}

export type ReleaseSectionContent =
  | { kind: "paragraph"; text: string }
  | { kind: "highlights"; items: ReleaseHighlight[] };

export interface ReleaseNotesSection {
  heading: string | null;
  content: ReleaseSectionContent[];
}

export interface ParsedReleaseNotes {
  summary: string;
  sections: ReleaseNotesSection[];
}

const headingPattern = /^\s{0,3}#{1,3}\s+(.+?)\s*#*\s*$/;
const bulletPattern = /^\s*(?:[-*+]|\d+\.)\s+(.+)$/;
const indentedContinuationPattern = /^(?:\t| {2,})(\S.*)$/;

function cleanInline(text: string): string {
  const trimmed = text.trim();
  const bold = trimmed.match(/^\*\*(.+)\*\*$/);
  return (bold?.[1] ?? trimmed).trim();
}

function splitHighlight(raw: string, number: number): ReleaseHighlight {
  const value = raw.trim();
  const boldColonInside = value.match(/^\*\*(.+?):\*\*\s*(.*)$/);
  if (boldColonInside) {
    return {
      number,
      title: cleanInline(boldColonInside[1]),
      detail: boldColonInside[2].trim(),
    };
  }

  const boldThenColon = value.match(/^\*\*(.+?)\*\*:\s*(.*)$/);
  if (boldThenColon) {
    return {
      number,
      title: cleanInline(boldThenColon[1]),
      detail: boldThenColon[2].trim(),
    };
  }

  const plain = value.match(/^([^:]{1,120}):\s+(.+)$/);
  if (plain) {
    return {
      number,
      title: cleanInline(plain[1]),
      detail: plain[2].trim(),
    };
  }

  return { number, title: cleanInline(value), detail: "" };
}

/**
 * Parse the deliberately small Markdown-compatible subset used in updater
 * manifests. Every input word is retained as text; HTML is never interpreted.
 */
export function parseReleaseNotes(notes?: string | null): ParsedReleaseNotes {
  const normalized = notes?.replace(/\r\n?/g, "\n").trim() ?? "";
  if (!normalized) {
    return {
      summary: DEFAULT_RELEASE_SUMMARY,
      sections: [
        {
          heading: null,
          content: [{ kind: "paragraph", text: DEFAULT_RELEASE_NOTES }],
        },
      ],
    };
  }

  const sections: ReleaseNotesSection[] = [{ heading: null, content: [] }];
  let current = sections[0];
  let paragraphLines: string[] = [];
  let highlightItems: ReleaseHighlight[] = [];
  let pendingBullet = "";
  let nextNumber = 1;

  const flushParagraph = () => {
    if (!paragraphLines.length) return;
    current.content.push({
      kind: "paragraph",
      text: paragraphLines.join(" ").replace(/\s+/g, " ").trim(),
    });
    paragraphLines = [];
  };

  const flushPendingBullet = () => {
    if (!pendingBullet) return;
    highlightItems.push(splitHighlight(pendingBullet, nextNumber));
    nextNumber += 1;
    pendingBullet = "";
  };

  const flushHighlights = () => {
    flushPendingBullet();
    if (!highlightItems.length) return;
    current.content.push({ kind: "highlights", items: highlightItems });
    highlightItems = [];
  };

  const flushAll = () => {
    flushParagraph();
    flushHighlights();
  };

  for (const line of normalized.split("\n")) {
    if (!line.trim()) {
      flushAll();
      continue;
    }

    const heading = line.match(headingPattern);
    if (heading) {
      flushAll();
      current = { heading: cleanInline(heading[1]), content: [] };
      sections.push(current);
      continue;
    }

    const bullet = line.match(bulletPattern);
    if (bullet) {
      flushParagraph();
      flushPendingBullet();
      pendingBullet = bullet[1].trim();
      continue;
    }

    const continuation = line.match(indentedContinuationPattern);
    if (pendingBullet && continuation) {
      pendingBullet += ` ${continuation[1].trim()}`;
      continue;
    }

    flushHighlights();
    paragraphLines.push(line.trim());
  }
  flushAll();

  const nonEmpty = sections.filter((section) => section.heading || section.content.length);
  const first = nonEmpty[0];
  const firstContent = first?.content[0];
  const hasContentAfterFirst =
    nonEmpty.length > 1 || (first?.content.length ?? 0) > 1;

  let summary = DEFAULT_RELEASE_SUMMARY;
  if (
    first?.heading === null &&
    firstContent?.kind === "paragraph" &&
    hasContentAfterFirst
  ) {
    summary = firstContent.text;
    first.content.shift();
  }

  const body = nonEmpty.filter((section) => section.heading || section.content.length);
  if (!body.length) {
    body.push({
      heading: null,
      content: [{ kind: "paragraph", text: DEFAULT_RELEASE_NOTES }],
    });
  }

  return { summary, sections: body };
}
