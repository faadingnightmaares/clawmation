# Loop Command Bar

## Goal

Make every Loop-level action immediately understandable while preserving the
maximum canvas area. Import and export must not depend on recognizing an edit
icon or opening an unrelated menu.

## Layout

The command bar uses three stable groups:

1. **Loop identity and files** on the left:
   - Loop selector;
   - New Loop;
   - Import;
   - Export;
   - More.
2. **Loop status** in the flexible middle:
   - node count;
   - connection count;
   - saved or unsaved state.
3. **Execution** on the right:
   - Run;
   - Save, with the existing primary gold treatment.

Every interactive control shares the existing 40-pixel height. Hairline
separators distinguish the groups without adding card containers. Icons remain
supporting cues; New Loop, Import, Export, More, Run, and Save keep visible text
labels.

## Command behavior

- **New Loop**, **Import**, and **Export** are direct commands.
- **More** contains only Rename and Delete, the less frequent management
  actions.
- Double-clicking the selected Loop name continues to begin rename.
- Export saves an unsaved Loop before opening the native save dialog.
- Existing busy, disabled, error, and confirmation behavior remains unchanged.

## Responsive behavior

The command bar remains one row. When horizontal space becomes limited, status
content disappears first. The Loop selector may shrink within a bounded range,
but New Loop, Import, Export, Run, and Save remain labeled and reachable.
Destructive actions stay in More at every width.

## File dialog terminology

The visible native file filters match their extensions exactly:

- `.clawbundle` uses **Clawbundle (.clawbundle)**;
- `.clawmation` uses **Clawmation (.clawmation)**.

Loop import and export continue to use `.clawbundle`. The terminology change
does not alter either archive format.

## Accessibility and motion

Controls use their visible text as their accessible name. Keyboard focus follows
the visual order from selector through execution actions. The redesign adds no
layout-shifting or delayed animation; hover and press feedback use the existing
compositor-safe styling.

## Verification

- Component tests confirm Import and Export are visible without opening More.
- Menu tests confirm More contains Rename and Delete only.
- Responsive assertions confirm status can collapse without hiding primary
  commands.
- Rust tests confirm Loop dialogs filter `.clawbundle` and macro dialogs filter
  `.clawmation`.
- TypeScript, frontend tests, Rust tests, formatting, and the production build
  must remain clean.
