# Clawmation portable formats

Both portable formats are versioned ZIP containers. Their `manifest.json`
declares `version`, the producing app version, every payload's uncompressed byte
length, and its BLAKE3 digest.

## `.clawmation`

A standalone macro:

```text
manifest.json
payload/macro.json
```

The macro is serialized as compact JSON and compressed with Zstandard level 10.

## `.clawbundle`

`.clawbundle` is the self-contained, asset-bearing container. Its manifest
identifies whether the payload is a macro or a Loop.

### Macro bundle

```text
manifest.json
payload/macro.json
payload/guards.json      (when safeguards exist)
assets/<blake3>.<ext>    (when referenced)
```

### Loop bundle

```text
manifest.json
payload/loop.json
assets/<blake3>.<ext>    (when referenced)
```

Loop bundles include images referenced by Wait and Find & Click nodes and every
embedded macro snapshot. Import installs the images into the local template
store, rewrites the Loop references, and saves the Loop under `macros/nodes`.

Loop edges may include an optional `waypoints` array of `{x, y}` canvas
positions. Older Loops without it load with automatic routing. Action, Macro,
and Chain node configuration may include `failure_mode` with `stop`, `continue`,
or `recovery`; when absent, an existing `error` edge preserves the legacy
recovery behavior and all other failures stop the Loop.

JSON uses Zstandard level 10. PNG, JPEG, and WebP files stay byte-for-byte
lossless and are stored without redundant recompression; BMP uses Zstandard.
Assets are content-addressed, so identical images are stored once even when
several safeguards, nodes, embedded steps, or image alternatives reference
them.

Vision operations keep their singular image field for compatibility and may
include up to seven alternatives. The eight candidates use OR semantics:
normal, hovered, selected, and other appearances all represent one target and
produce one action.

## Import guarantees

- Digests and declared sizes are verified before data is installed.
- Entry counts and expanded sizes are bounded.
- Absolute paths, traversal paths, duplicate entries, undeclared files, unsafe
  macro names, and dangling image references are rejected.
- Existing names are never overwritten; imports receive a numeric suffix.
- Legacy standalone `.json` macros and pre-manifest `.clawbundle` archives
  remain importable.
