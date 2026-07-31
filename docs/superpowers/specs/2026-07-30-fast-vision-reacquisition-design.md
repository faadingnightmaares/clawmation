# Fast Vision Reacquisition Design

## Objective

Make repeated Loop and Watch image detection react immediately when a previously
seen control disappears and reappears, including controls that switch between
normal and hovered images. Preserve the current robust first search, configured
confidence threshold, multi-scale recovery, and reliable input transaction.

## Measured Problem

The reproduced Loop searches the full 2560×1440 screen for two appearances of
the same button. Release measurements on the target machine are:

- present, cold: about 58 ms;
- present at a remembered location: about 8 ms;
- absent, GPU warm: about 117 ms per candidate.

When the button is absent, both appearance candidates run their complete robust
search sequentially. One negative Loop pass therefore occupies roughly 234 ms
before another frame can be captured. If the button reappears after that pass
captured its frame, detection cannot observe it until the stale negative work
finishes. A changed hover appearance adds another broad scan before the valid
alternative is reached.

The input actuator is not the cause of this delay. Its repeated-click path has a
fixed 96 ms deliberate budget and begins only after detection returns a hit.

## Selected Approach

Use a hybrid reacquisition tracker:

1. Keep the complete robust search for an operation with no trusted history.
2. After a confirmed hit, remember the operation's center, matched dimensions,
   appearance, and scale.
3. On later frames, check every configured appearance in a small hot zone around
   the last confirmed target.
4. If the hot-zone check misses, run a learned-scale coarse probe over the
   configured search region. All appearances share the frame preprocessing.
5. Confirm every coarse nomination at native resolution with the user's exact
   confidence threshold before reporting a hit.
6. Run the complete multi-scale and edge recovery search periodically, not on
   every negative frame. Stagger alternative candidates so one poll cannot be
   monopolized by several full negative searches.

This preserves robustness while removing repeated broad negative sweeps from
the latency-critical path.

## State and Boundaries

`Detector` will own per-operation reacquisition state keyed by the existing
guard or step operation key. State contains:

- the last confirmed detection rectangle;
- the source template dimensions or equivalent learned scale;
- the preferred appearance key;
- a deterministic miss/rescue counter;
- the next appearance eligible for a full recovery scan.

The state is detector-local and never serialized into macro files. Replacing a
template clears its template memory and invalidates any operation state that
references it. A screen-size or configured-region change invalidates an anchor
that no longer falls within the resolved search area.

The template matcher will expose focused primitives for:

- native/local confirmation within a bounded crop;
- one learned-scale coarse nomination pass with native confirmation;
- the existing complete robust recovery.

These primitives return ordinary `Detection` values. `Detector` remains
responsible for candidate order, operation memory, and deciding when recovery is
due. Capture, graph execution, and input actuation interfaces do not change.

## Detection Flow

For each frame and operation:

1. Convert the configured region to grayscale once.
2. Load configured candidates and put the preferred appearance first.
3. If trusted operation state exists:
   - crop a padded hot zone around the prior target;
   - test all appearances in that crop;
   - return the first threshold-confirmed hit and refresh state;
   - otherwise build one coarse search representation and probe all appearances
     at the learned scale;
   - native-confirm nominations and return a confirmed hit;
   - otherwise return a fast miss unless a recovery slot is due.
4. When recovery is due, run one candidate's complete robust search and advance
   the recovery cursor. A confirmed hit refreshes scale, location, and preferred
   appearance.
5. With no trusted state, run the existing complete candidate search so initial
   detection and imported workflows retain current behavior.

Normal and hovered images are always OR alternatives. A hit from either one
updates the shared operation anchor, so appearance changes do not force a cold
search.

## Correctness and Failure Handling

- A coarse or local score never directly triggers an action. Native-resolution
  confirmation must clear the configured threshold.
- A fast miss is not remembered as a new location.
- Recovery remains available for moved controls, DPI changes, scale changes,
  and stale anchors.
- An unreadable candidate cannot hide a valid alternative.
- Replacing or deleting an image cannot retain stale reacquisition state.
- The existing serialized input transaction, foreground-window verification,
  pointer verification, press hold, and release recovery remain unchanged.
- No detection worker or GPU task is allowed to outlive its owning run; the
  design stays synchronous and avoids new concurrency failure modes.

## Performance Budget

On the 2560×1440 two-appearance reproduction:

- hot-zone reappearance target: at most 15 ms detection;
- learned-scale moved-target target: at most 30 ms detection;
- steady negative fast pass target: at most 20 ms;
- first-ever detection: no regression from the current release baseline;
- end-to-end repeated hit, including the 96 ms reliable click: about 110–130 ms.

Timing assertions will not use wall-clock limits in ordinary unit tests. Release
benchmarks will print the measured distributions and the verification report
will compare them with the baseline.

## Regression Coverage

Tests will cover:

- first use still executes complete robust detection;
- a target disappearing and reappearing at the same location uses the fast path;
- a hovered appearance can reacquire from a normal-appearance anchor;
- a same-scale target moved elsewhere is found by the learned-scale probe;
- a scale-changed target is eventually found by staggered robust recovery;
- local and coarse false positives below threshold never become detections;
- multiple candidates share preprocessing and do not duplicate one target;
- unreadable candidates still fall through;
- template replacement invalidates affected state;
- input is emitted only after a confirmed detection;
- the existing reliable click and concurrency tests remain green.

Verification will run targeted vision tests, the complete Rust suite, frontend
tests, TypeScript, the production build, and release-mode cold, remembered,
absent, and reappearance benchmarks.

## Non-Goals

- No new model or neural detector.
- No user-facing speed or accuracy setting.
- No mandatory manual search region.
- No weakening of confidence thresholds.
- No change to macro, `.clawmation`, or `.clawbundle` formats.
- No release or push as part of implementation verification.
