// Renders the real app over mocked Tauri IPC (screenshot.html) and saves the
// README screenshot. Usage, from app/: bun run screenshot
// `bun run screenshot -- --detail <path>` opens #479's detail panel and saves
// that view to <path> instead; `--draft <path>` opens draft #488, which has
// no Slack request yet.
// `--width <px>` sets the viewport width (default 1440); `--out <path>` saves the
// list view somewhere other than the README asset. `--tooltips <dir>` hovers a
// few controls and saves tooltip-*.png crops showing the app tooltip open.
// `--scenes <dir>` saves the project-info and star views (project dialog, JIRA
// chips, note and star, starred pinned, Starred filter, compact rows, Settings).
// Uses an installed Chrome; set CHROME_PATH to pick another Chromium binary.
import { mkdirSync, statSync } from "node:fs";
import { dirname, resolve } from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright-core";
import { createServer } from "vite";

const appDir = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const tooltipsFlag = process.argv.indexOf("--tooltips");
const tooltipsDir = tooltipsFlag === -1 ? null : resolve(process.argv[tooltipsFlag + 1] ?? "");
const detailFlag = process.argv.indexOf("--detail");
const draftFlag = process.argv.indexOf("--draft");
const draftOut = draftFlag === -1 ? null : process.argv[draftFlag + 1];
if (draftFlag !== -1 && !draftOut) {
  throw new Error("--draft needs an output path");
}
const detailOut = detailFlag === -1 ? draftOut : process.argv[detailFlag + 1];
if (detailFlag !== -1 && !detailOut) {
  throw new Error("--detail needs an output path");
}
const widthFlag = process.argv.indexOf("--width");
const width = widthFlag === -1 ? 1440 : Number(process.argv[widthFlag + 1]);
if (!Number.isInteger(width) || width < 320) {
  throw new Error("--width needs a viewport width in px");
}
const outFlag = process.argv.indexOf("--out");
const listOut = outFlag === -1 ? null : process.argv[outFlag + 1];
if (outFlag !== -1 && !listOut) {
  throw new Error("--out needs an output path");
}
const out = resolve(detailOut ?? listOut ?? resolve(appDir, "../docs/assets/screenshot-list.png"));
const scenesFlag = process.argv.indexOf("--scenes");
const scenesDir = scenesFlag === -1 ? null : process.argv[scenesFlag + 1];
if (scenesFlag !== -1 && !scenesDir) {
  throw new Error("--scenes needs an output directory");
}
/** A row's own button, not its star (both carry the title). */
const rowButton = (title) => new RegExp(`^(?!Star |Unstar |Open ).*${title}`);
const port = Number(process.env.SCREENSHOT_PORT ?? 1430);
const url = `http://127.0.0.1:${port}/screenshot.html`;

const vite = await createServer({
  root: appDir,
  server: { port, strictPort: true, host: "127.0.0.1" },
  logLevel: "warn",
});
await vite.listen();

let browser;
try {
  browser = await chromium.launch(process.env.CHROME_PATH ? { executablePath: process.env.CHROME_PATH } : { channel: "chrome" });
  const page = await browser.newPage({ viewport: { width, height: 900 }, deviceScaleFactor: 2, colorScheme: "dark" });
  page.on("pageerror", (err) => console.error("page error:", err.message));
  await page.goto(url);
  await page.getByRole("group", { name: "Quick filters" }).waitFor();
  if (tooltipsDir) {
    await shootTooltips(page, tooltipsDir);
  } else if (scenesDir) {
    await scenes(page, resolve(scenesDir));
  } else if (detailOut) {
    await page
      .getByRole("button", {
        name: rowButton(draftOut ? "Show saved cards first" : "Add address autocomplete to checkout"),
      })
      .first()
      .click();
    await page
      .getByTestId("detail-panel")
      .getByRole("button", { name: /^Checks:/ })
      .waitFor();
    if (draftOut) {
      await page.getByTestId("detail-panel").getByRole("button", { name: "Request review on Slack" }).waitFor();
    } else {
      await page
        .getByTestId("detail-panel")
        .getByRole("button", { name: /^Checks:/ })
        .click();
    }
  }
  if (!tooltipsDir && !scenesDir) {
    await page.waitForTimeout(500);
    await page.screenshot({ path: out });
    console.log(`wrote ${out} (${Math.round(statSync(out).size / 1024)} KB)`);
  }
} finally {
  await browser?.close();
  await vite.close();
}

/** Hovers each target, waits for the app tooltip and saves a crop around both. */
async function shootTooltips(page, dir) {
  const list = page.getByRole("article");
  const targets = [
    ["slack-cell", () => list.getByRole("button", { name: /^Open Slack request for/ }).first()],
    ["status-pill", () => list.getByText(/^(Draft|Conflicts)$/).first()],
    ["toolbar-chip", () => page.getByRole("group", { name: "Quick filters" }).getByRole("button").first()],
    [
      "detail-icon-button",
      async () => {
        await page
          .getByRole("button", { name: rowButton("Add address autocomplete to checkout") })
          .first()
          .click();
        await page
          .getByTestId("detail-panel")
          .getByRole("button", { name: /^Checks:/ })
          .waitFor();
        return page.getByTestId("detail-panel").getByRole("button", { name: "Close details" });
      },
    ],
  ];
  for (const [name, find] of targets) {
    console.log(`hovering ${name}`);
    const target = await find();
    await page.mouse.move(0, 0);
    await page.waitForTimeout(100);
    await target.hover();
    const tip = page.locator('[role="tooltip"]:not([hidden])');
    await tip.waitFor();
    await page.waitForTimeout(150);
    const a = await target.boundingBox();
    const b = await tip.boundingBox();
    const pad = 24;
    const x = Math.max(0, Math.min(a.x, b.x) - pad);
    const y = Math.max(0, Math.min(a.y, b.y) - pad);
    const clip = { x, y, width: Math.max(a.x + a.width, b.x + b.width) + pad - x, height: Math.max(a.y + a.height, b.y + b.height) + pad - y };
    const file = resolve(dir, `tooltip-${name}.png`);
    await page.screenshot({ path: file, clip });
    console.log(`wrote ${file}`);
  }
}

async function shot(page, path) {
  await page.waitForTimeout(500);
  await page.screenshot({ path });
  console.log(`wrote ${path} (${Math.round(statSync(path).size / 1024)} KB)`);
}

async function scenes(page, dir) {
  mkdirSync(dir, { recursive: true });
  await shot(page, resolve(dir, "list-starred-pinned.png"));

  await detailScenes(page, dir);

  await page.getByRole("button", { name: "Filter menu" }).click();
  const menu = page.getByRole("menu");
  await menu.getByRole("button", { name: /^Grouping/ }).click();
  await menu.getByRole("button", { name: /^(✓ )?Project$/ }).click();
  await page.getByRole("button", { name: "Filter menu" }).click();
  await page.getByText("ACME-123").first().waitFor();
  await shot(page, resolve(dir, "project-jira-chips.png"));

  await page.getByRole("button", { name: "Actions for Checkout redesign" }).click();
  await page.getByRole("button", { name: "Edit auto-link rules" }).click();
  await page.getByRole("textbox", { name: "Project description" }).waitFor();
  await shot(page, resolve(dir, "project-dialog-description-jira.png"));
  await page.keyboard.press("Escape");

  await page
    .getByRole("button", {
      name: rowButton("Add address autocomplete to checkout"),
    })
    .first()
    .click();
  await page.getByTestId("detail-panel").getByRole("textbox", { name: "Note for this pull request" }).waitFor();
  await page
    .getByTestId("detail-panel")
    .getByText("Waiting on Sam")
    .waitFor()
    .catch(() => undefined);
  await shot(page, resolve(dir, "detail-note-star-compact-rows.png"));
  await page.keyboard.press("Escape");

  await page.getByRole("button", { name: "Filter menu" }).click();
  await page.getByRole("menu").getByText("Starred only").click();
  await page.getByRole("button", { name: "Filter menu" }).click();
  await shot(page, resolve(dir, "starred-filter.png"));

  await page
    .getByRole("button", { name: /Settings/ })
    .first()
    .click();
  await page.getByRole("tab", { name: "Review" }).click();
  await page.getByRole("textbox", { name: "JIRA base URL" }).scrollIntoViewIfNeeded();
  await shot(page, resolve(dir, "settings-jira-base-url.png"));
}

/** The merge card's draft action for a draft and a ready PR, and the stat cards at two widths. */
async function detailScenes(page, dir) {
  const panel = page.getByTestId("detail-panel");
  await page
    .getByRole("button", { name: rowButton("Show saved cards first") })
    .first()
    .click();
  await panel.getByRole("button", { name: "Mark ready for review" }).waitFor();
  await shot(page, resolve(dir, "detail-draft-draft.png"));
  await panel.getByTestId("merge-verdict").screenshot({ path: resolve(dir, "detail-merge-draft-default.png") });
  await page.setViewportSize({ width: 760, height: 900 });
  await page.waitForTimeout(400);
  await shot(page, resolve(dir, "detail-draft-draft-narrow.png"));
  await panel.getByTestId("merge-verdict").screenshot({ path: resolve(dir, "detail-merge-draft-narrow.png") });
  console.log(`merge card draft narrow: ${Math.round((await panel.getByTestId("merge-verdict").boundingBox()).width)}px`);
  await page.setViewportSize({ width, height: 900 });
  await page.waitForTimeout(400);
  await page.keyboard.press("Escape");

  await page
    .getByRole("button", {
      name: rowButton("Add address autocomplete to checkout"),
    })
    .first()
    .click();
  await panel.getByRole("button", { name: "Convert to draft" }).waitFor();
  await shot(page, resolve(dir, "detail-draft-ready.png"));
  await panel.getByTestId("merge-verdict").screenshot({ path: resolve(dir, "detail-merge-ready-default.png") });
  await page.setViewportSize({ width: 760, height: 900 });
  await page.waitForTimeout(400);
  await shot(page, resolve(dir, "detail-draft-ready-narrow.png"));
  await panel.getByTestId("merge-verdict").screenshot({ path: resolve(dir, "detail-merge-ready-narrow.png") });
  console.log(`merge card ready narrow: ${Math.round((await panel.getByTestId("merge-verdict").boundingBox()).width)}px`);
  await page.setViewportSize({ width, height: 900 });
  await page.waitForTimeout(400);
  await panel.getByTestId("stat-cards").screenshot({ path: resolve(dir, "detail-cards-default.png") });
  console.log("wrote detail-cards-default.png");

  const slack = panel.getByRole("region", { name: "Slack request" });
  await slack.screenshot({ path: resolve(dir, "detail-slack-default.png") });
  await page.setViewportSize({ width: 900, height: 900 });
  await page.waitForTimeout(500);
  await shot(page, resolve(dir, "detail-cards-narrow.png"));
  await slack.screenshot({ path: resolve(dir, "detail-slack-narrow.png") });
  await page.setViewportSize({ width, height: 900 });
  await page.keyboard.press("Escape");

  // Each Slack card state at the default and a narrow (~360px card) width.
  for (const variant of ["requested", "bumpable", "replies", "long-channel"]) {
    await page.evaluate((name) => {
      globalThis.__slackVariant = name;
    }, variant);
    for (const [label, size] of [
      ["default", width],
      ["narrow", 760],
    ]) {
      await page.setViewportSize({ width: size, height: 900 });
      await page
        .getByRole("button", { name: rowButton("Add address autocomplete to checkout") })
        .first()
        .click();
      const card = page.getByRole("region", { name: "Slack request" });
      await card.waitFor();
      await page.waitForTimeout(300);
      await card.screenshot({ path: resolve(dir, `slack-card-${variant}-${label}.png`) });
      console.log(`wrote slack-card-${variant}-${label}.png (${Math.round((await card.boundingBox()).width)}px)`);
      await page.keyboard.press("Escape");
    }
  }
  await page.setViewportSize({ width, height: 900 });
  await page.keyboard.press("Escape");
}
