# PR-Nexus

Desktop app (Tauri + Rust) to track your open GitHub pull requests by project and request or bump reviews from the owning teams on Slack.

This repository is private. The v1 technical specification is [docs/SPEC.md](docs/SPEC.md). Agent instructions are in [AGENTS.md](AGENTS.md). The release and branch-ruleset runbook is in [CONTRIBUTING.md](CONTRIBUTING.md).

The current build opens a window that says Hello. GitHub sync, Slack, and the project list are not in this build.

## Develop

- Rust stable (`rust-toolchain.toml`)
- Bun 1.3.14

```bash
cd app
bun install
bun run tauri dev
```

Linux needs the WebKitGTK packages listed in `.github/actions/linux-tauri-deps/action.yml`.

## Install a release

v1 publishes an unsigned, ad-hoc-sealed macOS Apple Silicon DMG only. Linux and Windows compile in CI and are not released.

Preferred, because `gh` does not set the quarantine flag:

```bash
gh release download vX.Y.Z -R harnesstap/pr-nexus -p '*.dmg'
```

A browser download can be blocked by Gatekeeper ("cannot verify the developer" or "damaged"). Open the app once, then allow it under System Settings → Privacy & Security → Open Anyway. Alternatively:

```bash
xattr -dr com.apple.quarantine /Applications/PR-Nexus.app
```

Each new build asks once for permission to use the keychain item `dev.pr-nexus`. The Hello build does not read the keychain yet. Details are in CONTRIBUTING.
