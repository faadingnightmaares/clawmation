# Releasing

Clawmation updates itself. The app asks a release manifest whether a newer build
exists, verifies that build's signature against a public key compiled into the
binary, then installs it and restarts. This file is how you publish a build the
running copies will accept.

## The signing key

Updates are signed with [minisign](https://jedisct1.github.io/minisign/); Tauri
refuses any download whose signature doesn't verify against the `pubkey` in
`src-tauri/tauri.conf.json`. The matching private key is **not** in this
repository and must never be; anyone holding it can push code to every install.

The current key lives at `~/.tauri/clawmation.key` and has **no password**.
That is fine for a laptop you control and wrong for anything shared. To replace
it with a protected one:

```bash
npm run tauri signer generate -w ~/.tauri/clawmation.key --force
```

It prompts for a password, writes `clawmation.key` and `clawmation.key.pub`, and
prints the public key. Paste that public key into `plugins.updater.pubkey` in
`src-tauri/tauri.conf.json` and commit it.

**Rotating the key strands every existing install.** Copies running the old
build verify against the old public key and will reject everything signed with
the new one; those users have to download an installer by hand. Rotate before
you have users, or accept that cost deliberately.

## Cutting a release, the short way

Add the private key as a repository secret named `TAURI_SIGNING_PRIVATE_KEY`
(plus `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` if it has one), bump the three version
numbers below, then push a tag:

```bash
git tag v1.0.1 && git push origin v1.0.1
```

`.github/workflows/release.yml` runs the three test suites, builds, signs,
generates `latest.json`, and opens a **draft** release. Edit the notes (the app
shows them verbatim in its update prompt), then publish. Installed copies see the
update from that moment.

The rest of this file is what that workflow does, for when you need to do it by
hand or work out why it didn't.

## Cutting a release by hand

1. Bump the version in **three** places, which must agree:
   `src-tauri/Cargo.toml`, `src-tauri/tauri.conf.json`, and `package.json`. The
   updater compares the manifest's version against the one baked into the
   running app, so a stale `tauri.conf.json` means the update is offered forever
   or never.

2. Build with **both** signing variables in the environment. Without the key the
   bundler still produces installers but no `.sig` files, and an unsigned update
   is one the app will not install. Without the password variable the signer
   prompts for one on stdin, which a scripted or backgrounded build never
   answers, so it bundles the installers and then hangs there silently. Set it
   to the empty string when the key has no password:

   ```bash
   TAURI_SIGNING_PRIVATE_KEY=$(cat ~/.tauri/clawmation.key) \
   TAURI_SIGNING_PRIVATE_KEY_PASSWORD= \
   npm run tauri build
   ```

   In PowerShell:

   ```powershell
   $env:TAURI_SIGNING_PRIVATE_KEY = (Get-Content ~/.tauri/clawmation.key -Raw).Trim(); $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = ""; npm run tauri build
   ```

   A signed build ends with a second summary ("Finished 2 updater signatures
   at:") after the bundle list. If you only see the bundle list, nothing was
   signed.

3. Collect the artifacts from `src-tauri/target/release/bundle/`:

   - `nsis/clawmation_<version>_x64-setup.exe`: the installer people download,
     and the same file the updater downloads.
   - `nsis/clawmation_<version>_x64-setup.exe.sig`: its signature.

   Tauri v2 signs the installer itself; there is no `.nsis.zip` unless
   `createUpdaterArtifacts` is set to `"v1Compatible"`. The MSI is signed too,
   but ignore it: NSIS is the only thing the updater installs on Windows.

4. Write `latest.json` next to them:

   ```json
   {
     "version": "1.0.1",
     "notes": "What changed, in a sentence or two. The app shows this verbatim.",
     "pub_date": "2026-07-25T00:00:00Z",
     "platforms": {
       "windows-x86_64": {
         "signature": "<the entire contents of the .sig file>",
         "url": "https://github.com/<owner>/clawmation/releases/download/v1.0.1/clawmation_1.0.1_x64-setup.exe"
       }
     }
   }
   ```

   `signature` is the file's text, not a path. `version` must not carry a `v`
   prefix; it is compared as semver against the running build.

5. Publish a GitHub release tagged `v<version>` with `latest.json`, the
   `-setup.exe`, and its `.sig` attached.

## The endpoint

`src-tauri/tauri.conf.json` points at:

```
https://github.com/a7mda/clawmation/releases/latest/download/latest.json
```

**Check that owner and repository name before the first release.** A wrong URL
404s, and a 404 is indistinguishable from "no update": the app will quietly
report that everyone is up to date forever. Tauri also expands `{{current_version}}`,
`{{target}}`, and `{{arch}}` in endpoint URLs if you'd rather serve a manifest
per platform.

## What the app does with all this

- At launch, `commands::misc::check_in_background` runs one check off the startup
  path and emits `update-available` if it finds something. `App.tsx` turns that
  into a toast pointing at Settings › About. A failure here is silent, because an
  offline machine still has to start.
- Settings › About checks on demand and, on a hit, offers the release notes and
  an **Install and restart** button. That calls `install_update`, which
  re-checks, downloads with progress, hands the installer control, and restarts.
- `installMode` is `passive`, so the user sees a progress window they can't
  misconfigure rather than a full installer wizard.
