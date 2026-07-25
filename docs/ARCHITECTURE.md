# Architecture

A map of the codebase for someone who has just cloned it. The
[README](../README.md) says what Clawmation does; this says how it is put together
and where to start reading.

## The shape of it

Clawmation is a Tauri 2 desktop app. A React frontend renders the window, a Rust
backend does everything else, and the two talk over Tauri's `invoke` bridge and an
event channel. There is no server, no database and no IPC to a second process. The
whole thing is one executable plus four data folders.

```
 React UI  ──invoke──▶  commands/  ──▶  core.rs  ──▶  engine/  ──▶  hardware/  ──▶  Win32
    ▲                                      │
    └──────────────── events ──────────────┘
```

Four layers, and the rule is that each one only reaches down.

**`src-tauri/src/commands/`** is the API surface, one thin module per feature area.
A command validates its arguments, delegates, and turns the result into JSON. It
holds no logic of its own. If you are looking for what a button does, start here and
follow the call.

**`src-tauri/src/core.rs`** is the wiring hub. `Core` owns one instance of each
long lived thing (config, log buffer, runtime state, player, input controller,
recorder, vision, guard engine, notifier, tray indicator, detection overlay) behind
`Arc`s, and every command reaches through it. It also carries the playback logic
that does not belong to any single engine.

**`src-tauri/src/engine/`** holds the stateful loops, and they are deliberately
hardware free. Each engine takes its side effects as injected closures, which is why
they can be unit tested without moving a real cursor or reading a real screen.

**`src-tauri/src/hardware/`** is everything that touches the operating system, and
the only layer that is allowed to. Capture, input synthesis, recording hooks, OCR,
the fullscreen picker overlays, and the vision module.

Two support modules sit outside the stack. `src-tauri/src/models/` holds the serde
types for the on disk formats, which are the compatibility contract with data users
already have saved. `src-tauri/src/shell/` holds the desktop furniture the backend
starts on its own: the tray icon, the global hotkeys, the recording indicator window,
and the live detection overlay.

## The engines

**`engine/guards.rs`, `GuardEngine`.** Runs alongside a playing macro. It polls the
screen for each of the macro's guards, and when one matches it pauses playback,
performs the guard's action, waits out the resume delay, and lets the macro carry on
from exactly where it stopped. This is what keeps an overnight farming loop alive
through a disconnect dialog.

**`engine/vision_agent.rs`, `VisionAgent`.** The standalone Watch loop, with no macro
involved. It shares the entire detection half with `GuardEngine`, and differs only in
what it does on a hit. It duty cycles: after each pass it idles in proportion to how
long that pass cost, which holds the CPU cost of watching down to roughly a fifth of
a core rather than pinning one.

**`engine/chains.rs`.** Plays several macros back to back.

**`engine/scheduler.rs`.** Fires a macro or a chain on an interval or at a set time.

**`engine/stats.rs`.** A thread safe wrapper over `config/stats.json`.

**`engine/ai.rs`.** The executor behind the per macro step editor.

### Why the two detection surfaces share their geometry

Both engines call `Vision::detect_guards_faithful`, so what they *see* is identical
by construction. What they *do* with a hit used to differ, and that was a bug: the
Watch surface is the only one that offers "snap it and mark where to click", which
writes a click offset or a drawn stroke into the trigger, and the agent was ignoring
those and pressing the middle of the captured picture instead. For anything that is
not a plain rectangular button, the middle is frequently not on the target at all.

Both now route through the same public helpers in `engine/guards.rs`: `plan_action`
resolves a match plus a trigger into a `Plan`, either a click at a point or a drag
along a set of strokes, and `stroke_points` expands a stroke into the cursor path.
If you add a third surface that acts on a detection, use those rather than reaching
for the match centre.

## Detection

`src-tauri/src/hardware/vision/` is a hand transcription of the OpenCV calls this app
used to make through a Python sidecar. It has no OpenCV, no Python and no vcpkg step
behind it; only `image` for decoding template files and `rayon` for row parallel
correlation.

A detection pass goes: grab a frame from `hardware/capture.rs`, crop it to the
trigger's region, then dispatch on the trigger's method.

- **Colour** converts to HSV, thresholds with an `inRange` equivalent, cleans up with
  a morphological open and close, and takes contours above a minimum area.
- **Image** runs the tiered template matcher described below.
- **Text** hands the crop to `Windows.Media.Ocr` and looks for the words.

The image matcher runs in tiers and the first hit wins. Tier 0 correlates at native
scale in a small window around wherever this template was last seen, which is what
makes a guard polling at 50 ms affordable. Tier 1 is a coarse to fine multi scale
sweep on CLAHE equalised pixels. Tier 2 repeats that sweep on Canny edges at a
relaxed threshold, which is what rescues a button whose colour has shifted under a
different game theme.

The module reproduces OpenCV's arithmetic including its quirks: the fixed point Canny
constants, `BORDER_REFLECT_101` edge handling, integer truncation at every scale step,
and CLAHE padding an extra tile when a dimension already divides evenly. That is
deliberate. The thresholds people have tuned against their own games came from those
exact numbers, so a tidier reimplementation silently changes what their guards fire
on. Treat any change in there as a behaviour change, not a cleanup.

## The frontend

`src/api.ts` is the single typed seam over `invoke`. Every backend call in the app
goes through a function there, and nothing else imports `@tauri-apps/api/core`. If
you add a command, add its wrapper and its result type in the same commit.

`src/views/` holds four surfaces plus Settings, and `src/nav.ts` is the routing
contract they switch on. Guards and chains are two halves of running unattended, so
they share the Autopilot surface; the guide reads as reference material, so it sits
inside Settings. Keeping the count that low is what lets every surface live in the
command bar with nothing hidden behind a More menu.

`src/lib/triggers.ts` is the single source of truth for converting between a stored
guard and the editor's working draft, in both directions. `Guard` carries a string
index signature, so a mistyped field name assigns cleanly and silently does nothing
on save, which the type checker cannot catch. `triggers.test.ts` round trips every
field to prove the converters do not drop one.

`src/components/editors/TriggerEditor.tsx` is the one editor behind both the Watch
sheet and the Autopilot guards sheet. It never names a detection method: whether a
trigger is colour, image or text is inferred from what the user showed it, which is
a rule from [DESIGN.md](DESIGN.md) rather than a preference.

## Data on disk

Four folders. In a debug build they resolve to the repository root, and in a release
build they sit next to the executable, which is what makes the installed app
portable.

| Folder | Contents |
| --- | --- |
| `config/` | Settings, statistics, schedules. |
| `macros/` | One JSON file per macro. `macros/guards/` holds per macro guards, and `macros/guards/_vision.json` holds the standalone Watch triggers. |
| `templates/` | Captured button images that image triggers match against. |
| `snapshots/` | Test screenshots and picker thumbnails. |

The serde types in `src-tauri/src/models/` define these formats. Adding an optional
field with a default is safe. Renaming or removing one breaks files people already
have, and needs a migration.

## Threading

Playback, recording, each engine loop and the OCR call all run on their own threads.
Shared state is `Arc<Mutex<...>>` on `Core`, and the rule that keeps the UI honest is
that no command holds a lock across a slow operation. `vision_start` opens the
capture backend before it locks the agent slot for exactly this reason: holding that
lock across a cold backend start wedges the stop command and every status poll behind
it, which the user experiences as a Start button stuck spinning forever.

`vision_status` goes further and uses `try_lock`, answering "ask again in a moment"
rather than queueing, because the Watch view polls it once a second for as long as it
is open.

## Where to start reading

- **A UI change**: the view in `src/views/`, then [DESIGN.md](DESIGN.md).
- **A new backend capability**: the matching module in `src-tauri/src/commands/`,
  then whatever it delegates to.
- **Detection behaviour**: `src-tauri/src/hardware/vision/mod.rs` for the dispatch,
  then `template.rs` for the matcher.
- **Why something is written the strange way it is**:
  [MIGRATION-NOTES.md](MIGRATION-NOTES.md), which records the deliberate oddities
  carried over from the Python version this was ported from.
