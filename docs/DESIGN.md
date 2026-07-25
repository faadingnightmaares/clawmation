# Clawmation — Design System

The design contract. Read this before touching any view. It is the "warm dark
instrument" north star: a calm appliance a non-technical gamer leaves running,
not a cockpit of dials.

---

## Who it's for

Non-technical gamers who want a task automated and left alone — "record what I
did, play it back"; "when the reconnect button shows up, click it." They are not
engineers. They do not want telemetry, dials, or the vocabulary of the machine.
The competitor they compare us to (Natro Macro) looks, in its own fans' words,
**"like a plane cockpit."** Every screen must pass the opposite test: **legible
in five seconds, no grid of dials.**

---

## What the earlier design got wrong

The app went through several rejected redesigns before this one, all of which
restyled *inside* a frame that was itself the problem. Worth knowing, because the
same two traps are easy to fall back into:

- **The skin was a generated-UI cliché** — warm cream ground, serif display
  headings, a gold "eyebrow" hairline over every title, trailing-period headlines
  ("Watch and react."), dashed-border empty states. Individually defensible;
  together, instantly recognisable as machine-made.
- **The skeleton was the most conventional desktop shell there is** — left
  icon-rail, top status bar, stacked cream cards. Changing colours inside that
  frame changes nothing that matters.

What survived unchanged: plain copy, no telemetry, one action per row. Those got
re-housed, not redone.

---

## The one locked constraint

**Keep the three brand hues. Everything else is open** — value structure,
typography, skeleton, density, copy. The move that made it work was inverting the
value structure: warm ink became the dominant ground, parchment became the text,
and gold moved to the single live accent. Same three colours, completely
different skin.

### The three hues (kept) and the dark ramp derived from them
| Role | Value | Note |
|---|---|---|
| Ground (was text) | `#15120E` → `#1B1712` → `#221D17` → `#2B241C` | warm-ink ramp: page → bar → surface → raised |
| Foreground (was ground) | `#FAF8F5` parchment | text; dim `.60`, faint `.36` |
| Accent | `#C2A370` gold (+ `#D4B88A` warm) | **used sparingly**: run/live state, active nav, focus, one chip, the play glyph |
| Stop / destructive | `#CC6A52` warm red | the only additional hue; Stop + Delete only |

No cream card surfaces. No textured parchment ground. No grid. A single
barely-there warm radial gives depth — no gradients-as-decoration, no
glassmorphism, no neon-on-dark (all AI-"bold" traps, explicitly rejected).

### Type — Inter-forward, serif dropped
- **Sans** `Inter` — *everything.* Display titles at 700 / -0.022em tracking;
  section labels at 600 / uppercase / `.11em`; body at 400–500. The serif
  (Newsreader) is **retired** — serif display headings were a top AI-slop tell.
- **Mono** `Fira Code` — the rare true-data chip only (a hotkey like `Esc`).
- **Banned:** the gold eyebrow tick; trailing-period headlines; mono-allcaps as
  status voice; serif anywhere in the new language.

---

## The skeleton

- **No left nav-rail.** A slim **top command bar**: cat mark + "Clawmation"
  wordmark · a compact segmented switcher · run-state at the right.
- **Fewer top-level choices.** Four surfaces ride the switcher — **Home**,
  **Macros**, **Watch**, **Autopilot** — with **Settings** as an icon beside it.
  Guards and Chains fused into Autopilot (they are two halves of "run it without
  me") and the Guide moved inside Settings, so nothing hides behind a **More**
  menu any more: every surface is one click deep. (Seven similar automation nouns
  was residual cockpit at the very first click.)
- **No card-stack.** Content is a full-width focused surface: a calm header with
  the one primary action, then the list as **hairline rows on the dark ground**,
  not cream cards.
- **The run-state is the trust anchor**, top-right: idle shows a quiet "Ready";
  running shows a gold pulse + what's running + a warm-red **Stop** + its `Esc`
  hotkey. No FPS, no scan-rate, no event log — the cat overlay is the away-signal.

---

## The grammar: "calm appliance"

1. **One obvious verb per row.** Each macro row is name + plain meta + **Run**; a
   single **`⋯`** collapses rename / duplicate / playback speed / repeat / share /
   delete. No button spray, no second primary.
2. **Progressive disclosure.** Record-and-replay is the front door; the step
   editor, speed, and repeat live in the row's `⋯`, not on the surface.
3. **Plain language, always** — see the [Voice table](#voice--microcopy). Binding.
4. **No telemetry.** The cat overlay is the only run-state signal while you're
   away; after a run, at most one glanceable line — never a log.
5. **Speak in human units.** "Repeat: Until I stop," not "count = 0." "Playback
   speed: Slower · Normal · Faster," not "0.2×–25×." "Loose · Normal · Strict,"
   not a 0.0–1.0 threshold.
6. **The cat is the ownable signature** — in the command bar and the empty state,
   not repeated as row decoration (four dim cats in a list is itself an AI tell).

---

## Vision → the "Watch" surface, in one sentence

The whole feature is **"When ⟨this⟩ appears on screen, ⟨do that⟩."** Build the
surface as that sentence, not a detection-settings form.

- **Hide the detection method entirely.** Color vs. image vs. text (OCR) is an
  engine detail inferred from what the user picked — never a user choice, never a
  visible word. One rule type: *"watch for something."*
- **The core gesture is "show it what to watch for"** — pick a spot on screen,
  store a thumbnail, choose what to do.
- **Threshold defaulted and hidden.** Only if a rule misfires: *Loose · Normal ·
  Strict* — never a percentage, never the word "confidence."

---

## Voice / microcopy

| Say this (plain) | Not this (jargon) |
|---|---|
| "When the reconnect button appears, click it" | "Template match, confidence ≥ 0.85" |
| "Show it what to watch for" / "Pick it on screen" | "Capture template / define ROI" |
| "Watch for something" (one rule type) | "OCR / Color / Image detection mode" |
| "Not catching it? Loose · Normal · Strict" | "Min score / similarity threshold / HSV" |
| "Repeat: Until I stop" | "Loop count = 0 (infinite)" |
| "Playback speed: Slower · Normal · Faster" | "0.2×–25× speed multiplier" |
| "Record your first macro" (one button, first run) | "Add trigger / New sequence / Configure" |
| "Running · Sunflower field farm" (top bar) | Live feed / FPS / capture backend / event log |
| "Share this macro" / "Add a macro from a file" | "Export / Import JSON" |

---

## Where it lives in the code

Every surface is built in this language. `src/components/CommandBar.tsx` is the
top bar and run-state anchor, `src/nav.ts` decides which surfaces ride the
switcher and which sit beside it, `src/views/` are the pages, `src/index.css`
holds the token ramp above, and `src/components/ui/` is shadcn/ui with the tokens
applied — restyle there rather than per-view, so a change lands everywhere.

One recurring detail worth not re-breaking: a row's overflow menu must flip up
when the row sits low enough that the menu would run past the viewport bottom.

---

## What preservation means here

"Keep EXACT features" governs *capability*, not *presentation*. Infinite repeat
still exists — worded "Until I stop." The detection method still runs — inferred,
not asked. Removing the live feed removed a *readout*, not a capability; the
reassurance survives as one line. When in doubt: keep the power, change the words,
the value structure, and the frame.
