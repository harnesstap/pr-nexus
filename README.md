# PR-Nexus

Desktop app (Tauri + Rust) to track your open GitHub pull requests by project and request or bump reviews from the owning teams on Slack.

The v1 technical specification is [`docs/SPEC.md`](docs/SPEC.md). Agent notes are in [`AGENTS.md`](AGENTS.md). Branch protection, changelog fragments, and the release runbook are in [`CONTRIBUTING.md`](CONTRIBUTING.md).

The app is early. The window shows Hello. Sync, projects, and Slack requests are not built yet.

## Develop

Requirements: Rust stable (rustfmt, clippy) and Bun 1.3.x. Ubuntu builds also need `libwebkit2gtk-4.1-dev`; see `scripts/install-linux-deps.sh`.

```bash
cargo test --workspace --locked
cd app
bun install
bun run test
bun run tauri dev
```

The product version is `[workspace.package] version` in the root `Cargo.toml`.

## Install

v1 ships an unsigned macOS Apple Silicon DMG. Linux and Windows build in CI and are not released.

```bash
gh release download vX.Y.Z -R github-christophe-oudar/pr-nexus -p '*.dmg'
```

Prefer `gh release download`. The DMG is ad-hoc sealed, not notarized, so a browser download is quarantined and the first launch needs **System Settings → Privacy & Security → Open Anyway** (or `xattr -dr com.apple.quarantine /Applications/PR-Nexus.app`). Each update asks once for the keychain entry that holds account tokens. Details are in [CONTRIBUTING.md](CONTRIBUTING.md).

This repository is private. Do not commit company names or company data. GitHub orgs and Slack workspaces are chosen in the app, not stored in the source tree.
