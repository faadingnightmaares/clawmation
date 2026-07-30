# Adaptive Fast Vision Clicks

## Goal

Increase repeated Find & Click throughput in a running Loop without weakening
the input guarantees introduced in 1.2.3. The optimization is automatic and
requires no new control in the Loop inspector.

The first click remains fully prepared. Later clicks may use a warm path only
while the same external window remains valid and foreground. Any uncertainty
before the press falls back to the existing full preparation path.

## Measured baseline

The release matcher finds a remembered full-screen target in approximately
8 ms on the development machine. The current reliable click transaction adds
226 ms of fixed waits to every hit:

- 80 ms after focusing the target window;
- 16 ms for the non-coalesced pointer arm;
- 50 ms for hover settling;
- 80 ms while the mouse button is held.

This caps a simple repeating Loop near four clicks per second before capture
and matching costs. With multiple image states, the detector can also scan an
absent alternative after an already acceptable preferred match, adding roughly
138 ms in the measured release benchmark.

## Design

### Preferred image short-circuit

Image candidates retain their existing OR semantics. The first search checks
the candidates normally and remembers the successful candidate. On later
searches, the preferred candidate runs first.

When that preferred candidate produces any hit accepted by the node's configured
confidence threshold, detection returns it immediately. It no longer searches
the remaining appearance alternatives merely to find a higher confidence score.
If the preferred appearance is absent, the detector continues through the other
candidates and remembers the next successful appearance.

The first search, missing-target behavior, confidence threshold, coordinates,
and single-image behavior remain unchanged.

### Warm click transaction

The Loop executor already retains the target established by the previous Vision
action. A repeated click offers that target to the reliable input layer.

The warm path is eligible only when:

1. the window currently beneath the new match has the same stable window id;
2. that window is still the foreground window; and
3. the cursor can be positioned and verified at the new match coordinates.

An eligible warm click:

1. retains the process-wide autonomous-input transaction lock;
2. moves to the detected point;
3. sends the existing two-pixel non-coalesced pointer arm;
4. waits one 16 ms frame, returns and synchronizes to the exact point;
5. verifies foreground ownership and cursor coordinates;
6. sends mouse down, retains the existing 80 ms hold, and sends mouse up;
7. performs the existing focus-loss and release recovery checks.

The warm transaction therefore keeps 96 ms of deliberate input timing while
removing the redundant 80 ms focus wait and 50 ms hover wait. With a remembered
image, the expected steady-state cycle is approximately 105 ms, or about
9–10 clicks per second.

### Fallback and failure boundaries

If a warm-path eligibility or pointer-preparation check fails before mouse down,
the same action immediately retries through the existing full reliable path.
This includes a changed window, lost foreground status, cursor drift, or a
failed pointer operation.

Once mouse down has been accepted, the action is never retried automatically.
A post-press focus or release failure uses the existing recovery release and is
reported to the Loop. This prevents an uncertain transaction from becoming a
double click.

The retained target is replaced only after a successful click. Watch actions
without a retained Loop target continue using the full reliable path.

## Scope

This change affects repeated Vision clicks in Loops and repeated AI-step runs
that retain a successful target. It does not:

- shorten the first-click preparation;
- reduce the 80 ms press hold;
- change normal recorded-macro playback timing;
- add a speed slider or per-node mode;
- remove target, foreground, cursor, transaction, or release checks;
- change the result schema or portable Loop format.

## Verification

Regression tests will prove:

- the first click still uses the complete preparation sequence;
- a warm click omits only the focus and hover waits;
- a warm click still arms movement, verifies the cursor, holds for 80 ms, and
  sends one ordered down/up pair;
- a changed or non-foreground window falls back before any press;
- a failure after mouse down performs recovery without a second press;
- concurrent Watch and Loop actions cannot interleave button edges;
- a remembered accepted image does not evaluate later alternatives;
- an absent preferred image still falls through to another valid state.

A deterministic timing test will cap the deliberate warm-path sleep budget at
96 ms. The release-mode full-screen benchmark will be rerun to confirm the
preferred image remains near the existing remembered-match baseline. The full
Rust, frontend, TypeScript, and production build checks must remain green.
