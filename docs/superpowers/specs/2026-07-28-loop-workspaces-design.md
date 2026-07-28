# Loop Workspaces

## Purpose

The Nodes tab must model complete automation workflows, not one graph per
recorded macro. A workflow is called a **Loop**. Each Loop owns one directed
node graph and may contain any number of recorded-macro nodes alongside waits,
vision guards, actions, branches, repeats, chains, and stop nodes.

## User experience

- Remove the recorded-macro library from the Nodes left sidebar.
- Use the full Nodes area for the selected Loop canvas.
- Put a compact Loop selector in the canvas toolbar.
- Allow creating a Loop from the empty-canvas state and from the canvas
  right-click menu.
- Allow renaming the selected Loop from the toolbar.
- Keep recorded macros available only where they belong: as choices inside a
  Macro node.
- Label the existing control-flow loop node **Repeat** so it is not confused
  with the top-level Loop workspace.
- Save, validate, and run the selected Loop.

## Persistence and compatibility

Loop files remain under `macros/nodes/*.json`. Existing saved node graphs are
listed as Loops without destructive migration. The file stem is the durable
Loop identifier and the graph `name` is kept in sync when a Loop is created or
renamed.

Loop names are trimmed, stripped of Windows-invalid filename characters, and
made unique. Rename is atomic from the user's perspective: the graph is saved
under its new name before the old file is removed. Delete is available from the
toolbar and never deletes recorded macros.

## Runtime

The graph executor remains the authority for node ordering and cancellation.
A Loop starts at its Start node and follows edges through any number of Macro
nodes and other actions. Macro nodes keep embedded action snapshots so a Loop
does not silently change when its source recording is edited or removed.

## Error handling

- Invalid or empty names are rejected with a visible message.
- Save, rename, delete, validation, and run errors keep the current editor
  state intact.
- A missing selected Loop falls back to the next available Loop or the
  empty-canvas state.
- Existing graphs that fail validation remain loadable for repair but cannot
  run until fixed.

## Verification

- Backend tests cover list, create, rename, delete, uniqueness, and preservation
  of recorded macro files.
- Frontend tests cover no macro sidebar, empty-state right-click creation,
  Loop switching, rename, and Macro-node availability.
- Existing graph validation/execution tests prove multi-macro sequencing,
  waits, vision branches, repeats, cancellation, and embedded snapshots.
- Full Rust tests, frontend tests, TypeScript, production build, and release
  version consistency must pass before tagging `v1.2.0`.
