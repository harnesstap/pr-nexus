# PR Nexus agent guide

This repository is **private**. Never commit company names, organization names, Slack workspace names, tokens, customer data, or any other company data. GitHub orgs and Slack workspaces are chosen by the user at runtime. Nothing in source, fixtures, or docs may hardcode them.

`docs/SPEC.md` is the source of truth for product behavior. When code and the spec disagree, fix one of them in the same change. Do not implement a later milestone than the one you were asked for.

## Changelog

Use Changie. Add one fragment per pull request:

```bash
changie new
```

Fragments live in `.changes/unreleased/`. Never edit `CHANGELOG.md` by hand. Skip the fragment only when the pull request has the `no-changelog` label.

## Layout

- `crates/pr-nexus-core` — domain model, errors, the provider registry, and the SQLite store. No network dependencies.
- `crates/pr-nexus-providers` — GitHub (blocking GraphQL sync) and Slack factories. Slack stays an empty module until M10. The spec uses Slack's official remote MCP server at <https://mcp.slack.com/mcp> (`rmcp`, OAuth). Do not add that client, a Web API client, tokens, or scopes before M10.
- `app/` — React, TypeScript, Vite, and Tailwind.
- `app/src-tauri` — Tauri v2 shell. It is a Cargo workspace member.

## Checks

From the repo root:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

The Tauri crate embeds `app/dist`, so build the frontend before `cargo test` or `cargo clippy`:

```bash
cd app && bun install && bun run build
```

From `app/`:

```bash
bun run lint
bun run typecheck
bun run test
bun run tauri dev
```

Toolchain: Rust stable (`rust-toolchain.toml`, with rustfmt and clippy) and Bun 1.3.14.

## Style

- Put imports at the top of the file. No inline imports.
- In TypeScript, switches over discriminated unions and enums need a `never` check in the default branch.
- Rust matches on enums stay exhaustive.
- No blocking work in sync Tauri commands. Tauri v2 runs a plain `#[tauri::command] pub fn` on the main thread, so one slow network call freezes the window. Every command is `pub async fn`; HTTP, Slack MCP, keychain, SQLite and file work goes through `crate::blocking::run` (see `app/src-tauri/src/blocking.rs`). `blocking::tests::every_command_is_async` fails on a sync command; add any new command file to its `SOURCES`.
