# Loops Pro Canvas Authoring

## Goal

Make Loops fast enough for a first-time user and powerful enough for complex
automation. Users should build a reliable flow without manually planning wire
geometry, guessing screen coordinates, or understanding execution-engine
failure semantics.

The selected direction is a polished **Pro Canvas**. Clawmation keeps the
freedom of a node graph while adding the guided creation and cleanup behavior
normally associated with code completion.

## Research basis

The interaction model combines proven patterns from established node editors:

- [Unreal Blueprints](https://dev.epicgames.com/documentation/en-us/unreal-engine/blueprint-editor-cheat-sheet-in-unreal-engine)
  opens a context-filtered action menu when a pin is dragged into empty graph
  space and emphasizes connected wires while hovering a pin.
- [Node-RED](https://nodered.org/docs/user-guide/editor/workspace/wires)
  supports quick-add-and-connect, dropping a node onto a wire to insert it,
  moving existing wires, and deleting a node while reconnecting its flow.
- [Blender](https://docs.blender.org/manual/en/latest/interface/controls/nodes/editing.html)
  auto-inserts nodes into links, offsets surrounding nodes, supports reroutes
  and link cutting, and collapses unused detail.

Clawmation adopts these ideas using its existing visual language rather than
copying another product's appearance.

## Scope

This release covers five connected authoring problems:

1. context-aware node autocomplete;
2. readable and editable wires;
3. simplified success and failure flow;
4. guided Repeat construction;
5. direct on-screen Click targeting.

It does not add a separate linear editor, replace the existing graph file
format, or remove manual node placement.

## Context-aware autocomplete

Dragging from an output port and releasing on empty canvas opens a command
palette at the release point. The palette is filtered to nodes that can accept
that connection. It supports:

- immediate keyboard focus;
- fuzzy matching with exact and prefix matches ranked first;
- categories and short descriptions;
- arrow-key navigation, Enter to create, and Escape to cancel;
- recent choices below matching results;
- the same palette from a small `+` affordance beside a hovered or
  keyboard-focused output.

Choosing a result creates the node at the release point and connects it in one
operation. The operation produces one undo checkpoint.

Right-click continues to open the full unfiltered node menu.

## Wire workflow

### Default rendering

Edges use a rounded orthogonal route with a short horizontal exit and entry.
This produces predictable lanes and makes direction readable without filling
the canvas with large labels. Outcome labels remain beside their source ports
rather than floating over unrelated wires.

### Focus

Hovering or keyboard-focusing a node, port, or edge:

- keeps its directly connected path at full opacity;
- fades unrelated edges;
- increases the connected edge hit area and contrast;
- shows its direction and endpoint labels.

Moving the pointer away restores the complete graph without a delayed layout
change.

### Editing

- Dropping a compatible single-input node onto an edge inserts it between the
  two existing nodes and reconnects both sides.
- Double-clicking an edge adds a movable reroute point.
- Reroute points persist with the edge and may be moved or deleted.
- Selecting an edge and pressing Delete removes it.
- Deleting a node offers **Delete** and **Delete & reconnect** when the node has
  one incoming and one normal continuation.
- New nodes use local auto-offset so they do not cover an existing node. Manual
  positions are never rearranged unless the user runs Arrange.

## Outcome model

Node ports use contextual language rather than the generic “If works” and
“If fails” pair:

| Node | Primary path | Optional recovery path |
| --- | --- | --- |
| Click, Key, Type, Scroll, Wait | Continue | On failure |
| Find & Click, Wait for image | Found | Not found |
| Macro, Chain | Continue | On failure |
| Branch | Matches | Otherwise |
| Repeat | Do | Then |

Normal actions expose only the primary port by default. If an action fails
without a recovery path, the Loop stops, the failed node is highlighted, and
the run result explains the failure.

The inspector has an **On failure** control:

- **Stop Loop** — default, no secondary port;
- **Continue** — failure follows the primary path;
- **Recovery path** — reveals the secondary port.

An existing saved secondary edge automatically selects **Recovery path**, so
all current Loops load without losing behavior. Removing the last secondary
edge leaves the recovery mode enabled until the user changes it, preventing an
accidental behavior change.

Vision and Branch nodes keep both outcomes visible because both results are
normal decisions rather than exceptional failures.

## Repeat workflow

Repeat keeps the current runtime semantics but removes the confusing visible
loop-back wire.

### Creation

Adding Repeat opens a compact setup panel with:

- **Times** presets: 2, 3, 5, custom;
- **Forever**, represented by the existing count value `0`;
- **Wrap selected nodes**, when the current selection forms one continuous
  chain.

The node displays `Repeat 3 times` or `Repeat forever`, with **Do** and **Then**
ports.

### Body construction

Dragging from **Do** uses autocomplete to create the first body node. When the
body has one terminal node, Clawmation maintains the return edge automatically.
The return is rendered as a quiet loop rail hugging the Repeat group instead of
crossing the graph.

When selected nodes are wrapped, Clawmation creates the Repeat node, connects
the incoming flow to it, connects **Do** to the first selected node, connects
the last selected node back to Repeat, and connects **Then** to the former
continuation. The operation is atomic and undoable.

Existing Repeat graphs are recognized by their edge returning to the Repeat
input and gain the compact loop-rail rendering automatically. A Repeat missing
its return shows one clear **Complete Repeat** repair action.

Flows with branching Repeat bodies retain manual wiring. Clawmation does not
guess which branch should return.

## Click target picker

Click nodes replace coordinate guessing with a primary **Pick on screen**
button.

The picker:

1. hides Clawmation without changing the active target application's layout;
2. shows a lightweight crosshair and magnified pixel preview;
3. captures one global desktop point on left click;
4. cancels on Escape or right click;
5. restores Clawmation and writes the physical screen coordinates.

The inspector shows the chosen coordinates, monitor, and a small target
preview. X and Y remain editable for precise adjustments and backward
compatibility.

Capture must account for mixed-DPI monitors and negative coordinates on
monitors positioned left or above the primary display.

## Data compatibility

Existing nodes and edges remain valid.

The edge model adds an optional `waypoints` array for reroute positions. Its
absence means no reroutes. Repeat loop rails are a rendering treatment of
existing edges, not a new runtime edge type.

Action recovery mode is inferred from existing edges when absent. New saves may
store an explicit `failure_mode` in node configuration, but older versions
ignore it safely.

Click coordinates remain the existing `Step.x` and `Step.y` fields.

## Error handling

Autocomplete creation is all-or-nothing. A failed create or connect operation
leaves the graph untouched and keeps the palette open with a short error.

Screen picking never changes coordinates until capture succeeds. Cancellation
is silent. Capture errors remain in the inspector and preserve the previous
point.

Auto-insert and Delete & reconnect refuse ambiguous multi-input or multi-output
cases and explain why rather than guessing.

## Performance and accessibility

Wire focus changes only opacity, stroke, and transform-safe decoration. It does
not rebuild the graph layout on pointer movement. Route calculation is memoized
per edge and invalidated only by endpoint or waypoint changes.

The command palette, ports, reroute points, Repeat setup, and picker controls
have keyboard access and visible focus. Reduced-motion mode removes decorative
transitions without removing state feedback.

## Verification

- Autocomplete filtering, keyboard navigation, create-and-connect, cancellation,
  and undo tests.
- Auto-insert, Delete & reconnect, reroute persistence, and edge-focus tests.
- Compatibility tests for existing success/error, found/missing, branch, and
  Repeat graphs.
- Repeat wrap, automatic return maintenance, infinite count, repair, and undo
  tests.
- Mixed-DPI and negative-coordinate picker math tests plus cancellation tests.
- Engine tests proving an unhandled action failure stops and a configured
  recovery edge runs.
- Rapid graph interaction test confirming focus effects do not cause repeated
  layout work.
- Full frontend tests, Rust tests, TypeScript, formatting, and production build.
