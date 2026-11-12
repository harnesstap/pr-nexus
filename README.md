# PR Nexus

[![Latest release](https://img.shields.io/github/v/release/harnesstap/pr-nexus?sort=semver)](https://github.com/harnesstap/pr-nexus/releases/latest)
[![Platforms: macOS, Linux, Windows](https://img.shields.io/badge/platforms-macOS%20%7C%20Linux%20%7C%20Windows-black)](#install)
[![License: MIT](https://img.shields.io/github/license/harnesstap/pr-nexus)](LICENSE)

All your open GitHub pull requests in one desktop app, grouped by project. Ask the right team for a review on Slack in one click.

![PR Nexus demo](docs/media/pr-nexus-demo.webp)

## What it does

- **One list for your PRs.** It shows the PRs you opened and the ones waiting for your review, across the GitHub orgs you pick, and you can save your own views. It refreshes in the background.
- **Grouped by project.** You put PRs into projects by hand or with rules (repo, label, branch, title, ticket key), and can link projects to Jira.
- **Spot stuck PRs.** Quick filters and row cues show failing checks, conflicts, drafts, requested changes and reviews that have waited too long.
- **Review requests on Slack.** PR Nexus finds the owning teams from CODEOWNERS, maps each team to its Slack channel, and previews the message before it sends it. Later you can bump the same thread. It also works in bulk across several PRs.
- **Act on a PR without leaving the app.** You can mark a PR ready, convert it to draft, toggle auto-merge, update the branch, and keep notes and stars.
- **Keyboard first.** <kbd>⌘K</kbd> (<kbd>Ctrl+K</kbd> on Linux and Windows) opens the command palette. Press <kbd>?</kbd> to list the shortcuts.
- **Works with AI assistants.** An optional local MCP server lets Cursor, VS Code and other MCP clients read your PRs and, if you allow it, propose Slack messages for you to approve.

![The PR list grouped by project, with quick status filters and the Slack review status of each PR](docs/assets/screenshot-list.png)

## Install

Downloads for macOS (Apple Silicon), Linux (x86_64, arm64) and Windows (x64) are on the [Releases](https://github.com/harnesstap/pr-nexus/releases/latest) page, with a `SHA256SUMS` file. Linux and Windows builds start with v0.1.1. The builds are not notarized or code signed yet, so each OS warns you the first time you open the app (see [Troubleshooting](#troubleshooting)).

### macOS

```bash
brew install --cask harnesstap/tap/pr-nexus
```

Upgrade with `brew upgrade --cask pr-nexus`. Or download the `.dmg` from Releases.

### Linux

```bash
curl -fsSL https://raw.githubusercontent.com/harnesstap/pr-nexus/main/scripts/install.sh | sh
```

The script installs the `.deb` (Debian, Ubuntu) or the `.rpm` (Fedora, RHEL, openSUSE). On other distributions it installs the AppImage in `~/.local/bin`. It checks every download against `SHA256SUMS`, and running it again upgrades the app. Set `PR_NEXUS_DRY_RUN=1` to see what it would do, or `PR_NEXUS_METHOD=deb|rpm|appimage` to choose the package type.

You can also install a package from Releases yourself:

```bash
sudo apt install ./PR-Nexus_X.Y.Z_amd64.deb          # Debian, Ubuntu
sudo dnf install ./PR-Nexus-X.Y.Z-1.x86_64.rpm       # Fedora, RHEL
chmod +x PR-Nexus_X.Y.Z_amd64.AppImage && ./PR-Nexus_X.Y.Z_amd64.AppImage
```

On ARM64, use the `_arm64.deb`, `-1.aarch64.rpm` and `_aarch64.AppImage` files instead.

### Windows

Download `PR-Nexus_X.Y.Z_x64-setup.exe` (installs for your user only) or `PR-Nexus_X.Y.Z_x64_en-US.msi` from Releases. A WinGet package is coming soon.

## Getting started

On first launch, a short setup wizard walks you through these steps:

1. **Connect GitHub.** Use your `gh` CLI login, paste a personal access token, or sign in with the device flow.
2. **Pick the orgs to track**, and choose whether to include your personal repos.
3. **Connect Slack (optional).** Sign in to one or more workspaces. You can skip this step and do it later in Settings.
4. **Import a team → channel mapping (optional).** Import a CSV such as `acme,web-team,Web Team,web-reviews,C0123456789`, or skip it and pick channels as you go.

Then the first sync runs. Create projects from the sidebar, and drag PRs into them or add rules. To run the setup again, open Settings → About.

## Profiles

Keep work and side projects apart with profiles. Each profile has its own GitHub account, Slack workspaces, projects, templates, agent send policy and activity. Theme and other preferences are shared. Switch profiles, or add one, from the account menu at the bottom left. A new profile can copy items from your other profiles and then runs the setup steps for the rest. Copies are independent of the original. If `gh` is logged in to several accounts, you can pick which one a profile uses. Rename or delete profiles in **Settings → Profiles**.

## Using with AI assistants

The MCP server is off by default. To turn it on, open **Settings → Agents**. The server listens only on `127.0.0.1`, and clients need a token to connect. The same screen shows the config to paste into your client (Cursor, VS Code, Claude Code, …). You choose what agents can do:

- read PRs and projects;
- write local data (notes, stars, projects);
- read Slack in the workspaces you expose;
- send Slack messages, including review requests for several PRs at once (`request_reviews`).

Slack messages wait for your approval unless you allow specific channels. Every call is listed in the activity log on that screen. Its **Stats** tab shows how fast your own PRs get a first review and who you ask most (7, 30 or 90 days, per repo or org); agents can read the same numbers with `get_review_stats`. Tools act on the active profile unless the agent passes a `profile`.

## Privacy and security

Tokens are stored in your OS keychain: the macOS Keychain, the Linux Secret Service, or the Windows Credential Manager. Everything else stays in a local database on your computer. PR Nexus talks only to GitHub and to the Slack workspaces you connect, and it sends no telemetry.

## Troubleshooting

- **macOS says the app can't be opened or is damaged.** Open **System Settings → Privacy & Security** and click **Open Anyway** once. If you download the DMG with `gh release download`, macOS does not show this warning.
- **macOS asks to use the keychain item `dev.pr-nexus`.** Click **Always Allow**. The prompt can come back once after an update.
- **Linux shows a keyring banner and tokens are not saved.** A Secret Service is needed (GNOME Keyring or KWallet). Install and start one, then click Retry.
- **Windows shows "Windows protected your PC".** Click **More info**, then **Run anyway**.
- **A GitHub org is missing or shows an SSO hint.** Authorize your token for that org's SAML SSO on GitHub, then run the setup again.

## Contributing

To build from source or work on the code, read [CONTRIBUTING.md](CONTRIBUTING.md).

## License

MIT. See [LICENSE](LICENSE).
