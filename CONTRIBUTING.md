# Contributing

PR-Nexus is a private desktop app. Do not commit company names, organization names, Slack workspaces, tokens, or customer data. See `AGENTS.md`.

## Prerequisites

- Rust stable, via rustup. `rust-toolchain.toml` selects the channel and installs rustfmt and clippy.
- Bun 1.3.14.
- On Linux, the WebKitGTK packages CI installs (see `.github/actions/linux-tauri-deps`).

```bash
cd app
bun install
bun run tauri dev
```

`bun run build` writes `app/dist`. `cargo test` and `cargo clippy` need that directory because `tauri-build` embeds it.

## Changelog

Do not edit `CHANGELOG.md` by hand. Install [Changie](https://changie.dev/) and add a fragment:

```bash
changie new
```

Kinds: Breaking (major), Features and Added (minor), Changed, Fixes, Fixed, Docs, and Dependencies (patch). A pull request without a fragment under `.changes/unreleased/` fails CI unless it has the `no-changelog` label.

## CI

`.github/workflows/ci.yml` runs on pull requests, pushes to `main`, the merge queue (`merge_group`), a nightly schedule, and manual dispatch.

Path filters skip expensive jobs when they are not needed. The merge queue requires one check: **`validate`**. That job fails when any job that should have run failed or was cancelled. A skipped job is success.

Jobs behind `validate`:

| Job | What it does |
|---|---|
| `rust-lint` | `cargo fmt`, `cargo clippy -D warnings`, `cargo update --locked` |
| `rust-test` | `cargo test --workspace --locked` on Ubuntu 24.04 and macOS |
| `web` | `bun run lint`, `typecheck`, and `test` |
| `build-check` | Tauri build on macOS, Ubuntu 24.04, and Windows. macOS uploads the `.app` for 7 days |
| `deny` | `cargo deny check` |
| `guards` | actionlint, and a Changie fragment on pull requests |

Linux and Windows are built in CI. They are not attached to GitHub releases.

`.github/workflows/nightly.yml` runs the same kind of checks after `cargo update` without `--locked`. It does not open issues and it does not push the updated lockfile. Its cron is `47 6 * * *`, offset from CI's `17 6 * * *`.

Dependabot opens weekly grouped updates for Cargo, the `app/` Bun lockfile, and GitHub Actions.

## Branch ruleset

Do not create this ruleset from a script or the API in a pull request. A repository admin configures it in GitHub settings. Name: `main-ci-required`. Target: the default branch.

- Block deletion.
- Block force pushes.
- Require the status check `validate` (and no other check).
- Require a merge queue: squash merges, merge method ALLGREEN, maximum 5 entries.
- Delete the head branch after merge.
- Allow `github-actions[bot]` to bypass pull-request requirements so the Tag release workflow can push the release commit and the `v*` tag to `main`.

## Release

v1 ships one artifact: an ad-hoc-sealed macOS Apple Silicon DMG. There is no auto-updater and no Apple Developer ID signature.

1. Merge pull requests that include Changie fragments.
2. On `main`, run **Tag release** (`workflow_dispatch`). It batches fragments (the first cut is `v0.1.0`; later cuts use `changie batch auto`), stamps `[workspace.package] version` in the root `Cargo.toml`, commits `Release vX.Y.Z` as `github-actions[bot]`, and pushes the tag. `app/package.json` stays private at `0.0.0`. `tauri.conf.json` has no version; Tauri reads `Cargo.toml`.
3. **Release** runs because a tag push made with `GITHUB_TOKEN` does not start workflows by itself (`workflow_run`), and also on a direct `v*` push or a manual dispatch. It opens a draft release, builds `PR-Nexus_<version>_aarch64.dmg` on `macos-latest`, reseals the app (`scripts/reseal-macos-app.sh`), uploads with `gh release upload --clobber`, checks that the DMG is attached, then clears the draft. `--latest` is set only when the tag is the highest semver.
4. Re-running Release is safe: an existing draft is reused, uploads clobber, and an already published release is left in place when the DMG is present.

`workflow_dispatch` on Tag release and Release accepts `dry_run`. Dry run batches or checks scripts and does not push a tag, build a DMG, or call the Releases API to publish. A real DMG is only produced by a tag cut on `main`.

`APPLE_CERTIFICATE` is unused. The release job has a Developer ID step that stays skipped until that secret exists, and that step fails on purpose so a secret cannot silently change the ad-hoc build. Signing and notarization are future work.

### Install on a Mac

The DMG is ad-hoc signed (`codesign --force --deep --sign -`), not notarized.

1. Preferred: `gh release download vX.Y.Z -R github-christophe-oudar/pr-nexus -p '*.dmg'`. The repository is private, and files fetched by `gh` do not get the quarantine flag.
2. After a browser download, macOS may say the app is damaged or from an unidentified developer. Open it once, then System Settings → Privacy & Security → Open Anyway. On macOS 15 and later, Control-click → Open does not bypass that.
3. Or: `xattr -dr com.apple.quarantine /Applications/PR-Nexus.app`.

An ad-hoc signature has no stable team id, so macOS asks once per update before the app can read the keychain item `dev.pr-nexus` (one vault entry, so one prompt). Choose Always Allow. That prompt does not exist until the app stores a token; the current build does not.

Smoke test on a clean user account: download the DMG with `gh`, launch PR-Nexus, and confirm the window shows Hello. Installing over an older build should still launch.
