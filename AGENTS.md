# AGENTS.md

Guidance for coding agents working in this repository.

## Source of truth

`docs/SPEC.md` is the source of truth for product behavior. When code and the spec disagree, fix one of them in the same pull request. Sections marked **MUST** are acceptance-relevant. Anything marked *Future* is out of scope for v1.

## Private repository

This repository is private. Do not commit company names or company data.

GitHub organizations, Slack workspaces, teams, and channels are chosen by the user in the app and stored locally. Nothing org-specific is hardcoded or committed. Examples and fixtures use neutral placeholders (`acme`, `team-a`, `#team-reviews`). Do not put org, workspace, or channel literals in code, fixtures, or docs beyond those placeholders.

## Changelog

Never edit `CHANGELOG.md` by hand. Every pull request adds a Changie fragment under `.changes/unreleased/`. Kinds are listed in `.changie.yaml`. Use the `no-changelog` label only when a fragment does not apply.

The product version lives only in `[workspace.package] version` in the root `Cargo.toml`. Do not set `version` in `app/src-tauri/tauri.conf.json`. `app/package.json` is private and is not the product version.

## Layout

- `crates/pr-nexus-core` — domain, store, and provider traits. No network dependencies.
- `crates/pr-nexus-providers` — GitHub and Slack implementations. Not built out yet.
- `app/` — React + TypeScript + Vite + Tailwind UI.
- `app/src-tauri` — Tauri v2 shell. It is a workspace member.

Do not implement a later milestone unless the task asks for it. Stub modules exist so the layout compiles; fill them in when that milestone starts.

## Checks

Before pushing, from the repository root:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

On Linux CI the tests also pass `--features keyring-mock` because there is no secret service. The feature is a stub until the keyring vault exists.

Frontend, from `app/`:

```bash
bun install --frozen-lockfile
bun run lint
bun run typecheck
bun run test
```

Bun is pinned in `.bun-version` (1.3.x). Rust is the stable channel in `rust-toolchain.toml`, with rustfmt and clippy.

`cargo deny check` must pass. Do not weaken `deny.toml` to land a dependency without saying why.
