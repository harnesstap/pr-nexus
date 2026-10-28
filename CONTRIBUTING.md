# Contributing

PR Nexus is a private desktop app. Do not commit company names, organization names, Slack workspaces, tokens, or customer data. See `AGENTS.md`.

## Prerequisites

- Rust stable, via rustup. `rust-toolchain.toml` selects the channel and installs rustfmt and clippy.
- Bun 1.3.14.
- On Linux, the WebKitGTK packages listed in `.github/actions/linux-tauri-deps`.

```bash
cd app
bun install
bun run tauri dev
```

`bun run build` writes `app/dist`. `cargo test` and `cargo clippy` need that directory because `tauri-build` embeds it.

## Run on Linux

Install the WebKitGTK packages listed in `.github/actions/linux-tauri-deps/action.yml` (`libwebkit2gtk-4.1-dev`, `libgtk-3-dev`, `patchelf`, and the rest of that list). Then, from the repo root:

```bash
cd app
bun install
bun run build
bun run tauri dev
```

`bun run tauri dev` is the day-to-day command. It starts Vite and the Tauri shell. A one-shot binary, after `bun run build`:

```bash
cargo build -p pr-nexus
./target/debug/pr-nexus
```

Development overrides (none of these are required for a normal launch):

| Variable | Effect |
|---|---|
| `PR_NEXUS_DB_PATH` | SQLite file. Point it at a temp path so a test run does not touch the app-data database. |
| `PR_NEXUS_GITHUB_TOKEN` | GitHub token for this process. Preferred over `gh` and the keychain. Never written to the vault or to SQLite. |
| `PR_NEXUS_DISABLE_KEYRING=1` | Skip the secret service and keep tokens in memory. The window shows a warning. |
| `PR_NEXUS_GITHUB_OAUTH_CLIENT_ID` | OAuth client id for the device flow. Empty by default; `gh` and a pasted PAT still work. |
| `PR_NEXUS_DEMO=1` | Seed invented `acme` data and skip the wizard when the database has no accounts. |

Keychain calls never block the window. While a system prompt is pending the window shows a "Waiting for keychain access" hint; a missing secret service (common on a Linux desktop or CI) or a denied prompt keeps the vault in memory and shows the reason with a Retry button. Example:

```bash
PR_NEXUS_DB_PATH=/tmp/pr-nexus.db \
PR_NEXUS_GITHUB_TOKEN="$TOKEN" \
PR_NEXUS_DISABLE_KEYRING=1 \
  cargo run -p pr-nexus
```

`cargo run -p pr-nexus` needs `app/dist` from `bun run build`. `bun run tauri dev` rebuilds the frontend itself.

A static render of the wizard and the list, with invented data and no GitHub calls:

```bash
cd app
bun run dev
# http://127.0.0.1:1420/preview.html?screen=wizard
# http://127.0.0.1:1420/preview.html?screen=slack
# http://127.0.0.1:1420/preview.html?screen=list
```

## Changelog

Do not edit `CHANGELOG.md` by hand. Install [Changie](https://changie.dev/) and add a fragment:

```bash
changie new
```

Kinds: Breaking (major), Features and Added (minor), Changed, Fixes, Fixed, Docs, and Dependencies (patch). A pull request without a fragment under `.changes/unreleased/` fails CI unless it has the `no-changelog` label.

## CI

`.github/workflows/ci.yml` runs on pull requests, pushes to `main` and `v*` tags, the merge queue (`merge_group`), a nightly schedule, and manual dispatch.

Linux jobs run on the harnesstap org's self-hosted runners (labels `self-hosted`, `Linux`; override with the repository variable `LINUX_RUNNER_LABELS`, a JSON array). The runners are shared: CI never installs system packages on them. If the WebKitGTK headers are missing, the `linux-tauri-deps` step fails and prints the `apt-get` command for a runner admin. macOS and Windows jobs use GitHub-hosted runners and only run for `v*` tags and manual dispatch, to save Actions minutes.

Path filters skip expensive jobs when they are not needed. The merge queue requires one check: **`validate`**. That job fails when any job that should have run failed or was cancelled. A skipped job is success.

Jobs behind `validate`:

| Job | What it does |
|---|---|
| `rust-lint` | `cargo fmt`, `cargo clippy -D warnings`, `cargo update --locked` |
| `rust-test` | `cargo test --workspace --locked` on Linux |
| `web` | `bun run lint`, `typecheck`, and `test` |
| `build-check` | Tauri build (`--no-bundle`) on Linux |
| `desktop` | Tags and dispatch only: macOS tests and `.app` (uploaded for 7 days), Windows build |
| `deny` | `cargo deny check` |
| `guards` | actionlint, and a Changie fragment on pull requests |

Linux and Windows are built in CI (Windows on tags and dispatch). They are not attached to GitHub releases.

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

## macOS release signing (self-signed)

Without a Developer ID, the release job signs `PR Nexus.app` with a stable self-signed certificate, `PR Nexus Release (self-signed)` (RSA 3072, valid until 2046), stored in the `MACOS_SELFSIGN_P12_BASE64` and `MACOS_SELFSIGN_P12_PASSWORD` Actions secrets. `APPLE_CERTIFICATE` stays reserved for a future Developer ID and takes precedence when set; with neither secret the job falls back to the ad-hoc reseal.

The job imports the p12 into a temporary keychain, runs `scripts/reseal-macos-app.sh` with `RESEAL_IDENTITY` (nested code first, then the bundle, `--identifier dev.pr-nexus --timestamp=none`, no hardened runtime since nothing is notarized), checks `codesign --verify --deep --strict`, prints the designated requirement, rebuilds the DMG from the signed app and deletes the keychain. `workflow_dispatch` with `dry_run` and `macos_sign_dry_run` builds and signs on macOS without uploading or publishing.

What users see:

- Gatekeeper still shows the "unidentified developer" warning: a self-signed certificate is not trusted by macOS and cannot be notarized.
- The keychain prompt for `dev.pr-nexus` appears once after the first signed install; click **Always Allow**. The designated requirement (identifier + certificate) stays the same across updates, so it does not come back.
- Rotating this certificate, or moving to a Developer ID, changes the requirement and causes one more prompt.

Verify an installed app:

```sh
codesign -dv --verbose=4 "/Applications/PR Nexus.app"   # Authority=PR Nexus Release (self-signed), Identifier=dev.pr-nexus
codesign -d -r- "/Applications/PR Nexus.app"
codesign -d --extract-certificates=/tmp/prn "/Applications/PR Nexus.app" && openssl x509 -inform DER -in /tmp/prn0 -noout -fingerprint -sha256
```

Certificate fingerprints:

- SHA-1: `D8:CF:D5:07:DB:4E:53:B1:31:EC:FD:3A:6A:F3:EC:48:11:DF:87:50`
- SHA-256: `AE:73:5C:1D:EF:D8:46:7D:22:49:3D:BE:2E:AE:81:60:63:08:17:7E:79:E4:F6:4A:1E:C0:9E:AA:59:F8:0C:85`

## Release

v1 ships one artifact: an ad-hoc-sealed macOS Apple Silicon DMG. There is no auto-updater and no Apple Developer ID signature.

1. Merge pull requests that include Changie fragments.
2. On `main`, run **Tag release** (`workflow_dispatch`). It batches fragments (the first cut is `v0.1.0`; later cuts use `changie batch auto`), stamps `[workspace.package] version` in the root `Cargo.toml`, commits `Release vX.Y.Z` as `github-actions[bot]`, and pushes the tag. It stamps the same version into `app/package.json` (a CI test keeps the two equal). `tauri.conf.json` has no version; Tauri reads `Cargo.toml`.
3. **Release** runs because a tag push made with `GITHUB_TOKEN` does not start workflows by itself (`workflow_run`), and also on a direct `v*` push or a manual dispatch. It opens a draft release, builds `PR-Nexus_<version>_aarch64.dmg` on `macos-latest`, reseals the app (`scripts/reseal-macos-app.sh`), uploads with `gh release upload --clobber`, checks that the DMG is attached, then clears the draft. `--latest` is set only when the tag is the highest semver.
4. Re-running Release is safe: an existing draft is reused, uploads clobber, and an already published release is left in place when the DMG is present.
5. The `homebrew-tap` job then runs `.github/workflows/homebrew-tap.yml` on a GitHub-hosted runner. It renders `Casks/pr-nexus.rb` from `packaging/homebrew/pr-nexus.rb.tmpl` (`scripts/render-homebrew-cask.sh`, using the release's `*_<version>_aarch64.dmg` asset, its sha256 digest, and `productName` from `tauri.conf.json`) and pushes it to [harnesstap/homebrew-tap](https://github.com/harnesstap/homebrew-tap) with a short-lived token from the tap's release GitHub App (variable `HOMEBREW_TAP_APP_CLIENT_ID`, secret `HOMEBREW_TAP_APP_PRIVATE_KEY`; the app is installed only on `homebrew-tap` with Contents read and write). Without them it logs a notice and skips. It skips drafts and prereleases and never downgrades the tap. To backfill or check the setup: `gh workflow run homebrew-tap.yml -f tag=vX.Y.Z`.

`workflow_dispatch` on Tag release and Release accepts `dry_run`. Dry run batches or checks scripts and does not push a tag, build a DMG, or call the Releases API to publish. A real DMG is only produced by a tag cut on `main`.

`APPLE_CERTIFICATE` is unused. The release job has a Developer ID step that stays skipped until that secret exists, and that step fails on purpose so a secret cannot silently change the ad-hoc build. Signing and notarization are future work.

### Install on a Mac

The DMG is ad-hoc signed (`codesign --force --deep --sign -`), not notarized.

1. Preferred: `brew install --cask harnesstap/tap/pr-nexus`, or `gh release download vX.Y.Z -R harnesstap/pr-nexus -p '*.dmg'` (files fetched by `gh` do not get the quarantine flag).
2. After a browser download, macOS may say the app is damaged or from an unidentified developer. Open it once, then System Settings → Privacy & Security → Open Anyway. On macOS 15 and later, Control-click → Open does not bypass that.
3. Or: `xattr -dr com.apple.quarantine /Applications/PR Nexus.app`.

An ad-hoc signature has no stable team id, so macOS asks once per update before the app can read the keychain item `dev.pr-nexus` (one vault entry, so one prompt). Choose Always Allow. That prompt does not exist until the app stores a token; the current build does not.

Smoke test on a clean user account: download the DMG with `gh`, launch PR Nexus, and confirm the window shows Hello. Installing over an older build should still launch.
