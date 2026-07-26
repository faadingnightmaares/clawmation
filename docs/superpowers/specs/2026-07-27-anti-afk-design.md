# Anti-AFK Design

## Goal

Add an Anti-AFK control to Home that lets the user select a running game or
application window, choose a repeat interval, and enable a lightweight jump.
When enabled, Clawmation immediately visits the selected window, presses Space
once, and restores the window the user was previously using. It repeats at the
selected interval while Clawmation remains running, including while its main
window is hidden.

## User Experience

Home gains one focused Anti-AFK panel below the primary action cards. The panel
contains:

- A selector populated from visible, titled top-level windows. Each option shows
  the window title and process ID so otherwise identical game instances can be
  distinguished.
- A refresh action for discovering windows opened after Home loaded.
- A 1–20 minute interval slider, defaulting to 15 minutes.
- An Anti-AFK toggle.
- A compact status message describing whether Anti-AFK is active, waiting for a
  target, or paused because the selected window is no longer available.

Enabling Anti-AFK requires a selected target. A successful enable performs the
first jump immediately and schedules later jumps from that completion time.
Changing the interval while enabled restarts the countdown with the new value.
Changing the target while enabled directs the next jump to the new target.

## Architecture

### Native window layer

A focused Windows-only hardware module owns these operations:

1. Enumerate visible, titled top-level windows, excluding Clawmation-owned
   windows.
2. Describe each window with its opaque handle, title, and process ID.
3. Check whether a stored handle still identifies a live window.
4. Capture the current foreground window.
5. Restore and focus the selected target.
6. Restore the previously foreground window.

The selected handle is session-scoped. It is not persisted across application
restarts because native window handles are recyclable. The enabled flag,
therefore, also starts false on each launch. The interval remains persisted in
`settings.yaml`.

### Anti-AFK service

`AppState` owns one background service. The service holds its small runtime
configuration behind a mutex and uses a condition variable to react immediately
to enable, disable, target, interval, and shutdown changes. It does not depend
on the React window or macro scheduler.

Each firing:

1. Revalidates the selected handle.
2. Records the current foreground handle.
3. Focuses the selected target.
4. Sends one Space key press through the existing `InputController`.
5. Restores the previous foreground handle when it is still valid and differs
   from the target.

Foreground restoration is implemented with a scope guard so it is attempted
even when focusing or input injection fails. The service serializes only its
own actions; it does not stop or mutate macro playback, recording, guards, or
vision features.

### Tauri API

The backend exposes three commands:

- `anti_afk_list_windows` returns selectable windows.
- `anti_afk_get` returns the live enabled, target, interval, and status state.
- `anti_afk_update` applies a partial target/interval/enabled update and returns
  the resulting state.

The React API wrapper mirrors these response types. Home loads the config and
window list independently so either failure can be shown without blocking the
rest of the page.

## Failure Handling

- Enabling without a target is rejected and the toggle remains off.
- If the target closes, Anti-AFK stays configured but pauses safely and reports
  that the window is unavailable. It never falls back to another same-titled
  instance.
- A failed focus, Space injection, or foreground restore is logged. The worker
  remains alive and retries at the next interval.
- Disabling wakes the worker immediately and prevents further jumps.
- The foreground window is restored only if it is still a valid window, avoiding
  activation of a stale or recycled handle.

## Testing

Rust unit tests cover:

- Window-list filtering and stable DTO formatting through pure helpers.
- Configuration validation, including enable-without-target rejection.
- Worker scheduling decisions for immediate first fire, interval changes,
  disabling, and missing targets through injected window/input operations.
- Restoration ordering: capture foreground, focus target, press Space, restore
  previous foreground.

React tests cover:

- Loading and rendering selectable windows.
- Keeping the toggle off when no target is selected.
- Persisting slider changes.
- Enabling sends the selected handle and performs the immediate backend action.
- Unavailable-target state is visible and recoverable through refresh.

The final verification runs the complete Vitest suite, TypeScript/Vite build,
Rust tests, and Rust formatting checks.

## Scope Boundaries

- This feature sends Space only; configurable keys or movement patterns are out
  of scope.
- It targets one selected window at a time.
- It does not launch games, reconnect closed instances, or identify accounts.
- It does not open a browser or require a network connection.
