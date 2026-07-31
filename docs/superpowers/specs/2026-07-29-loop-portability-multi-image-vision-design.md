# Portable Loops and Multi-Image Vision

## Goal

Make Loops portable through Clawmation's existing file ecosystem and let one
vision operation recognize multiple visual states of the same target. A normal
button, its hovered state, and other small state variations can all trigger the
same Wait, Find & Click, or Watch action without duplicate nodes or triggers.

## Scope

This change covers:

- importing and exporting complete Loops;
- packaging every image referenced directly or through an embedded macro;
- multiple alternative images in Loop Wait and Find & Click nodes;
- multiple alternative images in Watch triggers;
- backward-compatible loading of every existing single-image graph and trigger.

Recorded macro playback, color detection, OCR, checkpoint events, and the
meaning of `.clawmation` macro files remain unchanged.

## Portable Loop format

Loops use `.clawbundle`, because that format is Clawmation's self-contained
asset-bearing container. A Loop export always uses one workflow and one
extension, even when the particular Loop has no images.

The archive layout is:

```text
manifest.json
payload/loop.json
assets/<blake3>.<ext>    (when referenced)
```

The manifest identifies the payload as `com.clawmation.loop`. It retains the
current container version, producing app version, uncompressed byte lengths,
and BLAKE3 digests. Loop JSON uses the existing Zstandard compression policy.
PNG, JPEG, and WebP assets remain byte-for-byte lossless and are stored without
redundant recompression; BMP assets use Zstandard.

Every distinct image is content-addressed and stored once, even if several
nodes or embedded steps reference it. Import verifies the complete manifest,
entry set, declared sizes, digests, image types, graph limits, and every image
reference before writing anything.

Import installs images through the existing collision-safe template store,
rewrites the imported graph to those installed paths, and saves it under
`macros/nodes`. An existing Loop is never overwritten; a numeric suffix is
added to the imported name. Invalid or partial archives leave no installed Loop.

The current `.clawbundle` macro format and all legacy imports remain supported.

## Multi-image data model

The existing singular fields remain the compatibility source:

- Loop/AI `Step.template`;
- Watch `Guard.template_path`.

New fields add alternatives:

- `Step.templates: Vec<String>`;
- `Guard.template_paths: Vec<String>`.

The effective candidate list is the singular field followed by the plural
field, with empty and duplicate paths removed. New saves keep the first
candidate in the singular field so older Clawmation versions can still use the
primary image. Existing files with only the singular field load unchanged.

Each operation accepts at most eight candidates. The candidates share the
operation's region, confidence threshold, action, and click geometry. They are
alternative appearances of one target, not eight independent actions.

Embedded macro snapshots are included in traversal. Their Wait and Find & Click
steps receive the same path collection and remapping as top-level Loop vision
nodes.

## Matching behavior

Image candidates use OR semantics:

1. Capture the configured region once.
2. Check the last successful candidate first when one exists.
3. Accept it immediately only for a near-exact match at or above `0.98`;
   otherwise evaluate every remaining candidate against the same frame.
4. Select the highest-confidence valid result and fire the action once.

The detector caches decoded templates as it does today. It remembers the last
successful candidate per operation and checks that candidate first on later
polls. Only a near-exact sticky match returns immediately; otherwise the
remaining candidates are evaluated and the best result wins. This keeps the common
steady-state path close to single-image cost while still handling a normal to
hovered transition.

A missing or corrupt candidate is reported once and skipped. Other valid
candidates remain usable. The operation fails validation only when template
matching is selected and no usable candidate is configured.

## Loop editor

The Loop toolbar's existing controls menu adds:

- **Import Loop**
- **Export Loop**

Export saves the current valid graph first when it has unsaved changes, then
opens the `.clawbundle` destination picker. Import opens a `.clawbundle` picker,
installs the Loop, refreshes the Loop list, and selects the imported Loop.

The single-image drop zone in Wait and Find & Click inspectors becomes a compact
thumbnail gallery:

- existing images appear as individual tiles;
- dropping or choosing images appends them;
- Magic Select appends a captured state;
- each tile has its own remove action;
- the primary tile is visually identified;
- duplicate image content is ignored;
- the gallery clearly shows the eight-image limit.

Node cards show the primary thumbnail plus an image-count badge when alternatives
exist. Thumbnails are presentation data only; runtime and export correctness
depend on the referenced image files.

## Watch editor

The Watch image section uses the same gallery interaction. Surgical capture or
file selection appends an alternative instead of silently replacing the current
image. The first captured image retains the shared click point. Replacing that
geometry remains an explicit capture action, preventing an additional hovered
state from unexpectedly changing where the trigger clicks.

Saving, testing, readiness checks, and trigger normalization understand both the
legacy primary image and the alternative list.

## Commands and integration

New backend commands provide Loop import and export. They reuse the transfer
archive's bounded reads, digest verification, compression, safe path checks,
content deduplication, and collision-safe installation instead of introducing a
second archive implementation.

The frontend API exposes typed results for both commands. Loop workspace refresh
and selection stay owned by the parent workspace, while the editor only initiates
the transfer and reports its result.

The file-association dispatcher recognizes a `com.clawmation.loop`
`.clawbundle`, imports it through the same command path, and opens the Loops
workspace when the application receives the file.

## Error handling and safety

- A referenced image missing at export produces a clear error naming the node
  or trigger and path.
- Import rejects undeclared assets, unsafe paths, duplicate archive entries,
  oversized payloads, unsupported graph versions, malformed graphs, and
  dangling image references.
- Validation and installation complete before the Loop file is written.
- Import never overwrites an existing Loop or unrelated template.
- Cancelling either file picker changes nothing.
- Failed import/export keeps the current editor state and selection.
- Unsupported older applications still see the primary image if they open a
  compatible saved JSON file, but they correctly reject the new Loop bundle
  manifest instead of misreading it as a macro bundle.

## Verification

Backend coverage:

- Loop bundle round-trip preserves graph nodes, edges, and name.
- All direct and embedded-step images are included and remapped.
- Identical images are stored once.
- A missing, undeclared, corrupt, oversized, or traversal asset is rejected.
- Existing-name imports receive a safe suffix and never overwrite.
- Existing macro `.clawmation`, macro `.clawbundle`, and legacy bundles still
  round-trip.
- Old singular-image steps and guards produce one effective candidate.
- Candidate normalization removes empty paths and duplicates.
- Normal and hovered templates use OR semantics and fire only one action.
- A bad candidate does not suppress a valid alternative.
- The eight-image limit is enforced at validation and import boundaries.

Frontend coverage:

- Loop import/export controls invoke the correct commands.
- Successful import refreshes and selects the new Loop.
- Unsaved export validates and saves before packaging.
- Adding, dropping, Magic Selecting, and individually removing images update the
  gallery without replacing other states.
- Existing single-image graphs and Watch triggers render correctly.
- The primary indicator, count badge, duplicate handling, and eight-image limit
  are accessible and deterministic.

Run focused transfer, graph, vision, and editor tests followed by the complete
Rust library suite, frontend tests, TypeScript checking, Cargo checking, and the
production frontend build. Runtime changes remain uncommitted and unpublished
until manual normal/hovered button testing succeeds in both Watch and Loops.
