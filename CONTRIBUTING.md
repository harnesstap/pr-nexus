# Contributing

PR-Nexus is a private desktop app. Do not commit company names or company data. GitHub orgs and Slack workspaces are user configuration, not source. Fixtures use placeholders (`acme`, `team-a`, `#team-reviews`).

The product behavior lives in [`docs/SPEC.md`](docs/SPEC.md). [`AGENTS.md`](AGENTS.md) is the short version for coding agents.

## Setup

- Rust stable, with rustfmt and clippy (`rust-toolchain.toml`).
- [Bun](https://bun.sh) 1.3.x (`.bun-version`).
- On Ubuntu 24.04, the Tauri shell needs the packages in `scripts/install-linux-deps.sh` (`libwebkit2gtk-4.1-dev` and friends).

```bash
cargo test --workspace --locked
cd app && bun install && bun run test
cd app && bun run tauri dev
```

Linux and Windows must build in CI. v1 does not ship installers for them.

## Changelog

Add a fragment for every pull request. Do not edit `CHANGELOG.md` by hand.

```bash
# or write .changes/unreleased/<name>.yaml with kind, body, and time
changie new
```

Kinds: Breaking (major), Features and Added (minor), Changed, Fixes, Fixed, Docs, and Dependencies (patch). Label a pull request `no-changelog` only when a fragment does not apply. CI checks that a fragment exists.

## Pull requests and the main branch

CI is merge-queue friendly. The single required check is the `validate` job in `.github/workflows/ci.yml`. It fails when any needed job failed or was cancelled. Skipped jobs (docs-only path filters) are fine. Path filters keep docs-only pulls cheap; `workflow_dispatch` runs the full set.

This repository does **not** create branch rulesets from code or the API. Configure them in GitHub:

**Ruleset `main-ci-required`** on the default branch `main`:

- Restrict deletions.
- Block force pushes.
- Require the status check `validate` (the CI job of that name).
- Require a merge queue:
  - merge method: squash
  - grouping strategy: ALLGREEN
  - maximum entries to build: 5
  - maximum entries to merge: 5
- Bypass the ruleset for `github-actions[bot]` only, so "Tag release" can push the release commit and tag. People do not get a bypass.

Also turn on **Settings → General → Pull Requests → Automatically delete head branches**.

The merge queue needs the `merge_group` trigger already present on CI. Do not require any check other than `validate`.

## Release

v1 publishes one asset: an ad-hoc sealed macOS arm64 DMG. Linux and Windows are build-checked only. There is no auto-updater and no notarization.

1. Merge pull requests that include Changie fragments.
2. On `main`, run **Actions → Tag release**. Leave "dry run" off. The workflow batches the changelog, stamps `[workspace.package] version`, commits `Release vX.Y.Z` as `github-actions[bot]`, and pushes the tag. A dry run batches and stamps on the runner, uploads a result marked dry-run, and pushes nothing. Release then exits without checking out a tag.
3. **Release** starts when Tag release succeeds (a `GITHUB_TOKEN` tag push does not start workflows by itself). It opens a draft, builds the DMG, ad-hoc seals it, uploads with `--clobber`, checks the asset name, and publishes. Re-running Release is idempotent.
4. To exercise the resolver without publishing, dispatch **Release** with the tag and "dry run" enabled. That path checks the tag and the Cargo.toml version only.
5. Smoke-test on a clean macOS user: `gh release download` → first launch → add accounts → install over the previous version and confirm exactly one keychain prompt.

Expected asset: `PR-Nexus_<version>_aarch64.dmg`. `.app.tar.gz` and `.sig` are for a future updater and are not required.

Developer ID signing stays dormant. The Release job calls `scripts/sign-macos-developer-id.sh` only when `APPLE_CERTIFICATE` is set, and that script fails until signing is implemented. Leave the secret unset.

### Unsigned macOS builds

The DMG is ad-hoc sealed (`codesign --force --deep --sign -`), not notarized. A browser download gets the quarantine flag and macOS may refuse the first launch ("cannot verify the developer" or "damaged").

1. Preferred: `gh release download vX.Y.Z -R github-christophe-oudar/pr-nexus -p '*.dmg'`. The repo is private, and files fetched by `gh` do not get a quarantine flag.
2. After a browser download, try to open the app once, then System Settings → Privacy & Security → **Open Anyway**. Control-click → Open does not bypass this on macOS 15 or later.
3. Or `xattr -dr com.apple.quarantine /Applications/PR-Nexus.app`.

An ad-hoc signature has no stable team identity, so the keychain "Always Allow" rule is tied to the binary hash. Each update asks once for the single `dev.pr-nexus` / `vault` keychain entry. Choose **Always Allow**. Denying the prompt leaves accounts as "token unavailable" once that UI exists; nothing is wiped. The prompt is one per update because every token lives in that single entry.

## Supply chain

Dependabot opens weekly grouped updates for Cargo, Bun, and GitHub Actions. `cargo deny check` covers advisories, licenses, and bans.
