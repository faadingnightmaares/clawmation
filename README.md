<div align="center">
  <img src="public/Clawmation.svg" width="88" alt="Clawmation">
  <h1>Clawmation</h1>
  <p><em>Record what you did. Play it back. Let it watch the screen while you are away.</em></p>
  <p>
    <img alt="Platform: Windows 10 and 11" src="https://img.shields.io/badge/platform-Windows%2010%20%7C%2011-0a7cbd">
    <img alt="Built with Tauri 2, Rust and React 19" src="https://img.shields.io/badge/built%20with-Tauri%202%20%C2%B7%20Rust%20%C2%B7%20React%2019-c2410c">
    <img alt="License: MIT" src="https://img.shields.io/badge/license-MIT-444">
  </p>
</div>

Clawmation is a macro recorder and player for Windows games. You play through a task
once, every click, keypress and pause is captured, and it repeats that for as long as
you like.

On its own that is a stopwatch with hands. What makes it useful for long AFK grinds is
the part that watches the screen.

## What it does

**Macros.** Press record, do the thing, press stop. Every step is editable afterwards:
retime it, delete the fumble, add a keypress you forgot. Play it back once, a hundred
times, or forever.

**Watch.** Point Clawmation at something on screen (a colour, a picture of a button, or
some words), say what to do when that thing appears, and leave it running. No macro
required. Good for AFK checks, invite popups, and reward screens. If you snap a picture
of a button you can also mark the exact spot inside it to press, which matters when the
button is not a plain rectangle.

**Autopilot.** Two halves of running unattended, in one place.

*Guards* attach to a macro and poll a corner of the screen while it plays. The classic
setup is a farming loop set to infinite reps with a guard watching for the *Reconnect*
button: the moment it appears the guard pauses the macro, clicks it, waits for the game
to load, and resumes exactly where it left off, so the loop never dies overnight.

*Chains* play several macros back to back, optionally on a schedule, so an evening's
routine runs itself.

**You can watch it look.** Whenever a detection loop is running, a transparent overlay
draws a box around everything the triggers are currently finding, the way an object
detector's demo video does. A trigger matching the wrong thing used to look identical to
one matching nothing; now you can see which it is without guessing.

**A recording indicator that stays out of the way.** A small cat hangs from the top of
the screen while recording, counts down before playback starts, and is click through, so
it never steals a click from the game underneath.

## Privacy

Everything runs locally. There is no account and no telemetry. The app captures your
screen because that is the whole point, and none of what it captures ever leaves the
machine. The one network call it makes is the update check, which asks a release
manifest for a version number and nothing else.

## Requirements

- **Windows 10 or 11.** The hardware layer is Win32 specific throughout: DXGI Desktop
  Duplication for capture, `SendInput` for playback, low level hooks for recording, and
  `Windows.Media.Ocr` for text triggers. There is no cross platform path.
- **Rust** (stable) with the **MSVC** toolchain: `rustup default stable-msvc`, plus the
  Visual Studio Build Tools "Desktop development with C++" workload.
- **Node.js 20 or newer.**
- **WebView2**, preinstalled on Windows 11 and on up to date Windows 10.

There is deliberately no OpenCV, no Python, and no vcpkg step. See
[Vision without OpenCV](#vision-without-opencv) below.

## Getting started

```bash
npm install
```

```bash
npm run tauri dev
```

A debug build resolves its data root to the repository folder, so `config/`, `macros/`,
`templates/` and `snapshots/` appear here as you use it. They are gitignored, because
that is one developer's state rather than source. A release build keeps the same four
folders next to the `.exe`, which is what makes the installed app portable.

To produce installers (MSI and NSIS, written to `src-tauri/target/release/bundle/`):

```bash
npm run tauri build
```

The app updates itself: it checks a signed release manifest at launch and offers the new
version from Settings, under About. Publishing a build that installed copies will accept,
including the signing key you need to hold in order to do it, is documented in
[docs/RELEASING.md](docs/RELEASING.md).

## Tests

```bash
cargo test --manifest-path src-tauri/Cargo.toml
```

```bash
npm test && npx tsc --noEmit
```

That is 222 Rust tests and 36 frontend tests. Eleven more Rust tests are marked
`#[ignore]`: ten drive real hardware, moving the cursor, taking over the screen with a
fullscreen overlay, or reading the live desktop, and one is a hover timing benchmark that
only prints. Run them deliberately, one at a time, on a machine you are not otherwise
using:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib -- --ignored --test-threads=1
```

Note that `libtest` takes a single positional filter. Passing two test names matches
nothing and reports success.

## Layout

| Path | What lives there |
| --- | --- |
| `src/` | React 19 and TypeScript UI. `views/` holds the four surfaces plus Settings, `components/ui/` is shadcn/ui, and `api.ts` is the single typed seam over `invoke`. |
| `src-tauri/src/commands/` | The Tauri command surface, one thin module per feature area. Commands validate and delegate; they hold no logic. |
| `src-tauri/src/core.rs` | The wiring hub every command reaches through: config, runtime state, the capture and detector pair, the log buffer. |
| `src-tauri/src/engine/` | The long lived loops: guard engine, chain runner, scheduler, watch agent. |
| `src-tauri/src/hardware/` | Everything that touches the OS: capture, input synthesis, recording, OCR, the picker overlays, and `vision/`. |
| `src-tauri/src/models/` | Serde types for the on disk formats. These define the file compatibility contract. |
| `src-tauri/src/shell/` | The desktop furniture that runs without the UI asking: tray icon, global hotkeys, the recording indicator window, and the detection overlay. |
| `assets/` | The raster logo master. Regenerate the whole window, taskbar and tray icon set with `npm run tauri -- icon assets/Clawmation.png`. |
| `public/` | Static files the UI loads at runtime, including `Clawmation.svg`, the vector logo the app draws in its title bar. |
| `docs/` | [Architecture](docs/ARCHITECTURE.md), the [design contract](docs/DESIGN.md), the [release process](docs/RELEASING.md), and the notes behind the deliberate oddities in the port. |

New here? [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) is the map: the four layers, how
a detection actually flows, what lives on disk, and where to start reading for the kind
of change you have in mind.

## Vision without OpenCV

`src-tauri/src/hardware/vision/` is a hand transcription of the OpenCV calls this app
used to make through a Python sidecar: `cvtColor`, `inRange`, `morphologyEx`,
`findContours` (Suzuki and Abe), `resize`, `floodFill`, `CLAHE`, `Canny`, and masked
`matchTemplate` with `TM_CCOEFF_NORMED`. Only `image`, for decoding template files, and
`rayon`, for row parallel correlation, back it.

Two things are worth knowing before changing anything in there.

**It reproduces OpenCV's arithmetic, quirks included.** That covers the fixed point Canny
constants, `BORDER_REFLECT_101` edge handling, Python's `int()` truncation at every scale
step, and CLAHE padding a whole extra tile when a dimension already divides evenly. That
is not carelessness. The detection thresholds users have tuned against their own games
were derived from those exact numbers, and a cleaner reimplementation silently changes
what their guards fire on.

**Matching runs in tiers, and the first hit wins.** Tier 0 correlates at native scale in
a small window around wherever the template was last seen, which is what makes a guard
polling at 50 ms affordable. Tier 1 is a coarse to fine multi scale sweep on CLAHE
equalised pixels. Tier 2 repeats the sweep on Canny edges at a relaxed threshold. A
fourth ORB and FLANN tier existed in the Python version and was deliberately not ported:
reproducing OpenCV's hard coded rBRIEF sampling table has no derivation, and on flat game
UI it rarely found the six good matches it needed.

## Contributing

Contributions are welcome. Start with [CONTRIBUTING.md](CONTRIBUTING.md), which covers
the house style, the three test suites, and what a good pull request looks like here.

The short version: match the surrounding code, document *why* rather than *what*, and
lean on the type system instead of defensive branches. A `catch` and continue that hides
a real failure is treated as a bug being introduced, not as robustness.

If you are changing detection behaviour, say so explicitly in the description. Those
paths are load bearing for setups nobody in this repository can see.

## License

MIT. See [LICENSE](LICENSE).
