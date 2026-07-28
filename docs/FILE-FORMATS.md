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

A self-contained macro with its safeguards and vision images:

```text
manifest.json
payload/macro.json
payload/guards.json      (when safeguards exist)
assets/<blake3>.<ext>    (when referenced)
```

Macro and safeguard JSON use Zstandard level 10. PNG, JPEG, and WebP files stay
byte-for-byte lossless and are stored without redundant recompression; BMP uses
Zstandard. Assets are content-addressed, so identical images are stored once
even when several safeguards reference them.

## Import guarantees

- Digests and declared sizes are verified before data is installed.
- Entry counts and expanded sizes are bounded.
- Absolute paths, traversal paths, duplicate entries, undeclared files, unsafe
  macro names, and dangling image references are rejected.
- Existing names are never overwritten; imports receive a numeric suffix.
- Legacy standalone `.json` macros and pre-manifest `.clawbundle` archives
  remain importable.
