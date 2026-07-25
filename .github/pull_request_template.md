## What this changes

<!-- One paragraph. What is different after this pull request, and why. -->

## Why

<!-- The reasoning, or a link to the issue. Skip if the subject line carries it. -->

## Checks

- [ ] `cargo test --manifest-path src-tauri/Cargo.toml`
- [ ] `npm test`
- [ ] `npx tsc --noEmit`
- [ ] `cargo build --release` is warning free
- [ ] Hardware tests run by hand, if anything under `src-tauri/src/hardware/` changed

## Detection behaviour

<!--
Delete this section if it does not apply. If your change alters what a guard or a Watch
trigger matches, say so plainly and explain the trade. Those paths are load bearing for
setups nobody here can see.
-->
