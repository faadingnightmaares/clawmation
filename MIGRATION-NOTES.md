# Migration notes — Python → Rust/Tauri

This is a faithful 1:1 port of the original Python "Clawmation" app. The prime
directive is **behavioral fidelity**: existing user data (`config/`, `macros/`,
`templates/`, `snapshots/`) must keep working unchanged, and every feature must
behave exactly as it did.

This file records the places where fidelity meant deliberately preserving
surprising source behavior, so a future maintainer can decide whether to change
it — as an intentional, separate decision rather than an accidental regression.

## Preserved source quirks (bug-for-bug)

### 1. Chain / Schedule zero-coercion on load

`Chain.from_dict` and `Schedule.from_dict` in the Python source use `x or default`:

```python
delay_between = float(d.get("delay_between", 1.0) or 1.0)
repeat        = int(d.get("repeat", 1) or 1)          # Chain
interval_min  = float(d.get("interval_min", 30.0) or 30.0)  # Schedule
repeat        = int(d.get("repeat", 1) or 1)          # Schedule
```

Because `0 or default == default`, a **persisted** `0` is silently replaced with
the default on load. This defeats "repeat = 0 → run forever" and
"interval_min = 0" after a save/reload round-trip.

The Rust port reproduces this exactly (`models/chain.rs`, `models/schedule.rs`
deserialize a raw struct, then coerce). `Guard` and `Step` do **not** coerce —
they preserve a persisted `0`/`""`/`[]` — matching their Python `from_dict`.

**To change later:** drop the `if raw.x == 0 { default } else { raw.x }` branches
in `Chain`/`Schedule` deserialization and let the plain serde default apply only
when the key is absent.

## Intentionally not ported

- **AI step-builder (`ai_*` commands + builder view)** — orphaned/dead in the
  current app (loaded but never reachable in the UI). The 9 `ai_*` methods and
  `image-slot.js` are excluded. The live "Steps" feature (`steps_*` commands) is
  a separate, active surface and **is** ported.
- **ML-OCR (EasyOCR / torch)** — lazy-imported and excluded from the Python
  production build. The `"ocr"` guard method stays present but reports
  "unavailable", matching what the shipped app actually does.

- **`InputController` dead paths (`hardware/input.rs`)** — the `humanize`
  jitter, the `click_delay_ms` inter-click sleep, and the `clicks > 1` loop are
  all unreachable in the shipped app: every call site constructs
  `InputController()` with defaults, and `clicks` is always 1. They are omitted.
  The user-facing **"humanize clicks"** option is a *separate* feature that
  lives in the guard engine (`humanize_clicks` config → bezier-move then click)
  and is ported with the guards slice — not lost. Likewise `double_click`,
  `right_click`, `hotkey`, and `type_text` have no live caller (only the
  unported `ai_macro.py` touched `type_text`) and are omitted.

## Deliberate upgrades (behavior-preserving)

- **Atomic file writes** — JSON/YAML saves write to a temp file and rename, to
  avoid truncation on crash. Output bytes are unchanged; only the write is safer.

- **`bulk_delete` never aborts mid-batch** — Python's `path.unlink()` has no
  `try/except`, so an existing-but-unremovable file (permission/lock) raises and
  aborts the whole call with no result. The Rust port reports that file in
  `failed` and continues, keeping the `{ok, deleted, failed}` contract intact.
  Every reachable path is identical (a missing file still lands in `failed`, a
  removable file in `deleted`); only the rare unremovable-file case differs.

- **Key resolution replaces the `pydirectinput` fallback (`hardware/input.rs`)**
  — Python resolves recorded key strings through its `_VK` map and, on a miss,
  falls back to `pydirectinput`. There is no Rust equivalent, so the standard
  US-layout OEM punctuation keys (`; = , - . / ` [ \ ] '`) are folded into the
  VK map instead: they resolve to the same scan codes `pydirectinput` would emit
  (each verified via `MapVirtualKeyW`), so a punctuation key still presses
  correctly rather than being dropped. A truly unmappable key (non-US layout,
  exotic name) becomes a no-op. This is unobservable for this app in practice —
  0 of 604 events across the real macro files use any key beyond `a`/`b`.

- **Virtual-screen metrics are read directly, not cached** — Python cached
  `GetSystemMetrics` for 1 second to avoid ctypes marshalling overhead on every
  mouse move. A direct syscall in Rust is negligible, so the cache (and its
  mutable time-keyed state) is dropped; the coordinates produced are identical.

## Recorder (`hardware/recorder.rs`)

Faithful port of `recorder.py::MacroRecorder`. Decisions worth recording:

- **Hand-rolled `WH_MOUSE_LL` / `WH_KEYBOARD_LL` hooks, not the `rdev` crate.**
  pynput installs exactly these two low-level hooks under the hood; we install
  them directly via `windows-sys` — the same binding crate the input slice uses
  for `SendInput`. This keeps the dependency surface minimal for an open-source
  release and, more importantly, gives us exact control over the recorded event
  shape and key naming so a recording round-trips to identical playback. The
  hooks are **non-suppressing** (always `CallNextHookEx`), matching pynput's
  observe-only listeners: the user's real clicks/keys still reach the game while
  recording.

- **Two structural facts of low-level hooks are handled deliberately.** (1) The
  hook procedure is a bare C callback with no user-data parameter, so the live
  recording state lives in a process-global `Mutex<Option<RecorderState>>` the
  procs lock — honest, since a system-wide hook is inherently a singleton (as are
  pynput's module-level listeners). (2) A low-level hook only fires while its
  installing thread pumps messages, so recording runs on a dedicated thread with
  a `GetMessage` loop, stopped by `PostThreadMessage(WM_QUIT)`.

- **`key_name` is the exact inverse of `input.rs::resolve_vk`** (verified by the
  `key_name_is_the_inverse_of_resolve_vk` test): recording a virtual key emits
  the string pynput's `_key_name` would (`key.char or ""`, else `key.name`), and
  that string resolves straight back to the same VK. Character keys record the
  **unshifted** base character (`A`→`"a"`); that is replay-equivalent to pynput
  storing the shifted char, because `resolve_vk` lower-cases and Shift is a
  separate recorded event. The same best-effort boundary as `resolve_vk` applies
  (see "Key resolution replaces the `pydirectinput` fallback" above): numpad keys
  record their character (a pynput-equivalent, not a strict inverse) and the
  handful of pynput names Python's `_VK` never had (`menu`, `num_lock`,
  `scroll_lock`, `f13`–`f20`) record faithfully but replay as no-ops — exactly as
  in Python. None occur in this app's macros (0 of 604 real events).

- **Modifier keys are named from the distinguished VK the LL hook delivers.**
  The hook reports `VK_LSHIFT`/`VK_RSHIFT`/etc. (not the generic `VK_SHIFT`), so
  the map covers both the specific codes (→ `shift_l`/`shift_r`) and the generic
  ones (→ `shift`); whichever Windows delivers produces pynput's name for it.

- **X-buttons and horizontal wheel match pynput.** `WM_XBUTTON*` reads the high
  word of `mouseData` for `x1`/`x2`; `WM_MOUSEWHEEL` records the signed high word
  floor-divided by one notch (a high-resolution partial notch rounds to 0, as in
  pynput's `// WHEEL_DELTA`); `WM_MOUSEHWHEEL` records a delta-0 scroll (pynput's
  horizontal `on_scroll` reports `dy = 0`).

- **Timestamps are rounded to 4 decimals in `stop()`**, mirroring Python's
  `InputEvent.to_dict` (`round(timestamp, 4)`). Every macro on disk already has
  4-decimal timestamps, so freshly-recorded events must round the same way to
  land at identical precision. Rounding happens in the recorder rather than the
  model serializer, keeping the (already byte-identical) model load/save path
  untouched. `created_at` is **not** rounded — it is set to the current
  wall-clock time (`SystemTime::now`), matching Python's `Macro.created_at =
  field(default_factory=time.time)`; the `float_roundtrip` serde_json feature
  preserves its full precision. (Loading a macro file that predates `created_at`
  still defaults it to `0.0` via the model's serde default — that path is
  unchanged.)

## MacroPlayer (`hardware/player.rs`)

Faithful port of `recorder.py::MacroPlayer`. Playback runs on a background
thread; the public handle (`play`/`stop`/`pause`/`resume`/`is_playing`) is called
from the command thread and, later, the guard engine, so all shared state lives
in an `Arc<PlayerShared>` (atomics + a `Condvar` pause gate). Decisions worth
recording:

- **The pause "catch-up burst" is a preserved quirk, not a bug.** Each iteration
  fixes a single `t0 = Instant::now()` and every event fires at
  `t0 + timestamp/speed`; `t0` is **never rebased** when playback pauses and
  resumes (Python's `_play_loop` does the same). So if a pause spans, say, 2
  seconds, every event whose timestamp falls inside that window is already "due"
  on resume and fires back-to-back in an instant burst before real-time pacing
  resumes. This matters because the **guard engine pauses/resumes the player
  mid-playback** (`ui_app.py` attaches the guards with `player=self._player`): a
  guard that takes 2s to handle produces a 2s catch-up burst of macro actions on
  resume. It is intentional and inherited by the guards slice — an open-source
  reader will be tempted to rebase `t0` on resume; that would be a behavior
  change, so decide it deliberately.

- **Stop-during-pause can't be lost.** `stop()` sets `stop_flag` *under the
  pause mutex* and then notifies the condvar; `wait_while_paused` checks
  `stop_flag` *under the same lock* before waiting. This ordering closes the
  classic lost-wakeup race (a stop landing between the predicate check and the
  `wait`). `stop()` also flips `playing` to false immediately, matching Python's
  `stop()` setting `_playing = False` before the thread has unwound.

- **1ms timer + no mouse acceleration per iteration, via RAII.** Each iteration
  raises the system timer resolution (`timeBeginPeriod(1)`, needs the
  `Win32_Media` feature) so `thread::sleep` isn't stuck on the ~15ms grid, and
  disables pointer acceleration so relative-delta moves map 1:1 to cursor motion.
  Both are `Drop` guards (`HiResTimer`, `NoAcceleration`) that restore on scope
  exit, mirroring Python's `timeEndPeriod` `finally` and the `_NoAcceleration`
  context manager.

- **`wait_until` sleeps the long gap and spins the last ~1-2ms** — a direct port
  of `_wait_until` (sleep `remaining - 1ms` while `remaining > 2ms`, else
  busy-wait to the target), checking `stop_flag` in both branches. Stop latency
  is therefore bounded by the current inter-event gap, exactly as in Python (a
  single `time.sleep` is likewise uninterruptible mid-sleep).

- **Mouse moves replay as relative deltas; button edges send button-only.**
  After the first move seeds the absolute position, subsequent moves send
  `move_relative(dx, dy)` (skipping zero deltas), and `MOUSE_DOWN`/`MOUSE_UP`
  send only the button — never an absolute move — because an absolute
  `SetCursorPos` here emits a `WM_INPUT` that corrupts Roblox's Raw Input delta
  tracking (breaks right-click-drag camera rotation). Legacy synthetic
  `MOUSE_CLICK` events are skipped whenever the macro carries real down/up edges.

- **Two Python conveniences are dropped as dead code.** `_densify_moves` (the
  move-interpolation helper) has no caller — the play loop's own comment says
  intermediate points are deliberately *not* fabricated — so it is omitted. The
  `on_event` playback callback is defined on `play()` but **never passed** at any
  of the four `.play()` call sites (`ui_app.py:736`, `ui_app.py:2416`,
  `__main__.py:156`, `__main__.py:206`); it is distinct from the VisionAgent's
  own `on_event(kind, message)` and is omitted with it.

- **The vision `CHECKPOINT` branch is deferred, not lost.** Playback currently
  skips `CHECKPOINT` events, which is faithful to the Python player's behavior
  when no detector/frame_provider is wired (it logs "skipped" and continues) and
  is zero-impact for real data — 0 of 604 real macro events are checkpoints. The
  `_run_checkpoint` / `_hold_follow` / `_trace_line` / `_do_action` logic depends
  on the detector (`PixelDetector`) and capture, so it ports in with the
  detection/capture slice; `play_loop`'s checkpoint arm marks the seam.

## Frontend skeleton (`src/`)

The React frontend is a faithful port of the Python pywebview UI, which is
already React-shaped (`index.html`'s `class Component extends DCLogic` with
`state`/`setState`/`render`). This slice ports only the **app shell** — the
window chrome, the 54px status/action bar, the 206px sidebar, and the
`get_status` heartbeat — proving the Tauri↔React command seam. The seven views
are stubs; each fills in with its own slice.

- **Native window chrome, not frameless.** `tauri.conf.json` sets
  `decorations: true`, `1180×760` default, `960×640` minimum — read directly
  from the Python `webview.create_window(... frameless=False, min_size=(960,
  640) ...)` and its `win_kwargs=dict(width=1180, height=760)`. The OS titlebar
  handles drag / minimize / maximize / close; the only window command the app
  ever exposes is minimize-to-tray.

- **Fonts: the runtime Google Fonts `@import` is dropped.** The source's
  `colors_and_type.css` pulled Newsreader / Inter / Fira Code from
  `fonts.googleapis.com` at load. An offline-first desktop app must not depend
  on the network to render its own text, so `theme.css` keeps the token font
  *stacks* (which fall back to system serif/sans/mono) but omits the `@import`.
  Vendoring the woff2s locally with `@font-face` is a behaviour-preserving
  fast-follow that restores the exact typefaces.

- **The "Guards" nav item routes to view id `ai`.** A historical name from an
  earlier AI-macro-builder; the label is "Guards" but the internal id stays
  `ai` for fidelity (`nav.ts`). Alt+1..7 map to the sidebar order.

- **Top-bar minimize + Stop buttons render disabled.** They belong to the 54px
  bar's layout, but their commands (`window_minimize_to_tray`, `emergency_stop`)
  land in the tray/hotkeys slice. Rendering them disabled preserves the layout
  honestly rather than wiring a dead onClick.

- **Icons use `@phosphor-icons/react`.** The Python UI used the Phosphor
  webfont (`.ph .ph-*`); the React port uses the same icon set as tree-shaken
  components (MIT, offline once installed) — `SquaresFour`, `List`,
  `ShieldCheck`, `Eye`, `FlowArrow`, `BookOpenText`, `Gear`, `Tray`, `Stop`,
  `ArrowClockwise` map 1:1 to the source glyphs.

- **What's verified, and what isn't.** Three checks, each covering a different
  layer: `App.test.tsx` (vitest + RTL) mounts `<App>` with
  `@tauri-apps/api/core`'s `invoke` **mocked**, then asserts a `get_status`-shaped
  payload drives the mode indicator and the activity log and that a sidebar click
  swaps the view — this proves the **frontend contract** (given a `Status`, the UI
  renders and routes correctly), *not* that `invoke("get_status")` reaches the Rust
  command over IPC. `npm run build` (`tsc` + `vite`) proves types + bundling.
  `cargo check` in `src-tauri` proves `tauri.conf.json` deserializes (via
  `tauri-build` in `build.rs`) and the full app binary compiles with `dist/` in
  place. What stays **unobserved until the first `tauri dev`** is both the visual
  render *and* the real IPC round-trip — the frontend half is tested against a
  mocked backend, so live invoke→command→render is confirmed only when the app is
  booted on a display. (App-defined commands aren't ACL-gated in Tauri v2, so no
  permissions setup is pending — the round-trip should just work once booted.)

## Capture (`hardware/capture.rs`)

Faithful port of `capture.py::ScreenCapture` — the DXGI Desktop Duplication
("dxcam") primary path with a GDI ("mss") fallback. Backend auto-selection
matches the source exactly: `"dxcam"` tries Desktop Duplication and falls back to
GDI when it can't initialise; `"wgc"`, `"mss"`, and any unknown string all resolve
to GDI. Output is tightly-packed BGR in a plain `Frame { bgr, width, height }`,
matching dxcam's `output_color="BGR"` and mss's `np.array(shot)[:, :, :3]` (the
DXGI surface is `B8G8R8A8`, so dropping the alpha byte yields the same B,G,R order
dxcam emits). Decisions worth recording:

- **The `windows` crate is used here — and only here — instead of `windows-sys`.**
  Every other hardware slice (input, recorder, player) binds Win32 through
  `windows-sys` for a minimal dependency surface. Desktop Duplication is ~8
  chained COM interfaces (`ID3D11Device` → `IDXGIDevice` → `IDXGIAdapter` →
  `IDXGIOutput1` → `IDXGIOutputDuplication`, plus the staging `ID3D11Texture2D`
  and its map), and hand-indexing those vtables through `windows-sys` is
  impractical, so `capture.rs` alone pulls the method-wrapper `windows` crate
  (same Microsoft family). The alternative — adopting a capture crate such as
  `xcap` — was rejected: `xcap` returns an `image::RgbaImage` (which would
  prematurely settle the still-open vision-stack decision, see below) and its
  recorder model doesn't match dxcam's reuse-on-timeout semantics, so we'd end up
  re-wrapping its backend anyway. The rationale for hand-rolling is *control of
  the exact semantics to preserve*, not dependency minimalism — the `windows`
  crate is heavy. The Cargo.toml block carries the same note.

- **Synchronous on-demand capture replaces the threaded `video_mode` ring buffer.**
  Python always started dxcam threaded (`start(target_fps=60, video_mode=True)`)
  and read via `get_latest_frame()`, whose whole point was two observable
  guarantees: the first read blocks until a frame exists, and afterwards it
  *never returns `None`* — a static screen reuses the last frame
  (`capture.py:74-83, 143-160`). The Rust port drops the background thread (a
  clean codebase doesn't need it) and does one `AcquireNextFrame` per `grab()`,
  but preserves both guarantees exactly: the first `grab()` blocks up to 1000 ms
  for a frame, and every later `grab()` reuses the cached `Frame` on
  `DXGI_ERROR_WAIT_TIMEOUT` (static screen) rather than returning `None`. This is
  precisely the "#1 reliability problem" the Python docstring calls out — one-shot
  mode returning `None` on a static screen — and the port must not reintroduce it.
  On the very first call, before any frame is cached, a 1000 ms timeout returns
  `None`; that mirrors Python's own `if frame is None: return self._last_frame`
  fallback (`capture.py:155-157`, `_last_frame` starts `None`), and the detection
  consumer already tolerates `None`. In practice the first real frame arrives in
  milliseconds (the app's own window is painting), so the deadline is a safety
  bound, not a latency the user sees.

- **The blank first frame is skipped (`LastPresentTime != 0`).** Desktop
  Duplication's first `AcquireNextFrame` after `DuplicateOutput` returns a
  *metadata-only* frame — `LastPresentTime == 0`, `AccumulatedFrames == 0`, and an
  **unpopulated (all-zero) surface**. Copying it yields an entirely black image
  (this was diagnosed on-hardware: correct dimensions and row pitch, source
  surface all zeros). The grab loop therefore copies only frames carrying a real
  desktop present (`LastPresentTime != 0`) and skips metadata-only updates,
  looping until a real frame arrives or the deadline passes — exactly what dxcam's
  own capture loop does internally. Without this the hardware smoke test failed
  with "frame was entirely black"; with it the test is green and stable.

- **WGC (`Windows.Graphics.Capture`) is not ported.** The Python `_init_wgc`
  method exists but is **dead**: `_init_backend` rewrites `"wgc"` to `"mss"`
  *before* dispatch (`capture.py:48-53`) and only dxcam→mss are ever in the
  fallback chain, because — per the source's own comment — WGC "segfaults on some
  machines" and a segfault "CANNOT be caught by try/except". Selecting WGC in
  Settings warns and uses GDI. The Rust port reproduces that mapping and omits the
  WGC path entirely; nothing reachable is lost.

- **The fps metric is read-throughput, and the formula is preserved bit-for-bit.**
  `capture.py::_track_fps` times `dt = perf_counter() - t0` around the frame read
  (`t0` at the top of `grab()`, recorded after the frame is in hand), pushes
  `1/dt` into a window capped at 60 samples (dropping `dt <= 0`), and reports the
  arithmetic mean (or `0.0` when empty). `FpsWindow` mirrors this exactly. Note
  this measures *read* throughput, not the true capture rate, and the absolute
  number will differ from Python (a hand-rolled Desktop Duplication read costs
  differently than dxcam's `get_latest_frame`). That's acceptable: the value is
  cosmetic telemetry surfaced in the status heartbeat and is never a decision
  input, so preserving the *formula* — not the number — is the fidelity bar.

- **Three Desktop Duplication hazards are handled explicitly**, each of which
  silently corrupts output if ignored: (1) `DXGI_ERROR_ACCESS_LOST` /
  `DXGI_ERROR_ACCESS_DENIED` — raised on fullscreen / resolution / secure-desktop
  (UAC) transitions — drop the duplication so the next `grab()` re-runs
  `DuplicateOutput`, reusing the cached frame meanwhile; (2) the staging texture's
  `D3D11_MAPPED_SUBRESOURCE.RowPitch` is padded wider than `width * 4` on some
  GPUs, so BGRA→BGR conversion walks the source row-by-row at `RowPitch` (a direct
  `width*height*4` read would produce a skewed image); (3) `WAIT_TIMEOUT` reuses
  the last frame *and still records an fps sample*, matching the threaded model
  where every `get_latest_frame` counted. The RowPitch handling is covered by a
  pure unit test (`bgra_to_bgr_respects_row_pitch`) since it's the one piece of
  real logic in an otherwise thin OS-plumbing module; the rest is proven by the
  `#[ignore]`d on-hardware smoke test, which asserts the frame's dimensions equal
  the primary output size (not merely non-zero — a skewed frame is non-zero but
  wrong) and that it isn't uniformly black.

- **AppState wiring and real-backend/fps status are deferred to the detection
  slice.** No consumer warms capture yet, and `commands/status.rs` already
  faithfully reports the `_capture is None` state — Python's
  `self._capture.backend if self._capture else self.config.capture_backend`
  reduces to `config.capture_backend` with `0.0` fps, which is exactly the current
  stub. Lazy `get_capture` construction in `AppState` and the resolved-backend /
  live-fps report become observable only once the detection loop pulls frames, so
  they land there (that slice also validates cross-thread `Send`/`Sync` for the
  COM handles). `grab_rgb` (Python's `grab()[:, :, ::-1]` for OCR/RGB consumers)
  is likewise deferred to its first RGB caller. These are tracked deferrals, not
  silent omissions.
