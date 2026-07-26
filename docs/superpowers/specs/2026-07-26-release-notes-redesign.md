# Release Notes Redesign for v1.1.6

## Goal

Make the in-app update prompt easy to scan without losing information. The
prompt must remain usable when a release carries substantially more text than
v1.1.6, and the v1.1.6 release must ship with notes that explain the two
reliability fixes in user-facing language.

## Selected Direction

Use the approved **Structured highlights** layout:

- an `Update available` eyebrow;
- a clear release title and one-sentence summary;
- a visible current-version to available-version transition;
- numbered highlight rows with a short title and supporting detail;
- persistent `Not now` and `Install and restart` actions.

The layout follows the app's existing warm parchment/ink/gold visual system. It
does not introduce another typeface, accent color, or decorative treatment.

## Component Boundary

Add a focused release-notes module rather than extending the already broad
Settings component:

1. A pure parser accepts the updater's optional notes string and returns a small
   presentation model made of summary text and ordered content blocks.
2. A `ReleaseNotes` component renders that model as semantic headings, lists,
   paragraphs, and numbered highlights.
3. Settings continues to own update checking, dismissal, download progress, and
   installation. It passes only the current version, available version, notes,
   installation state, progress, and action callbacks into the update prompt.

This keeps updater state unchanged and makes note parsing/rendering independently
testable.

## Accepted Input

Release notes remain a plain string in `UpdateInfo.notes`. The parser recognizes
the small Markdown-compatible subset normally produced by GitHub releases:

- `#`, `##`, and `###` headings;
- unordered bullets beginning with `-`, `*`, or `+`;
- ordered bullets beginning with a number and a period;
- blank-line-separated paragraphs.

The first standalone paragraph before a heading becomes the release summary. If
there is no such paragraph, the prompt uses `A reliability and usability update
for Clawmation.` For the structured-highlight treatment, a heading introduces a
group and each bullet becomes a highlight, numbered continuously in source
order. A `Title: detail` bullet splits into a short title and supporting text;
optional Markdown bold markers around the title are removed. Bullets without
that delimiter use their full text as the highlight title. Paragraphs, including
ones beneath a heading, stay in source order. Unrecognized lines are preserved
as readable paragraph text.

The parser normalizes Windows and Unix line endings, trims surrounding
whitespace, and collapses repeated blank separators without altering words.

Rendering uses normal React text nodes. The component must not use
`dangerouslySetInnerHTML`, execute embedded HTML, or discard text it cannot
classify.

## Long-Content Behavior

The dialog is constrained to the available viewport and uses three vertical
regions:

1. The release identity and version transition remain visible at the top.
2. The notes region grows until it reaches its responsive maximum, then becomes
   the only scrolling region.
3. The decision buttons remain visible at the bottom.

Text wraps normally at every supported window width. There is no line clamp,
ellipsis, item limit, or silent truncation. Large notes can always be read by
scrolling inside the note region. A visible `Release highlights` label gives the
scrolling region an accessible name.

## Empty and Irregular Notes

- Missing or whitespace-only notes show: `This update includes reliability and
  maintenance improvements.`
- A short unstructured note renders as a normal paragraph instead of an empty
  highlight list.
- Extra blank lines are collapsed.
- Long unbroken strings wrap instead of widening the dialog.
- Installation failure behavior remains unchanged: the existing error toast is
  shown and the prompt closes so a later check can retry cleanly.

## Installation State

Before installation, the prompt displays the notes and both decision actions.
After the user selects `Install and restart`, dismissal is disabled as it is
today. The content region changes to download progress with a plain status line;
the title continues to identify the target version. No parser or layout work
changes the updater command protocol.

## v1.1.6 Release Content

The update manifest and GitHub release use the same concise notes:

### Reliable mouse playback

- **Accurate at every display scale:** Recorded mouse paths now land where they
  were captured when Windows scaling is set above 100%.

### Complete recordings

- **Full duration preserved:** Clawmation now keeps the final idle stretch before
  Stop, so a recording no longer plays back several seconds short.

### Easier updates

- **Release notes you can scan:** Update highlights are structured, readable, and
  remain usable for long changelogs.

The published updater manifest must carry these notes, not the workflow's
generic draft copy. Release verification compares the GitHub release body and
the `notes` value inside the uploaded `latest.json` before publication.

## Versioning and Attribution

Version `1.1.6` must agree in:

- `package.json`;
- `src-tauri/Cargo.toml`;
- `src-tauri/tauri.conf.json`;
- the root package entry in `package-lock.json`;
- the Clawmation package entry in `src-tauri/Cargo.lock`.

Commits for this release use GitHub user `faadingnightmaares` and the verified
GitHub noreply address
`50467767+faadingnightmaares@users.noreply.github.com`, so GitHub attributes the
contribution to the user's profile.

## Verification

Frontend tests cover:

- structured headings and highlights;
- plain-text fallback;
- missing and whitespace-only notes;
- preservation of all content in a deliberately long changelog;
- accessible release and notes-region labels;
- visible version transition and install actions;
- installation-progress rendering.

The release candidate must also pass TypeScript, frontend tests, the production
frontend build, all Rust library tests, and the safe ignored hardware cursor
regression at the machine's non-100% display scale. The release workflow is then
triggered by tag `v1.1.6`; its draft artifacts and signed updater manifest are
verified before publication.

## Out of Scope

- rich arbitrary Markdown or embedded HTML;
- links, images, or remote content inside release notes;
- changes to update cadence or background-check behavior;
- changes to the Home screen's Recent Activity history.
