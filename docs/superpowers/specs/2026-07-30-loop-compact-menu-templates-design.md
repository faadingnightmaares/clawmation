# Compact Loop Creation and Templates

## Goal

Make node creation consistent and give first-time users a working example they
can inspect, edit, and run.

## Unified right-click menu

Canvas and node right-click use the same compact menu, row styling, icons,
spacing, categories, and keyboard focus treatment.

- Canvas right-click adds an unconnected node at the pointer.
- Node right-click adds a node on the source node's primary continuation.
- If that continuation is already connected, the new node is inserted between
  the source and its current target.
- Start is hidden when adding after a node because a Loop has one entry point.
- New Loop and Templates remain Loop-level actions and appear only on the
  canvas menu.
- Dragging an output into empty space keeps the searchable autocomplete for
  fast keyboard-heavy authoring.

## Templates

The canvas menu has a visible Templates action that opens a compact adjacent
picker. Templates create a new Loop and never overwrite the current Loop.

The initial templates are:

- **Learn Loops:** an editable, runnable example containing Start, Wait,
  Branch, successful and failed Stop outcomes, plus instructional Note nodes.
- **Basic Sequence:** a minimal runnable Start, Wait, Stop workflow.

Template creation first reserves a collision-safe Loop name through the
existing backend, then saves the selected graph under the returned name. If
the template save fails, the reserved blank Loop is removed so users are not
left with a partial result.

## Verification

- Node right-click opens the same compact menu as canvas right-click.
- Adding from a node connects through its primary continuation.
- The Templates picker delegates the selected template identifier.
- Template graphs have valid edge references and pass client validation.
- Creating a template Loop saves and selects the generated graph.
