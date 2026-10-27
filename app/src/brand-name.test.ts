import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { expect, test } from "vitest";

// `bun run test` runs with cwd = app/.
const repoRoot = join(process.cwd(), "..");

/**
 * The product is "PR Nexus". Identifiers keep the dash-free lowercase form
 * (`pr-nexus`, `pr_nexus_*`, `dev.pr-nexus`), and the release asset keeps the
 * stable name `PR-Nexus_<version>_aarch64.dmg`. Released changelog entries are
 * history and are not rewritten.
 */
const SKIPPED = [/^app\/src\/brand-name\.test\.ts$/, /^CHANGELOG\.md$/, /^\.changes\//, /\.lock$/, /(^|\/)bun\.lockb?$/, /\.(png|jpe?g|icns|ico|gif|webp)$/];

test("no user-facing text says PR-Nexus", () => {
  const files = execFileSync("git", ["ls-files", "-z"], { cwd: repoRoot, encoding: "utf8" })
    .split("\0")
    .filter((file) => file && !SKIPPED.some((pattern) => pattern.test(file)));
  const hits: string[] = [];
  for (const file of files) {
    let text: string;
    try {
      text = readFileSync(join(repoRoot, file), "utf8");
    } catch {
      continue; // deleted in the working tree
    }
    text.split("\n").forEach((line, index) => {
      if (/PR-Nexus(?!_)/.test(line)) hits.push(`${file}:${index + 1}: ${line.trim()}`);
    });
  }
  expect(hits).toEqual([]);
});

test("the app, window and reseal script use the product name", () => {
  const conf = JSON.parse(readFileSync(join(repoRoot, "app/src-tauri/tauri.conf.json"), "utf8")) as {
    productName: string;
    identifier: string;
    app: { windows: { title: string }[] };
  };
  expect(conf.productName).toBe("PR Nexus");
  expect(conf.app.windows.map((window) => window.title)).toEqual(["PR Nexus"]);
  // The identifier names the data folder, database and keychain item: never rename it.
  expect(conf.identifier).toBe("dev.pr-nexus");
  const reseal = readFileSync(join(repoRoot, "scripts/reseal-macos-app.sh"), "utf8");
  expect(reseal).toContain(`APP_NAME="${conf.productName}"`);
});
