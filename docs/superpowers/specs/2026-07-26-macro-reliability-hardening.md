# Macro Reliability Hardening

Date: 2026-07-26

## Problem

A correctly recorded macro can keep repeating after the game has diverged, lose
the idle time at an old loop boundary, or appear to complete even when Windows
did not accept every injected input. That combination can turn one transient
miss into many failed runs.

The observed implementation has four independent failure paths:

1. Macros recorded before 1.1.6 have no terminal `WAIT`, so an idle gap after
   the last action is absent and looping restarts early.
2. `SendInput` and `SetCursorPos` return values are discarded, so the player
   cannot distinguish delivered input from a blocked or partial injection.
3. Checkpoint timeouts silently continue, allowing a loop to proceed after the
   expected screen state did not appear.
4. A stopped or panicked player can be presented and stored as "completed".

The user's friend's file is not on this machine. The hardening therefore targets
the provable engine-level failure modes and makes future divergence fail closed.

## Safety Boundary

No input-only macro can mathematically guarantee a game result. Windows may
block injection, the game may lag, focus may change, and game state can be
different from the recording. Clawmation can guarantee the safer property:

> When delivery or an explicitly required visual condition cannot be verified,
> stop the macro before another repetition and report the exact reason.

A required end-of-run visual checkpoint is the result gate. Without one,
"completed" means the recorded sequence was delivered, not that the game was
won.

## Design

### Versioned macro files and one-time migration

- Add `format_version` and `recording_duration` to the macro schema.
- New recordings use format version 2 and retain their exact active duration.
- Missing `format_version` is read as legacy version 1.
- At application startup, inspect top-level macro JSON files.
- Before changing a legacy file, create a non-JSON `.pre-v2.bak` copy next to it.
- For a legacy repeating macro whose last event is not already a `WAIT` or
  checkpoint, append a conservative ten-second terminal `WAIT`. This restores
  the known old-recorder loss window without pretending the unknowable original
  duration can be reconstructed exactly.
- Upgrade checkpoint timeout policy to fail closed.
- Write atomically and make the migration idempotent.
- Never modify a malformed file; retain it and log the parse error.

### Playback validation

Reject before playback when:

- the format is newer than this app supports;
- the recording or target resolution is zero;
- the event list is empty or unreasonably large;
- any timestamp is non-finite, negative, or moves backwards;
- an input event uses an unknown key or mouse button;
- checkpoint timing values are invalid.

Validation errors name the event and field so users can repair or re-record the
macro instead of debugging a silent miss.

### Observable input delivery

- Compare the `SendInput` return count with the requested count.
- Retry only the unsent suffix, avoiding duplicate down edges after a partial
  batch.
- Capture the native error code for diagnostics.
- Check every `SetCursorPos` result used by playback.
- Treat exhausted delivery attempts as a playback failure.

### Fail-closed player lifecycle

- Return an explicit `Completed`, `Stopped`, or `Failed(reason)` outcome from the
  player thread.
- Convert panics into a failed outcome instead of a successful completion.
- Track keys and mouse buttons held by the macro and release them on every exit
  path.
- Do not emit a success toast for a manual stop or failure.
- Record `completed`, `stopped`, and `failed` honestly in play history.
- If playback falls far enough behind that a critical click/key would be sent as
  part of a catch-up burst, stop rather than corrupt the remaining sequence.

### Checkpoints

- `wait_for` checkpoints default to `on_timeout: "stop"`.
- Legacy checkpoint configs are migrated to the same safe policy.
- The editor exposes "Stop macro" and "Continue anyway", with stop recommended.
- A required checkpoint that times out ends playback and prevents the next loop.

## Verification

- Unit tests for legacy migration, backups, malformed-file preservation, and
  idempotence.
- Unit tests for timeline validation and unsupported input values.
- Unit tests for full, partial, retried, and exhausted native delivery.
- Unit tests for checkpoint timeout policy and outcome mapping.
- Existing Rust and frontend suites remain green.
- Hardware-moving tests remain opt-in; normal CI must not move the user's mouse.

## Non-goals

- Fabricating a success image or target condition for old macros.
- Editing the standalone `fortress/` project.
