# WinGet manifests

Templates for the `harnesstap.PRNexus` package in [microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs). Nothing is submitted automatically yet.

Render a release's manifests from its `SHA256SUMS`:

```bash
gh release download vX.Y.Z -R harnesstap/pr-nexus -p SHA256SUMS -D /tmp/sums
scripts/render-winget-manifests.sh vX.Y.Z /tmp/sums/SHA256SUMS out/
winget validate --manifest out/manifests/h/harnesstap/PRNexus/X.Y.Z   # on Windows
```

The output path follows the winget-pkgs layout (`manifests/h/harnesstap/PRNexus/<version>/`).

Submission: the Release workflow's `winget` job runs `komac update` for each published stable tag only when the `WINGET_TOKEN` secret (a classic PAT with `public_repo` on a fork of winget-pkgs) is set. It is not set, so the job logs a notice and skips. The first version of a new package must be reviewed by winget-pkgs moderators; submit it by hand (`komac new` or a PR with the rendered files) before enabling the job.
