<div align="center">
  <img src="public/Clawmation.svg" width="72" alt="">
  <h1>Clawmation</h1>
  <p><em>Record what you did. Play it back. Let it watch the screen while you're away.</em></p>
</div>

Clawmation is a macro recorder and player for Windows games. You play through a task
once — every click, keypress and pause is captured — and it repeats that for as long
as you like. On its own that is a stopwatch with hands; what makes it useful for
long AFK grinds is the part that watches the screen:

- **Guards** attach to a macro and poll a corner of the screen while it runs. The
  classic setup is a farming loop set to ∞ reps with a guard watching for the
  *Reconnect* button: the moment it appears the guard pauses the macro, clicks it,
  waits for the game to load, and resumes exactly where it left off — so the loop
  never dies overnight.
- **Watch** does the same thing without a macro. Point it at a colour, a picture of a
  button, or some on-screen text, say what to do when that appears, and leave it
  running. Good for AFK checks, invite popups, and reward screens.
- **Chains** play several macros back to back, optionally on a schedule.

Everything runs locally. There is no account, no telemetry, and no network call at
runtime — the app captures your screen because that is the whole point, and none of
what it captures leaves the machine.

## Requirements

- **Windows 10 or 11.** The hardware layer is Win32-specific throughout: DXGI Desktop
  Duplication for capture, `SendInput` for playback, low-level hooks for recording,
  and `Windows.Media.Ocr` for text triggers. There is no cross-platform path.
- **Rust** (stable) and the **MSVC** toolchain — `rustup default stable-msvc` plus the
  Visual Studio Build Tools "Desktop development with C++" workload.
- **Node.js 20+**.
- **WebView2** — preinstalled on Windows 11 and on up-to-date Windows 10.

There is deliberately no OpenCV, no Python, and no vcpkg step. See
[Vision](#vision-without-opencv) below.

## Getting started

```bash
npm install
```

```bash
npm run tauri dev
```

A debug build resolves its data root to the repository folder, so `config/`,
`macros/`, `templates/` and `snapshots/` appear here as you use it. They are
gitignored — that is one developer's state, not source. A release build keeps the
same four folders next to the `.exe`, which makes the installed app portable.

To produce installers (MSI and NSIS, in `src-tauri/target/release/bundle/`):

```bash
npm run tauri build
```

## Tests

```bash
cargo test --manifest-path src-tauri/Cargo.toml
```

```bash
npm test && npx tsc --noEmit
```

Ten Rust tests are `#[ignore]`d because they drive real hardware — they move the
cursor, take over the screen with a fullscreen overlay, or read the live desktop.
Run them deliberately, one at a time, on a machine you are not using:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib live_ -- --ignored --test-threads=1
```

Note that `libtest` takes a single positional filter; passing two test names matches
nothing and reports success.

## Layout

| Path | What lives there |
| --- | --- |
| `src/` | React 19 + TypeScript UI. `views/` are the seven pages, `components/ui/` is shadcn/ui, `api.ts` is the single typed seam over `invoke`. |
| `src-tauri/src/commands/` | The Tauri command surface — one thin module per feature area. Commands validate and delegate; they hold no logic. |
| `src-tauri/src/core.rs` | The wiring hub every command reaches through: config, runtime state, the capture/detector pair, the log buffer. |
| `src-tauri/src/engine/` | The long-lived loops — guard engine, chain runner, scheduler, watch agent. |
| `src-tauri/src/hardware/` | Everything that touches the OS: capture, input synthesis, recording, OCR, the picker overlays, and `vision/`. |
| `src-tauri/src/models/` | Serde types for the on-disk formats. These define the file compatibility contract. |
| `src-tauri/src/shell/` | Tray icon, global hotkeys, and the recording-indicator window. |
| `docs/` | Design contract and the notes behind the deliberate oddities in the port. |

## Vision without OpenCV

`src-tauri/src/hardware/vision/` is a hand transcription of the OpenCV calls this app
used to make through a Python sidecar: `cvtColor`, `inRange`, `morphologyEx`,
`findContours` (Suzuki–Abe), `resize`, `floodFill`, `CLAHE`, `Canny`, and masked
`matchTemplate` with `TM_CCOEFF_NORMED`. Only `image` (for decoding template files)
and `rayon` (row-parallel correlation) back it.

Two things are worth knowing before changing anything in there.

**It reproduces OpenCV's arithmetic, quirks included** — the fixed-point Canny
constants, `BORDER_REFLECT_101` edge handling, Python's `int()` truncation at every
scale step, and CLAHE padding a whole extra tile when a dimension already divides
evenly. That is not carelessness: the detection thresholds users have tuned against
their own games were derived from those exact numbers, and a "cleaner" reimplementation
silently changes what their guards fire on.

**Matching runs three tiers, first hit wins.** Tier 0 correlates at native scale in a
±60 px window around wherever the template was last seen, which is what makes a guard
polling at 50 ms affordable; tier 1 is a coarse-to-fine multi-scale sweep on
CLAHE-equalised pixels; tier 2 repeats the sweep on Canny edges at a relaxed
threshold. A fourth ORB/FLANN tier existed in the Python and was deliberately not
ported — reproducing OpenCV's hard-coded rBRIEF sampling table has no derivation, and
on flat game UI it rarely found the six good matches it needed.

## Contributing

Match the surrounding code. The Rust side documents *why*, not *what*, and leans on
the type system rather than defensive branches; a `catch`-and-continue that hides a
real failure is treated as a bug being introduced, not robustness. Before opening a
pull request, run the three suites above and make sure `cargo build --release` is
warning-free.

If you are changing detection behaviour, say so explicitly in the description. Those
paths are load-bearing for setups we cannot see.

## License

MIT — see [LICENSE](LICENSE).
