# Reliable Vision Actions

## Goal

Make every autonomous action triggered by Vision use one delivery path in Watch
and Loops. A successful result means Clawmation established the correct target
window, delivered a complete input gesture, and left no key or mouse button
held. Detection quality and matching behavior remain unchanged.

## Confirmed failure

The released paths stop at intent rather than reliable delivery:

- Watch focuses the detected window, but sends mouse/key down and up
  back-to-back. A frame-polled game can miss the pressed state even when Windows
  accepts both events.
- Loops uses the generic AI actuator. It does not establish the detected
  window as foreground and discards native input errors.
- The two paths can therefore report success after targeting different windows
  or after an action the game never sampled.
- Existing tests verify emitted action enums, not repeated native delivery,
  focus loss, partial sends, cursor drift, or release recovery.
- Field testing confirmed that Roblox can visually move the cursor after an
  absolute placement without updating its Raw Input hover state. A tiny
  physical mouse movement immediately makes the waiting click register.

## Chosen design

Introduce one serialized reliable-action executor used by Watch and Loops.
Click, key, and nudge requests carry the detected screen point so the executor
can resolve the intended external window.

For every request, the executor:

1. Acquires a process-wide autonomous-input lock.
2. Resolves the external top-level window beneath the detected point, excluding
   Clawmation's own capture-excluded windows.
3. Rejects a target running at a higher Windows integrity level with a clear
   administrator message.
4. Brings the target forward and confirms it is the actual foreground window.
5. Moves to the requested physical pixel and confirms the cursor landed within
   one pixel.
6. Arms Raw Input with a non-coalesced two-pixel relative move, waits one frame,
   returns by the same relative delta, and resynchronizes to the exact detected
   pixel. This preserves the target coordinate while forcing Roblox to observe
   real mouse motion.
7. Gives the foreground and hover state time to reach the target's next rendered
   frame.
8. Sends a complete gesture with a frame-spanning hold:
   - click: mouse down, hold, mouse up;
   - key: key down, hold, key up;
   - nudge: relative movement out, frame dwell, relative movement back.
8. Guarantees a best-effort release if any post-press operation fails.

Focus and cursor-establishment failures are retried with bounded backoff before
any press is sent. Native sends retain their existing partial-send retry.
Clawmation will not blindly repeat an accepted complete click because Windows
cannot prove whether the target applied it, and repeating could double-submit a
purchase or destructive action.

## Integration

- Watch dispatches click, key, and nudge directly through the reliable executor;
  it no longer composes focus and input as separate independently-successful
  actions.
- Loops routes vision-driven clicks and ordinary click/key action nodes through
  the same executor. Coordinate-bearing actions resolve their target by point.
  Key-only nodes retain the last successfully established Loop target for the
  current run; without one they fail clearly instead of typing into Clawmation.
- The AI/Loop actuator returns `Result` so action delivery failures mark the node
  or step failed and appear in the run summary.
- Watch increments its fired count and starts cooldown only after the reliable
  executor returns success. A rejected action stays armed for the next scan.
- Recorded macro playback is unchanged. Its timing remains the user's recorded
  timing and does not receive autonomous frame holds.

## Error and cancellation behavior

- No action is reported as successful when focus, cursor placement, integrity,
  or native input delivery fails.
- Every accepted down event is paired with a release attempt, including errors
  and stop requests.
- Retry waits are short and bounded. Stop remains responsive and prevents a new
  transaction from starting.
- Error messages name the failed phase and target window so field reports are
  actionable.

## Verification

Add deterministic tests around injected focus, cursor, send, and wait seams:

- hundreds of consecutive Watch and Loop actions preserve ordering;
- concurrent detections cannot interleave gestures;
- focus refusal retries before pressing, then surfaces failure;
- cursor drift is corrected before pressing;
- a Roblox-like target that rejects clicks until relative motion occurs receives
  `absolute move -> relative out -> frame dwell -> relative back -> exact
  resync -> hover settle -> down -> hold -> up`;
- partial native sends retry only unsent events;
- a failed release performs recovery and returns an error;
- key and mouse holds span the configured game-frame window;
- Loop action failures stop the failing path instead of reporting success;
- Watch action failures do not increment count or enter cooldown;
- target context is isolated to one Loop run and cleared afterward.

Run targeted Rust tests, the full Rust library suite, frontend tests, TypeScript
checking, and the production frontend build. The release remains uncommitted and
unpublished until manual Watch and Loop testing confirms repeated actions in the
target game.
