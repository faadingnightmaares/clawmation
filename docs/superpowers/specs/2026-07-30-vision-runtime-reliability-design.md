# Vision Runtime Reliability

## Goal

Make Vision actions in Watch, Loops, and guards react quickly and deliver every
requested click, key, or nudge through one observable, cancellation-safe path.
A successful action means that Clawmation used a fresh detection, targeted the
correct external window, completed the input gesture, and observed a newer
frame that confirms the target reacted.

The implementation must preserve existing macro and Loop files. Recorded macro
playback is outside this change.

## Chosen approach

Introduce a staged `VisionRuntime` behind the existing command and engine APIs.
The runtime centralizes fresh-frame capture, prioritized detection, target
tracking, reliable input, and post-action verification without rewriting the
entire application as actors.

This is preferred over separate Watch and Loop patches because isolated fixes
have already allowed the paths to diverge. A full engine rewrite would create
unnecessary release risk.

## Runtime components

### Latest-frame source

A single capture owner publishes immutable snapshots containing:

- a monotonically increasing generation;
- capture time and source identity;
- physical screen dimensions;
- the captured frame.

The buffer is latest-only. Consumers can wait for a generation newer than the
one used for detection, and stale work is discarded rather than queued. A
reused backend frame cannot masquerade as a fresh frame.

### Detection scheduler

Detection requests carry their source, deadline, cancellation state, and
priority. Interactive Watch and executing Loop work outrank background guards.
Compatible template requests may share one snapshot, but an expensive OCR or
cold full-screen miss cannot extend the polling delay of unrelated triggers.

Adaptive polling is based on the cost of each trigger, not the duration of an
entire mixed batch. Active targets use a fast reacquisition path. Moved targets
use an expanding search that covers all tiles within a bounded interval rather
than probing one tile every several misses.

GPU work has a short deadline. If a backend is unavailable or contended, the
request immediately falls back to the established CPU matcher instead of
blocking the runtime.

### Reliable action executor

Watch, Loops, and guards dispatch typed actions to the same serialized
executor. Each action transaction:

1. rejects stale or superseded detections;
2. resolves and verifies the external target window;
3. establishes foreground focus;
4. moves to the scale-correct physical click point;
5. arms Roblox Raw Input with a tiny reversible relative motion;
6. keeps a one-frame hover settle on both cold and warm clicks;
7. sends a frame-spanning down/hold/up gesture;
8. guarantees best-effort release after any accepted down event;
9. returns a receipt naming every completed phase.

The executor retains a verified target for warm repeated actions, but clears it
after focus loss, target change, stale detection, cancellation, or any gesture
failure. Native `SendInput` acceptance alone is delivery evidence, not proof
that the game reacted.

### Fresh-frame acceptance and automatic retry

After a delivered action, the runtime waits for a newer frame generation and
re-evaluates the target region. The action is accepted when the target
disappears or changes beyond the configured visual tolerance.

If the same actionable target remains visible, Clawmation automatically retries
the complete reliable gesture. Retries are serialized, use a fresh frame each
time, preserve the hover settle, and stop immediately on cancellation or target
window loss. One transaction permits three delivery attempts with bounded
backoff; after that it returns a visible failure and leaves Watch or the Loop
eligible to detect the target again. It never sends two presses from the same
frame generation.

Targets intentionally designed to remain visible continue through their normal
Watch or Loop cadence after the transaction. This bounds accidental duplicate
submissions while still recovering from Roblox ignoring a click.

### Coordinates and templates

Template detections expose the matched scale. Template-local click offsets are
scaled before translation to screen coordinates. Color and OCR actions retain
physical-screen-pixel semantics. Existing persisted actions keep their current
interpretation through explicit schema defaults; no user files are rewritten.

Multiple normal and hovered template images remain equivalent candidates. The
runtime tracks the winning candidate and its scale for action verification.

## Watch, Loops, and guards

- Watch retains its last verified target and uses warm action preparation when
  the same target reappears.
- Watch only increments its fired count or enters cooldown after action
  acceptance. Failed actions stay eligible.
- Loop vision nodes use the same detection receipt and action executor. Node
  success means accepted action, not merely detected target.
- Productive forever Loops are not stopped by a global transition count.
  Cancellation remains authoritative, while a progress watchdog throttles and
  reports zero-delay cycles that perform no wait, detection, action, or state
  change.
- Guards return `Result`. They emit success and stamp cooldown only after an
  accepted action; delivery failures remain retryable.
- `wait_for` remains detection-only unless its node explicitly contains an
  action.

## Concurrency and cancellation

Capture, detection, and input have an explicit lock order and never wait while
holding graph, player, or UI state locks. Queues are bounded and latest-only
where newer work supersedes old work. Stop cancels queued detection, prevents
new presses, and still releases any button or key already held.

Every detection and action receipt records generation, capture age, detector
duration, queue delay, target window, input phases, attempt count, acceptance
result, and total latency. Normal logs remain compact; failure details are
available for diagnosis.

## Performance requirements

- Warm target reacquisition must not use the full-screen cold path.
- Watch scheduling must not impose the existing cost-based delay of up to
  1.5 seconds on an active visual target.
- A GPU timeout must not block a CPU fallback for seconds.
- No unbounded frame or request queue may form under concurrent Watch, Loop,
  and guard activity.
- The runtime should react on the first fresh frame containing a reappeared
  target, subject only to capture and matcher time.

## Failure behavior

- Stale detections are retried from a fresh frame and never actuated.
- Focus, integrity-level, cursor, native-send, release, or acceptance failures
  identify their exact phase.
- A pre-press failure may retry safely. A post-down failure first releases,
  then starts a new verified attempt only if the target still exists on a fresh
  frame.
- No failed action increments counters, advances a success edge, or starts a
  cooldown.
- CPU detection remains the universal fallback when acceleration is absent or
  unhealthy.

## Verification

Add deterministic tests with fake capture, detection, focus, cursor, and native
input seams:

- stale and superseded frames never dispatch actions;
- all requests in a compatible batch use one immutable generation;
- Watch priority is unaffected by slow OCR, cold misses, or guard work;
- a reappeared target is detected on the first fresh frame;
- cold clicks and warm repeats both perform Raw Input arming and hover settle;
- a target unchanged on a new frame triggers an automatic retry;
- retries never reuse a generation and stop after three attempts;
- disappearance or visual change prevents an extra click;
- partial input sends and release failures recover without stuck buttons;
- scale-aware offsets are correct at 0.5x, 1x, 1.25x, and 1.5x;
- failed guards do not fire cooldowns;
- a productive forever Loop exceeds 10,000 transitions and stops by
  cancellation, while a zero-progress cycle is throttled;
- concurrent Watch, Loop, and guard soak tests preserve gesture ordering and
  bounded latency.

Verification also includes the full Rust suite, frontend tests, TypeScript
checking, the production frontend build, release-mode detection benchmarks, and
manual release-mode Roblox testing. Publishing occurs only after the user
approves the tested result.
