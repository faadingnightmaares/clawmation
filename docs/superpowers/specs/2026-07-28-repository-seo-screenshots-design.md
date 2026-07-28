# Repository SEO and Screenshots Design

## Goal

Make the Clawmation repository easier to discover and easier to understand
without turning the README into a marketing landing page. Everything must remain
formal, factual, and visually identical to the real application.

## Audience and search intent

The primary reader wants a Windows macro recorder for games, Roblox automation,
screen-aware playback, or local computer-vision automation. The secondary reader
is a Rust, React, or Tauri contributor evaluating the implementation.

Use these phrases naturally where they describe real behavior:

- Windows macro recorder and macro player
- game automation and Roblox macro
- screen detection, image matching, pixel detection, and OCR
- visual workflows and macro chains
- Rust, React, and Tauri

Do not repeat keywords unnaturally, claim support the app does not have, or use
promotional superlatives.

## GitHub metadata

- Replace the current description with one concise sentence containing the
  product category, Windows scope, and screen-aware differentiation.
- Expand repository topics to cover the real user and contributor search terms:
  Windows automation, game automation, Roblox, macro recording and playback,
  screen recognition, computer vision, OCR, Rust, React, and Tauri.
- Keep the repository public and retain the existing release and issue URLs.
- Prepare a 1280×640 social-preview image using the real Home screen, the
  Clawmation mark, and product name only. No slogan, glow, device mockup, or
  decorative marketing treatment.

## README information architecture

1. Product mark, name, one factual sentence, and a restrained badge row.
2. A visible link to the latest signed Windows installer.
3. One large real Home screenshot.
4. A short capabilities section covering Macros, Watch, Loops, and local-only
   privacy.
5. A large Loops demonstration followed by focused Macros and Watch screenshots.
6. Portable `.clawmation` and `.clawbundle` behavior.
7. Requirements, installation, development, testing, architecture, contributing,
   and license information.

The first 100 words must answer what Clawmation is, which platform it supports,
what makes it different, and where to download it. Existing technical depth
remains available lower in the README, but obsolete Autopilot and test-count
copy is corrected.

## Screenshot system

Capture the real current UI at one consistent viewport and theme. Store optimized
WebP files under `docs/media/`:

- `home.webp`: the uncluttered Home workspace with realistic recent activity.
- `loops.webp`: the complete Loops workspace and the primary product demo.
- `macros.webp`: a populated macro library with the editor visible.
- `watch.webp`: a configured screen-watch action with a real template preview.
- `social-preview.png`: the GitHub-safe 1280×640 repository preview.

Screenshots must contain only synthetic data. They may not expose personal file
paths, usernames, live game instances, or the developer's real macros.

The Loops capture uses a formal sample named **Daily Quest Rotation**. It must
show a readable connected workflow with Start, an imported saved Macro node, a
vision wait/check, explicit **If works** and **If fails** paths, recovery, delay,
and Stop. The imported Macro node must show its actual embedded snapshot or
configuration through the existing Clawmation UI; it may not be painted onto the
screenshot afterward.

The fixture exists only to produce deterministic documentation captures. It
must not alter a user's app data or activate recording, playback, input
injection, screen capture, or external processes.

## Visual rules

- Use the application's real chrome, typography, spacing, icons, and theme.
- Crop to the app; never include the desktop, browser controls, or Codex UI.
- Do not add gradients, glow, fake shadows, floating labels, arrows, captions
  inside the image, or device frames.
- Keep screenshot captions short and factual.
- Use descriptive alt text that helps accessibility and search discovery without
  keyword stuffing.
- Prefer WebP and keep each README screenshot reasonably small while preserving
  readable text.

## Implementation boundaries

Documentation capture support must be isolated from production behavior. A
deterministic documentation-only fixture or mock seam may supply API data, but it
must be excluded from production builds and must not change normal Tauri command
handling.

GitHub description and topics are updated only after the README and screenshots
are ready, so repository metadata never advertises unfinished material.

## Verification

- TypeScript, frontend tests, Rust tests, and the production build remain green
  if capture support touches source code.
- Every screenshot is visually inspected at full resolution.
- Image dimensions, formats, and file sizes are checked.
- README links, anchors, release download URLs, and image paths resolve.
- The README contains no obsolete Autopilot claims or stale test counts.
- Demo assets are scanned for personal paths and identifying data.
- GitHub metadata is read back after mutation to confirm the final description
  and topics.
- The working tree is checked so the unrelated `ui-concept/` directory remains
  untouched.
