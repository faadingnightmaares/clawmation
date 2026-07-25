# Contributing to Clawmation

Thanks for wanting to help. This file covers the things that are specific to this
repository, so a first pull request does not have to guess at them.

## Before you start

Clawmation only builds and runs on Windows 10 or 11. The whole hardware layer is Win32
specific, so there is no way to work on capture, input, recording or OCR from another
platform. UI work in `src/` is more portable in principle, but the app it belongs to is
not, so a Windows machine is the practical requirement.

You will need Rust (stable, MSVC toolchain) and Node.js 20 or newer. The
[README](README.md) has the full setup, and `npm run tauri dev` is the loop you will
spend your time in.

## Running the checks

Three suites, all of which should be green before you open a pull request.

```bash
cargo test --manifest-path src-tauri/Cargo.toml
```

```bash
npm test
```

```bash
npx tsc --noEmit
```

Eleven Rust tests carry `#[ignore]`. Ten drive real hardware: they move the cursor, take
over the screen with a fullscreen overlay, or read the live desktop. The eleventh is a
hover timing benchmark that prints rather than asserting. They are not part of the
default run, and they should stay that way. If you touch anything in
`src-tauri/src/hardware/`, run them by hand on a machine you are not otherwise using:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib -- --ignored --test-threads=1
```

Please also make sure `cargo build --release` finishes without warnings.

## House style

**Match the surrounding code.** Naming, error handling, comment density, abstraction
level. A change should read as though the original author wrote it.

**Comments say why, not what.** They exist for constraints the code cannot express:
invariants, non obvious reasoning, contracts with the OS or with a file format. A comment
that narrates the next line, or that addresses the reviewer, gets deleted in review.

**Lean on the type system rather than on defensive branches.** A `catch` and continue, or
a fallback default, that hides a real failure is treated as a bug being introduced rather
than as robustness. If something cannot happen, make it unrepresentable; if it can, let
it surface.

**No speculative flexibility.** No configuration options, abstractions or fallback paths
for needs nobody has stated yet.

**Prose in this repository does not use em dashes or en dashes.** Use a comma, a colon, a
semicolon, brackets, or a second sentence. This applies to code comments, UI copy, docs
and commit messages alike.

## Things that need extra care

**Detection behaviour.** `src-tauri/src/hardware/vision/` deliberately reproduces
OpenCV's arithmetic, quirks included, because the thresholds people have tuned against
their own games were derived from those exact numbers. A tidier reimplementation silently
changes what their guards fire on. If your change alters what matches, say so explicitly
in the pull request description and explain the trade.

**On disk formats.** The serde types in `src-tauri/src/models/` define the compatibility
contract for macros, guards, chains and config that users already have saved. Adding an
optional field with a default is fine. Renaming or removing one is a breaking change and
needs a migration.

**The design contract.** [docs/DESIGN.md](docs/DESIGN.md) governs the UI. It is not a
style suggestion; it settles questions like which words the interface is allowed to use
and how much a screen is allowed to ask before it does anything useful. Read it before
adding a control.

## Pull requests

- One concern per pull request. A bug fix and a refactor in the same diff are hard to
  review and harder to revert.
- Describe what changed and why. If there is a reproduction, include it.
- No drive by renames, formatting sweeps or unrelated improvements. If you spot something
  worth fixing, mention it in the description or open an issue.
- Commit messages are formal and to the point. An imperative subject line under about
  seventy characters, then a body explaining the reasoning if the subject cannot carry it
  alone.

## Reporting a bug

Open an issue with your Windows version, the Clawmation version from Settings under
About, what you expected, and what happened instead. For detection problems, the single
most useful thing you can attach is the template image or the trigger settings, plus a
screenshot of the game at the moment it should have matched.

## Security

If you find something with security impact, please do not open a public issue. Report it
privately through GitHub's security advisory form on this repository instead.
