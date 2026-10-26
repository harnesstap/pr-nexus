import { execFile } from "node:child_process";
import { chmod, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { promisify } from "node:util";
import { afterEach, expect, test } from "vitest";

const execFileAsync = promisify(execFile);
// `bun run test` runs with cwd = app/.
const repoRoot = join(process.cwd(), "..");
const resealScript = join(repoRoot, "scripts", "reseal-macos-app.sh");
const prepareScript = join(repoRoot, "scripts", "prepare-release.sh");
const stampScript = join(repoRoot, "scripts", "stamp-release-version.sh");
const assetsScript = join(repoRoot, "scripts", "check-release-assets.sh");
const planScript = join(repoRoot, "scripts", "tag-release-plan.sh");
const dirs: string[] = [];

afterEach(async () => {
  await Promise.all(dirs.splice(0).map((dir) => rm(dir, { recursive: true, force: true })));
});

async function tempDir(prefix: string): Promise<string> {
  const dir = await mkdtemp(join(tmpdir(), prefix));
  dirs.push(dir);
  return dir;
}

async function writeExecutable(path: string, body: string): Promise<void> {
  await writeFile(path, body);
  await chmod(path, 0o755);
}

test("reseal skips on non-Darwin unless required", async () => {
  const root = await tempDir("reseal-skip-");
  const skipped = await execFileAsync("/bin/bash", [resealScript, join(root, "missing")], {
    env: { ...process.env, RESEAL_UNAME: "Linux" },
  });
  expect(skipped.stdout).toContain("skipping on Linux");

  await expect(
    execFileAsync("/bin/bash", [resealScript, join(root, "missing")], {
      env: { ...process.env, RESEAL_UNAME: "Linux", RESEAL_REQUIRE: "1" },
    }),
  ).rejects.toThrow(/required on Darwin/);
});

test("reseal signs nested binaries and recreates the DMG", async () => {
  const root = await tempDir("reseal-ok-");
  const bundle = join(root, "bundle");
  const macosDir = join(bundle, "macos", "PR-Nexus.app", "Contents", "MacOS");
  const dmgDir = join(bundle, "dmg");
  const app = join(bundle, "macos", "PR-Nexus.app");
  const dmg = join(dmgDir, "PR-Nexus_0.1.0_aarch64.dmg");
  const signLog = join(root, "codesign.log");
  const hdiLog = join(root, "hdiutil.log");
  await mkdir(macosDir, { recursive: true });
  await mkdir(dmgDir, { recursive: true });
  await writeFile(join(macosDir, "pr-nexus"), "desktop-bin");
  await writeFile(dmg, "stale-unsealed-dmg");

  await writeExecutable(
    join(root, "codesign"),
    `#!/usr/bin/env bash
set -euo pipefail
echo "$*" >> ${JSON.stringify(signLog)}
if [[ " $* " == *" -dv "* ]]; then
  echo "Identifier=dev.pr-nexus" >&2
  echo "Sealed Resources version=2 rules=13 files=42" >&2
  exit 0
fi
exit 0
`,
  );
  await writeExecutable(
    join(root, "hdiutil"),
    `#!/usr/bin/env bash
set -euo pipefail
echo "$*" >> ${JSON.stringify(hdiLog)}
out="\${@: -1}"
mkdir -p "$(dirname "$out")"
echo sealed-dmg > "$out"
exit 0
`,
  );

  const result = await execFileAsync("/bin/bash", [resealScript, bundle], {
    env: {
      ...process.env,
      PATH: `${root}:/usr/bin:/bin`,
      RESEAL_UNAME: "Darwin",
      RESEAL_REQUIRE: "1",
      RESEAL_CODESIGN: join(root, "codesign"),
      RESEAL_HDIUTIL: join(root, "hdiutil"),
    },
  });
  expect(result.stdout).toContain("sealed PR-Nexus.app (Sealed Resources present)");
  expect(result.stdout).toContain("recreating PR-Nexus_0.1.0_aarch64.dmg from sealed app");

  const signed = (await readFile(signLog, "utf8")).trim().split("\n");
  expect(signed.some((line) => line.includes("pr-nexus") && line.includes("--force"))).toBe(true);
  expect(signed.some((line) => line.includes("--deep") && line.includes(app))).toBe(true);
  const hdi = await readFile(hdiLog, "utf8");
  expect(hdi).toContain("-volname PR-Nexus");
  expect(hdi).toContain("-format UDZO");
  expect(await readFile(dmg, "utf8")).toBe("sealed-dmg\n");
});

test("reseal fails when the seal is still missing", async () => {
  const root = await tempDir("reseal-none-");
  const bundle = join(root, "bundle");
  const macosDir = join(bundle, "macos", "PR-Nexus.app", "Contents", "MacOS");
  await mkdir(macosDir, { recursive: true });
  await mkdir(join(bundle, "dmg"), { recursive: true });
  await writeFile(join(macosDir, "pr-nexus"), "desktop-bin");
  await writeFile(join(bundle, "dmg", "PR-Nexus_0.1.0_aarch64.dmg"), "stale");
  await writeExecutable(
    join(root, "codesign"),
    `#!/usr/bin/env bash
if [[ " $* " == *" -dv "* ]]; then
  echo "Sealed Resources=none" >&2
  exit 0
fi
exit 0
`,
  );
  await expect(
    execFileAsync("/bin/bash", [resealScript, bundle], {
      env: {
        ...process.env,
        RESEAL_UNAME: "Darwin",
        RESEAL_CODESIGN: join(root, "codesign"),
      },
    }),
  ).rejects.toThrow(/Sealed Resources=none/);
});

test("stamp rewrites only the workspace package version", async () => {
  const root = await tempDir("stamp-");
  await writeFile(
    join(root, "Cargo.toml"),
    `[workspace]
members = ["crates/pr-nexus-core"]

[workspace.package]
version = "0.1.0"
edition = "2024"

[workspace.dependencies]
serde = "1"
`,
  );
  await execFileAsync("/bin/bash", [stampScript, "v0.2.0"], {
    env: { ...process.env, STAMP_RELEASE_ROOT: root, STAMP_SKIP_CARGO: "1" },
  });
  const text = await readFile(join(root, "Cargo.toml"), "utf8");
  expect(text).toContain('version = "0.2.0"');
  expect(text).toContain('serde = "1"');
});

test("release asset check requires the arm64 DMG only", async () => {
  const ok = await execFileAsync("/bin/bash", [
    assetsScript,
    "v0.1.0",
    "PR-Nexus_0.1.0_aarch64.dmg",
    "notes.txt",
  ]);
  expect(ok.stdout).toContain("PR-Nexus_0.1.0_aarch64.dmg");
  await expect(
    execFileAsync("/bin/bash", [assetsScript, "0.1.0", "PR-Nexus_0.1.0_x64.dmg"]),
  ).rejects.toThrow(/missing PR-Nexus_0.1.0_aarch64.dmg/);
});

test("release dry run checks the version without calling gh", async () => {
  const result = await execFileAsync("/bin/bash", [prepareScript], {
    env: { ...process.env, DRY_RUN: "1", PREPARE_RELEASE_ROOT: repoRoot },
  });
  expect(result.stdout).toContain("dry_run=true");
  expect(result.stdout).toContain("skip=true");
  expect(result.stdout).toContain("release assets ok");
});

test("tag release plan batches v0.1.0 when nothing has shipped", async () => {
  const root = await tempDir("plan-");
  await mkdir(join(root, ".changes", "unreleased"), { recursive: true });
  await writeFile(
    join(root, "Cargo.toml"),
    '[workspace.package]\nversion = "0.1.0"\n',
  );
  await writeFile(
    join(root, ".changes", "unreleased", "Features-bootstrap.yaml"),
    "kind: Features\nbody: Bootstrap\ntime: 2026-10-26T00:00:00Z\n",
  );
  const result = await execFileAsync("/bin/bash", [planScript], {
    env: { ...process.env, TAG_RELEASE_ROOT: root, CHANGIE_LATEST: "" },
  });
  expect(result.stdout).toContain("has_unreleased=true");
  expect(result.stdout).toContain("batch_args=v0.1.0");
  expect(result.stdout).toContain("action=prepare");
});
