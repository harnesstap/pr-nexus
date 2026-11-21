// Renders the real app over mocked Tauri IPC (screenshot.html) and saves the
// README screenshot. Usage, from app/: bun run screenshot
// `bun run screenshot -- --detail <path>` opens #479's detail panel and saves
// that view to <path> instead; `--draft <path>` opens draft #488, which has
// no Slack request yet. `--agree <path>` opens #479 after a detail refresh
// so the list row and the panel show the same review, checks, and merge state.
// `--width <px>` sets the viewport width (default 1440); `--out <path>` saves the
// list view somewhere other than the README asset. `--tooltips <dir>` hovers a
// few controls and saves tooltip-*.png crops showing the app tooltip open.
// `--scenes <dir>` saves the project-info and star views (project dialog, JIRA
// chips, note and star, starred pinned, Starred filter, compact rows, Settings).
// `--branch <dir>` saves the Branch card states (branch-*.png) into <dir>: one
// PR per state, the Update branch menu open, and the rebase confirmation.
// `--agents-tools <dir>` saves Settings → MCP server → Permissions tool switches (agents-tools-*.png
// and agents-workspace-signin.png): collapsed, one category with tools off,
// a category off with its tools greyed, and Sign in again on two workspaces.
// `--slack-draft <dir>` saves the Slack dialogs' draft warning (slack-draft-*.png):
// the single request for draft #488 (default and narrow), after Mark ready, and
// the bulk dialog with #488 and #58 as drafts, before and after Mark all ready.
// `--repo-filter <dir>` saves the repository picker (repo-filter-*.png): the
// popover filtered to widgets, both widget repositories selected, a 720px
// toolbar, and the alphabetically sorted project list.
// `--ready-preview <dir>` saves the bulk dialog before and after Mark all ready
// when two selected drafts are CI-blocked and only one of those checks clears.
// `--github-query <dir>` saves the GitHub query field (github-query-*.png):
// matching rows, a rejected query, and the same list at 720px.
// `--pr-link <dir>` saves a pasted pull request URL (pr-link-*.png): a stored
// pull request from another view, and one read from GitHub for the panel.
// `--people-filters <dir>` saves the reviewer-team and assignee menus, then
// the list after those menus write qualifiers into the search field.
// `--update <dir>` saves the sidebar's update indicator (update-*.png): none,
// the icon for a newer release, and its popover for a release and a dev build.
// `--merge-queue <dir>` saves the merge queue preview (merge-queue-*.png):
// the list, the add confirmation, the bulk dialog, and a narrow list.
// `--views <dir>` saves custom views (views-*.png): the sidebar at rest,
// one row hovered, and the … menu open.
// `--sidebar-tree <dir>` saves parent views and collapsible section headers
// (sidebar-tree.png, sidebar-new-view.png, sidebar-edit-parent.png,
// sidebar-views-collapsed.png, sidebar-projects-collapsed.png).
// `--view-drag <dir>` saves dragging a view onto a parent, the confirm,
// the tree after the move, and the Move to submenu.
// `--close-pr <dir>` saves Close PR (close-*.png): the detail menu, the
// confirmation with a comment, the bulk confirmation, and the narrow bar menu.
// `--settings-nav <dir>` saves the settings rail (settings-nav-*.png): Slack,
// Review, MCP server with Reads expanded, and the section menu below 960px.
// `--settings-story <dir>` saves the merged settings pages, the setup notice,
// and an add left pending on one AI agent.
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
const agreeFlag = process.argv.indexOf("--agree");
const agreeOut = agreeFlag === -1 ? null : process.argv[agreeFlag + 1];
if (agreeFlag !== -1 && !agreeOut) {
  throw new Error("--agree needs an output path");
}
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
const branchFlag = process.argv.indexOf("--branch");
const branchDir = branchFlag === -1 ? null : process.argv[branchFlag + 1];
if (branchFlag !== -1 && !branchDir) {
  throw new Error("--branch needs an output directory");
}
const out = resolve(agreeOut ?? detailOut ?? listOut ?? resolve(appDir, "../docs/assets/screenshot-list.png"));
const scenesFlag = process.argv.indexOf("--scenes");
const scenesDir = scenesFlag === -1 ? null : process.argv[scenesFlag + 1];
if (scenesFlag !== -1 && !scenesDir) {
  throw new Error("--scenes needs an output directory");
}
const slackDraftFlag = process.argv.indexOf("--slack-draft");
const slackDraftDir = slackDraftFlag === -1 ? null : process.argv[slackDraftFlag + 1];
if (slackDraftFlag !== -1 && !slackDraftDir) {
  throw new Error("--slack-draft needs an output directory");
}
const reviewOpenFlag = process.argv.indexOf("--review-open");
const reviewOpenDir = reviewOpenFlag === -1 ? null : process.argv[reviewOpenFlag + 1];
if (reviewOpenFlag !== -1 && !reviewOpenDir) {
  throw new Error("--review-open needs an output directory");
}
const repoFilterFlag = process.argv.indexOf("--repo-filter");
const repoFilterDir = repoFilterFlag === -1 ? null : process.argv[repoFilterFlag + 1];
if (repoFilterFlag !== -1 && !repoFilterDir) {
  throw new Error("--repo-filter needs an output directory");
}
const readyPreviewFlag = process.argv.indexOf("--ready-preview");
const readyPreviewDir = readyPreviewFlag === -1 ? null : process.argv[readyPreviewFlag + 1];
if (readyPreviewFlag !== -1 && !readyPreviewDir) {
  throw new Error("--ready-preview needs an output directory");
}
const githubQueryFlag = process.argv.indexOf("--github-query");
const githubQueryDir = githubQueryFlag === -1 ? null : process.argv[githubQueryFlag + 1];
if (githubQueryFlag !== -1 && !githubQueryDir) {
  throw new Error("--github-query needs an output directory");
}
const prLinkFlag = process.argv.indexOf("--pr-link");
const prLinkDir = prLinkFlag === -1 ? null : process.argv[prLinkFlag + 1];
if (prLinkFlag !== -1 && !prLinkDir) {
  throw new Error("--pr-link needs an output directory");
}
const sidebarTreeFlag = process.argv.indexOf("--sidebar-tree");
const sidebarTreeDir = sidebarTreeFlag === -1 ? null : process.argv[sidebarTreeFlag + 1];
if (sidebarTreeFlag !== -1 && !sidebarTreeDir) {
  throw new Error("--sidebar-tree needs an output directory");
}
const viewDragFlag = process.argv.indexOf("--view-drag");
const viewDragDir = viewDragFlag === -1 ? null : process.argv[viewDragFlag + 1];
if (viewDragFlag !== -1 && !viewDragDir) {
  throw new Error("--view-drag needs an output directory");
}
const peopleFiltersFlag = process.argv.indexOf("--people-filters");
const peopleFiltersDir = peopleFiltersFlag === -1 ? null : process.argv[peopleFiltersFlag + 1];
if (peopleFiltersFlag !== -1 && !peopleFiltersDir) {
  throw new Error("--people-filters needs an output directory");
}
const filterBarFlag = process.argv.indexOf("--filter-bar");
const filterBarDir = filterBarFlag === -1 ? null : process.argv[filterBarFlag + 1];
if (filterBarFlag !== -1 && !filterBarDir) {
  throw new Error("--filter-bar needs an output directory");
}
const updateFlag = process.argv.indexOf("--update");
const updateDir = updateFlag === -1 ? null : process.argv[updateFlag + 1];
if (updateFlag !== -1 && !updateDir) {
  throw new Error("--update needs an output directory");
}
const mergeQueueFlag = process.argv.indexOf("--merge-queue");
const mergeQueueDir = mergeQueueFlag === -1 ? null : process.argv[mergeQueueFlag + 1];
if (mergeQueueFlag !== -1 && !mergeQueueDir) {
  throw new Error("--merge-queue needs an output directory");
}
const viewsFlag = process.argv.indexOf("--views");
const viewsDir = viewsFlag === -1 ? null : process.argv[viewsFlag + 1];
if (viewsFlag !== -1 && !viewsDir) {
  throw new Error("--views needs an output directory");
}
const closePrFlag = process.argv.indexOf("--close-pr");
const closePrDir = closePrFlag === -1 ? null : process.argv[closePrFlag + 1];
if (closePrFlag !== -1 && !closePrDir) {
  throw new Error("--close-pr needs an output directory");
}
const settingsStoryFlag = process.argv.indexOf("--settings-story");
const settingsStoryDir = settingsStoryFlag === -1 ? null : process.argv[settingsStoryFlag + 1];
if (settingsStoryFlag !== -1 && !settingsStoryDir) {
  throw new Error("--settings-story needs an output directory");
}
const settingsNavFlag = process.argv.indexOf("--settings-nav");
const settingsNavDir = settingsNavFlag === -1 ? null : process.argv[settingsNavFlag + 1];
if (settingsNavFlag !== -1 && !settingsNavDir) {
  throw new Error("--settings-nav needs an output directory");
}
const slackSettingsFlag = process.argv.indexOf("--slack-settings");
const slackSettingsDir = slackSettingsFlag === -1 ? null : process.argv[slackSettingsFlag + 1];
if (slackSettingsFlag !== -1 && !slackSettingsDir) {
  throw new Error("--slack-settings needs an output directory");
}
const agentsToolsFlag = process.argv.indexOf("--agents-tools");
const agentsToolsDir = agentsToolsFlag === -1 ? null : process.argv[agentsToolsFlag + 1];
if (agentsToolsFlag !== -1 && !agentsToolsDir) {
  throw new Error("--agents-tools needs an output directory");
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

/** The Branch card fixtures (src/screenshot/fixtures.ts), one PR per state. */
const BRANCH_STATES = [
  ["up-to-date", "up_to_date", /split the payment step/],
  ["behind", "behind", /Add address autocomplete to checkout/],
  ["update-required", "update_required", /Register passkeys from account settings/],
  ["conflicts", "conflicts", /Bump the minimum OS version/],
  ["checking", "checking", /Crash on empty wishlist/],
];

async function branchShots(page, dir) {
  const panel = page.getByTestId("detail-panel");
  const save = async (name) => {
    const path = resolve(dir, `branch-${name}.png`);
    await page.waitForTimeout(300);
    await page.screenshot({ path });
    console.log(`wrote ${path} (${Math.round(statSync(path).size / 1024)} KB)`);
  };
  for (const [name, state, title] of BRANCH_STATES) {
    await page
      .getByRole("button", { name: new RegExp(`^(?!Star |Unstar |Open ).*${title.source}`, title.flags) })
      .first()
      .click();
    await panel.locator(`[data-branch-state="${state}"]`).waitFor();
    await save(name);
    if (state === "update_required") {
      // The merge card with Update branch on the behind blocker, at default and narrow width.
      for (const [label, size] of [
        ["default", width],
        ["narrow", 760],
      ]) {
        await page.setViewportSize({ width: size, height: 900 });
        await page.waitForTimeout(400);
        const merge = panel.getByTestId("merge-verdict");
        await merge.screenshot({ path: resolve(dir, `branch-merge-card-${label}.png`) });
        console.log(`wrote branch-merge-card-${label}.png (${Math.round((await merge.boundingBox()).width)}px)`);
      }
      await page.setViewportSize({ width, height: 900 });
    }
  }
  await page
    .getByRole("button", { name: rowButton("Add address autocomplete to checkout") })
    .first()
    .click();
  const card = panel.locator('[data-branch-state="behind"]');
  await card.waitFor();
  await card.getByRole("button", { name: "Update branch options" }).click();
  await save("menu-open");
  await page.getByRole("menuitem", { name: /Rebase/ }).click();
  await page.getByRole("dialog").waitFor();
  await save("rebase-confirm");
}

async function slackDraftShots(page, dir) {
  mkdirSync(dir, { recursive: true });
  const dialog = page.getByRole("dialog");
  const save = async (name) => {
    await page.waitForTimeout(300);
    await page.screenshot({ path: resolve(dir, `slack-draft-${name}.png`) });
    console.log(`wrote slack-draft-${name}.png`);
  };
  const openSingle = async () => {
    await page.getByRole("button", { name: rowButton("Show saved cards first") }).first().click();
    await page.getByTestId("detail-panel").getByRole("button", { name: "Request review on Slack" }).click();
    await dialog.getByLabel("Message preview").waitFor();
  };
  await openSingle();
  await save("single-default");
  await page.setViewportSize({ width: 760, height: 900 });
  await save("single-narrow");
  await page.setViewportSize({ width, height: 900 });
  await dialog.getByRole("button", { name: "Mark ready for review" }).click();
  await dialog.getByRole("button", { name: "Send request" }).waitFor();
  await save("single-after-mark-ready");
  await page.keyboard.press("Escape");
  await page.keyboard.press("Escape");

  // Bulk: #488 is ready now, so turn it back into a draft through the same path.
  await page.evaluate(() => globalThis.__TAURI_INTERNALS__.invoke("set_pr_draft", { id: 4, draft: true }));
  for (const title of ["Show saved cards first", "Pin the base image for CI runners", "Add address autocomplete to checkout", "Register passkeys from account settings", "Offline cart sync"]) {
    await page.getByRole("checkbox", { name: `Select ${title}` }).first().check();
  }
  await page.getByRole("button", { name: "Request / bump on Slack" }).first().click();
  await dialog.getByLabel("Message preview").waitFor();
  await save("bulk-default");
  await page.setViewportSize({ width: 760, height: 900 });
  await save("bulk-narrow");
  await page.setViewportSize({ width, height: 900 });
  await dialog.getByRole("button", { name: "Mark all ready" }).click();
  await dialog.getByRole("button", { name: /^Send 1 message$/ }).waitFor();
  await save("bulk-after-mark-ready");
}

/** Bulk preview before and after Mark all ready, with one real CI block left. */
async function readyPreviewShots(page, dir) {
  mkdirSync(dir, { recursive: true });
  const dialog = page.getByRole("dialog");
  const save = async (name) => {
    await page.waitForTimeout(300);
    const box = await dialog.boundingBox();
    await page.screenshot({ path: resolve(dir, `ready-preview-${name}.png`), clip: box ?? undefined });
    console.log(`wrote ready-preview-${name}.png`);
  };
  for (const title of ["Add address autocomplete to checkout", "Show saved cards first", "Pin the base image for CI runners"]) {
    await page.getByRole("checkbox", { name: `Select ${title}` }).first().check();
  }
  await page.getByRole("button", { name: "Request / bump on Slack" }).first().click();
  await dialog.getByLabel("Message preview").waitFor();
  await dialog.getByText("Blocked: check ci is fail").first().waitFor();
  await save("before");
  await dialog.getByRole("button", { name: "Mark all ready" }).click();
  await dialog.getByText("Show saved cards first").waitFor();
  await dialog.getByText("Blocked: check ci is fail").waitFor();
  await save("after");
}

const UPDATES = {
  release: {
    kind: "release",
    current: "0.1.0",
    version: "0.2.0",
    url: "https://github.com/harnesstap/pr-nexus/releases/tag/v0.2.0",
    command: "gh release download v0.2.0 -R harnesstap/pr-nexus -p '*.dmg'",
    behind_by: null,
  },
  dev: {
    kind: "dev",
    current: "1a2b3c4",
    version: "9f8e7d6",
    url: "https://github.com/harnesstap/pr-nexus/compare/1a2b3c4...9f8e7d6",
    command: "git pull && bun install --cwd app && bun run tauri dev",
    behind_by: 12,
  },
};

/** One page per variant: the update answer is read once at startup. */
async function updateShots(browser, dir) {
  mkdirSync(dir, { recursive: true });
  const scenes = [
    ["none", null, false],
    ["available", UPDATES.release, false],
    ["popover-release", UPDATES.release, true],
    ["popover-dev", UPDATES.dev, true],
    ["aligned", UPDATES.release, null],
  ];
  for (const [name, update, open] of scenes) {
    const page = await browser.newPage({ viewport: { width, height: 900 }, deviceScaleFactor: 2, colorScheme: "dark" });
    await page.addInitScript((value) => {
      globalThis.__update = value;
    }, update);
    await page.goto(url);
    await page.getByRole("group", { name: "Quick filters" }).waitFor();
    if (update) {
      const icon = page.getByRole("button", { name: /update details$/ });
      await icon.waitFor();
      if (open) {
        await icon.click();
        await page.getByRole("dialog", { name: "Update available" }).waitFor();
      } else if (open === false) {
        await icon.hover();
        await page.waitForTimeout(800);
      }
    }
    await page.waitForTimeout(300);
    if (open === null) {
      // Footer close-up: report both trailing icons' centres so drift shows up as numbers.
      const centre = async (locator) => {
        const box = await locator.locator("svg").boundingBox();
        return { x: box.x + box.width / 2, y: box.y + box.height / 2, box };
      };
      const up = await centre(page.getByRole("button", { name: /update details$/ }));
      const sync = await centre(page.getByRole("button", { name: "Refresh" }));
      console.log(`update icon centre x=${up.x} box=${JSON.stringify(up.box)}`);
      console.log(`sync icon centre x=${sync.x} box=${JSON.stringify(sync.box)}`);
      const footer = await page.locator("aside footer").first().boundingBox();
      await page.screenshot({ path: resolve(dir, `update-${name}-sidebar.png`), clip: footer });
      await page.close();
      continue;
    }
    await page.screenshot({ path: resolve(dir, `update-${name}.png`) });
    const aside = await page.locator("aside").first().boundingBox();
    if (aside) {
      const height = 360;
      const clip = { x: aside.x, y: aside.y + aside.height - height, width: Math.min(aside.width + 140, width), height };
      await page.screenshot({ path: resolve(dir, `update-${name}-sidebar.png`), clip });
    }
    await page.close();
    console.log(`wrote update-${name}.png`);
  }
}

/** Repository picker open, both widgets selected, a narrow toolbar, and the sorted sidebar. */
async function repoFilterShots(page, dir) {
  mkdirSync(dir, { recursive: true });
  const save = async (name) => {
    await page.waitForTimeout(300);
    await page.screenshot({ path: resolve(dir, `repo-filter-${name}.png`) });
    console.log(`wrote repo-filter-${name}.png`);
  };
  const aside = await page.locator("aside").first().boundingBox();
  if (aside) {
    await page.screenshot({
      path: resolve(dir, "repo-filter-sidebar.png"),
      clip: { x: aside.x, y: aside.y, width: aside.width, height: Math.min(aside.height, 640) },
    });
    console.log("wrote repo-filter-sidebar.png");
  }
  await page.getByRole("button", { name: "Repository" }).click();
  const dialog = page.getByRole("dialog", { name: "Repositories" });
  await dialog.getByRole("textbox", { name: "Find a repository" }).fill("widgets");
  await dialog.getByRole("checkbox", { name: "acme/widgets" }).waitFor();
  await save("picker-open");
  await dialog.getByRole("checkbox", { name: "acme/widgets" }).check();
  await dialog.getByRole("checkbox", { name: "acme-labs/widgets" }).check();
  await page.keyboard.press("Escape");
  await page.getByRole("button", { name: "Repository, 2 repositories" }).waitFor();
  await save("selected");
  await page.setViewportSize({ width: 720, height: 900 });
  const toolbar = page.getByRole("toolbar", { name: "List toolbar" });
  const pageOverflow = await toolbar.evaluate((node) => {
    const root = node.ownerDocument.documentElement;
    return root.scrollWidth - root.clientWidth;
  });
  console.log(`page overflow ${pageOverflow}px at 720px`);
  if (pageOverflow > 0) {
    throw new Error(`page overflows by ${pageOverflow}px at 720px`);
  }
  await save("narrow");
}

/** A pasted pull request URL: one stored in another view, and one read for the panel. */
async function prLinkShots(browser, dir) {
  mkdirSync(dir, { recursive: true });
  const other = await browser.newPage({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: 2, colorScheme: "dark" });
  other.on("pageerror", (err) => console.error("page error:", err.message));
  await other.addInitScript(() => {
    globalThis.__prLinkScene = "other";
  });
  await other.goto(url);
  await other.getByRole("textbox", { name: "Search" }).waitFor();
  await other.getByRole("textbox", { name: "Search" }).fill("https://github.com/acme/infra/pull/61/files?w=1#diff");
  await other.getByRole("button", { name: rowButton("rotate the staging") }).waitFor();
  await other.getByRole("heading", { name: /rotate the staging/ }).waitFor();
  await other.screenshot({ path: resolve(dir, "pr-link-other-view.png") });
  console.log("wrote pr-link-other-view.png");
  await other.close();

  const fetched = await browser.newPage({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: 2, colorScheme: "dark" });
  fetched.on("pageerror", (err) => console.error("page error:", err.message));
  await fetched.addInitScript(() => {
    globalThis.__prLinkScene = "fetch";
  });
  await fetched.goto(url);
  await fetched.getByRole("textbox", { name: "Search" }).waitFor();
  await fetched.getByRole("textbox", { name: "Search" }).fill("https://github.com/acme/widgets/pull/332");
  await fetched.getByText("Opened from GitHub. Not in your lists.").first().waitFor();
  await fetched.getByRole("heading", { name: /Add a retry budget/ }).waitFor();
  await fetched.screenshot({ path: resolve(dir, "pr-link-fetched.png") });
  console.log("wrote pr-link-fetched.png");
  await fetched.close();
}

async function githubQueryShots(browser, dir) {
  mkdirSync(dir, { recursive: true });
  const shoot = async (scene, name, widthPx) => {
    const page = await browser.newPage({ viewport: { width: widthPx, height: 900 }, deviceScaleFactor: 2, colorScheme: "dark" });
    page.on("pageerror", (err) => console.error("page error:", err.message));
    await page.addInitScript((value) => {
      globalThis.__githubScene = value;
    }, scene);
    await page.goto(url);
    const query = scene === "error" ? "author:no-such-user repo:acme/missing" : "is:pr author:octocat sort:updated-desc";
    await page.waitForFunction((value) => {
      const input = globalThis.document.querySelector('[aria-label="Search"]');
      return input instanceof globalThis.HTMLInputElement && input.value === value;
    }, query);
    if (scene === "error") {
      await page.getByText("cannot be searched").waitFor();
    } else {
      await page.getByRole("button", { name: "#90 Ship the widget" }).waitFor();
      await page.getByText("Merged", { exact: true }).first().waitFor();
      // The count sits on the toolbar only once the row is wide enough to show it.
      if (widthPx > 720) {
        await page.getByText("2 PRs", { exact: true }).waitFor();
      }
    }
    await page.getByRole("textbox", { name: "Search" }).waitFor();
    if (widthPx <= 720) {
      const overflow = await page.evaluate(() => globalThis.document.documentElement.scrollWidth - globalThis.document.documentElement.clientWidth);
      console.log(`page overflow ${overflow}px at ${widthPx}px`);
      if (overflow > 0) {
        throw new Error(`page overflows by ${overflow}px at ${widthPx}px`);
      }
    }
    await page.screenshot({ path: resolve(dir, name) });
    console.log(`wrote ${name}`);
    await page.close();
  };
  await shoot("results", "github-query-results.png", 1440);
  await shoot("error", "github-query-error.png", 1440);
  await shoot("results", "github-query-narrow.png", 720);
}

/** Reviewer team and assignee filters, and the qualifiers they write into the field. */
async function peopleFilterShots(browser, dir) {
  mkdirSync(dir, { recursive: true });
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: 2, colorScheme: "dark" });
  page.on("pageerror", (err) => console.error("page error:", err.message));
  await page.goto(url);
  await page.getByRole("button", { name: /To review/ }).click();
  await page.getByRole("button", { name: "Filter menu" }).click();
  await page.getByRole("button", { name: /^Reviewer team/ }).click();
  await page.getByRole("checkbox", { name: /acme\/web-platform/ }).waitFor();
  await page.mouse.move(8, 8);
  await page.screenshot({ path: resolve(dir, "filters-team-menu.png") });
  await page.getByRole("button", { name: /^Assignee/ }).click();
  await page.getByRole("checkbox", { name: /^Me \(/ }).waitFor();
  await page.mouse.move(8, 8);
  await page.screenshot({ path: resolve(dir, "filters-assignee-menu.png") });
  await page.getByRole("button", { name: /^Reviewer team/ }).click();
  await page.getByRole("checkbox", { name: /acme\/web-platform/ }).click();
  await page.getByRole("button", { name: /^Assignee/ }).click();
  await page.getByRole("checkbox", { name: /^Me \(/ }).click();
  await page.keyboard.press("Escape");
  await page.getByRole("textbox", { name: "Search" }).waitFor();
  await page.waitForFunction(() => {
    const field = globalThis.document.querySelector('[aria-label="Search"]');
    return (
      field instanceof globalThis.HTMLInputElement &&
      field.value.includes("team-review-requested:acme/web-platform") &&
      field.value.includes("assignee:@me")
    );
  });
  await page.getByRole("article", { name: /Add address autocomplete/ }).waitFor();
  await page.screenshot({ path: resolve(dir, "filters-team-list.png") });
  console.log("wrote people filter shots");
  await page.close();
}

/** The one search field, with the filter menu open on Sort. */
async function filterBarShots(browser, dir) {
  mkdirSync(dir, { recursive: true });
  const shoot = async (name, widthPx, query, locked) => {
    const page = await browser.newPage({ viewport: { width: widthPx, height: 900 }, deviceScaleFactor: 2, colorScheme: "dark" });
    page.on("pageerror", (err) => console.error("page error:", err.message));
    await page.goto(url);
    const input = page.getByRole("textbox", { name: "Search" });
    await input.waitFor();
    await input.fill(query);
    await page.waitForFunction((value) => {
      const field = globalThis.document.querySelector('[aria-label="Search"]');
      if (!(field instanceof globalThis.HTMLInputElement) || field.value !== value) {
        return false;
      }
      const label = field.closest("label");
      return label !== null && !label.textContent.includes("Searching");
    }, query);
    await page.getByRole("button", { name: "Filter menu" }).click();
    await page.getByRole("button", { name: /^Sort/ }).click();
    if (locked) {
      await page.getByText("sort: in the search sets this order. Remove it to choose another.").waitFor();
    } else {
      await page.getByRole("button", { name: "✓ Needs attention" }).waitFor();
      const lockedCopy = await page.getByText(/sets this order/).count();
      if (lockedCopy !== 0) {
        throw new Error("Sort is locked without a sort: qualifier");
      }
    }
    if (widthPx <= 720) {
      const overflow = await page.evaluate(() => globalThis.document.documentElement.scrollWidth - globalThis.document.documentElement.clientWidth);
      console.log(`page overflow ${overflow}px at ${widthPx}px`);
      if (overflow > 0) {
        throw new Error(`page overflows by ${overflow}px at ${widthPx}px`);
      }
    }
    const menu = page.getByRole("menu");
    const box = await menu.boundingBox();
    const height = Math.min(900, Math.ceil((box?.y ?? 0) + (box?.height ?? 280) + 24));
    await page.screenshot({ path: resolve(dir, name), clip: { x: 0, y: 0, width: widthPx, height } });
    console.log(`wrote ${name}`);
    await page.close();
  };
  await shoot("filter-bar-normal.png", 1440, "author:me is:draft passkeys", false);
  await shoot("filter-bar-narrow.png", 720, "author:me is:draft passkeys", false);
  await shoot("filter-bar-sort.png", 1440, "author:me sort:created-asc", true);
}

async function reviewOpenShots(browser, dir) {
  mkdirSync(dir, { recursive: true });
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: 2, colorScheme: "dark" });
  page.on("pageerror", (err) => console.error("page error:", err.message));
  await page.addInitScript(() => {
    globalThis.__reviewTiming = true;
  });
  await page.goto(url);
  await page.getByRole("group", { name: "Quick filters" }).waitFor();
  await page.getByRole("button", { name: rowButton("lazy-load the card form") }).first().click();
  await page.getByTestId("detail-panel").getByRole("button", { name: "Request review on Slack" }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByText("New message in #web-reviews · Acme.").waitFor();
  await dialog.getByText("Checking channel…").waitFor();
  if ((await dialog.getByText("Finding channels…").count()) !== 0) {
    throw new Error("Finding channels is still showing");
  }
  await page.waitForFunction(() => typeof globalThis.__releaseReviewNames === "function");
  await page.screenshot({ path: resolve(dir, "review-dialog-open.png") });
  const marks = () =>
    page.evaluate(() =>
      performance
        .getEntriesByType("mark")
        .filter((mark) => mark.name.startsWith("review-dialog:"))
        .map((mark) => ({ name: mark.name, startTime: mark.startTime })),
    );
  const before = await marks();
  if (before.some((mark) => mark.name === "review-dialog:names")) {
    throw new Error("the live name landed before the first shot");
  }
  await page.evaluate(() => {
    globalThis.__releaseReviewNames();
  });
  await dialog.getByText("New message in #web-reviews-live · Acme.").waitFor();
  await page.waitForFunction(() => {
    const button = [...globalThis.document.querySelectorAll("button")].find((item) => item.textContent === "Send request");
    return button instanceof globalThis.HTMLButtonElement && !button.disabled;
  });
  if ((await dialog.getByText("Checking channel…").count()) !== 0) {
    throw new Error("Checking channel is still showing");
  }
  await page.screenshot({ path: resolve(dir, "review-dialog-live.png") });
  const after = await marks();
  console.log(JSON.stringify({ before, after }, null, 2));
  await page.close();
}

let browser;
try {
  browser = await chromium.launch(process.env.CHROME_PATH ? { executablePath: process.env.CHROME_PATH } : { channel: "chrome" });
  if (sidebarTreeDir) {
    await sidebarTreeShots(browser, resolve(sidebarTreeDir));
    await browser.close();
    await vite.close();
    process.exit(0);
  }
  if (viewDragDir) {
    await viewDragShots(browser, resolve(viewDragDir));
    await browser.close();
    await vite.close();
    process.exit(0);
  }
  if (viewsDir) {
    await viewShots(browser, resolve(viewsDir));
    await browser.close();
    await vite.close();
    process.exit(0);
  }
  if (settingsStoryDir) {
    await settingsStoryShots(browser, resolve(settingsStoryDir));
    await browser.close();
    await vite.close();
    process.exit(0);
  }
  if (settingsNavDir) {
    await settingsNavShots(browser, resolve(settingsNavDir));
    await browser.close();
    await vite.close();
    process.exit(0);
  }
  if (reviewOpenDir) {
    await reviewOpenShots(browser, resolve(reviewOpenDir));
    await browser.close();
    await vite.close();
    process.exit(0);
  }
  if (slackSettingsDir) {
    await slackSettingsShots(browser, resolve(slackSettingsDir));
    await browser.close();
    await vite.close();
    process.exit(0);
  }
  if (agentsToolsDir) {
    await agentToolShots(browser, resolve(agentsToolsDir));
    await browser.close();
    await vite.close();
    process.exit(0);
  }
  if (mergeQueueDir) {
    await mergeQueueShots(browser, resolve(mergeQueueDir));
    await browser.close();
    await vite.close();
    process.exit(0);
  }
  if (githubQueryDir) {
    await githubQueryShots(browser, resolve(githubQueryDir));
    await browser.close();
    await vite.close();
    process.exit(0);
  }
  if (prLinkDir) {
    await prLinkShots(browser, resolve(prLinkDir));
    await browser.close();
    await vite.close();
    process.exit(0);
  }
  if (peopleFiltersDir) {
    await peopleFilterShots(browser, resolve(peopleFiltersDir));
    await browser.close();
    await vite.close();
    process.exit(0);
  }
  if (filterBarDir) {
    await filterBarShots(browser, resolve(filterBarDir));
    await browser.close();
    await vite.close();
    process.exit(0);
  }
  if (updateDir) {
    await updateShots(browser, resolve(updateDir));
    await browser.close();
    await vite.close();
    process.exit(0);
  }
  const page = await browser.newPage({ viewport: { width, height: 900 }, deviceScaleFactor: 2, colorScheme: "dark" });
  page.on("pageerror", (err) => console.error("page error:", err.message));
  if (slackDraftDir || readyPreviewDir) {
    // #58 (row 12) is a draft too, for the bulk dialog.
    await page.addInitScript((ready) => {
      globalThis.__extraDrafts = [12];
      if (ready) {
        globalThis.__readyPreview = true;
      }
    }, Boolean(readyPreviewDir));
  }
  if (repoFilterDir) {
    await page.addInitScript(() => {
      globalThis.__repoScene = true;
    });
  }
  if (agreeOut) {
    await page.addInitScript(() => {
      globalThis.__agreeDetail = true;
    });
  }
  await page.goto(url);
  await page.getByRole("group", { name: "Quick filters" }).waitFor();
  if (repoFilterDir) {
    await repoFilterShots(page, resolve(repoFilterDir));
  } else if (readyPreviewDir) {
    await readyPreviewShots(page, resolve(readyPreviewDir));
  } else if (slackDraftDir) {
    await slackDraftShots(page, resolve(slackDraftDir));
  } else if (tooltipsDir) {
    await shootTooltips(page, tooltipsDir);
  } else if (scenesDir) {
    await scenes(page, resolve(scenesDir));
  } else if (branchDir) {
    await branchShots(page, resolve(branchDir));
  } else if (closePrDir) {
    await closePrShots(page, resolve(closePrDir));
  } else if (agreeOut) {
    await page
      .getByRole("button", { name: rowButton("Add address autocomplete to checkout") })
      .first()
      .click();
    const panel = page.getByTestId("detail-panel");
    await panel.getByText("You can merge").waitFor();
    await panel.getByLabel("Approvals: 1/1, Approved").waitFor();
    await page.waitForFunction(() => {
      const button = [...globalThis.document.querySelectorAll("button")].find(
        (el) => el.textContent?.includes("Add address autocomplete to checkout") && !el.getAttribute("aria-label")?.startsWith("Star"),
      );
      return Boolean(button) && !button.textContent.includes("Waiting");
    });
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
  if (!tooltipsDir && !scenesDir && !branchDir && !slackDraftDir && !repoFilterDir && !readyPreviewDir && !closePrDir) {
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
    [
      "pr-title",
      () =>
        page
          .locator("article")
          .filter({ hasText: "split the payment step" })
          .locator("button.truncate, button.line-clamp-2")
          .first(),
    ],
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

async function closePrShots(page, dir) {
  mkdirSync(dir, { recursive: true });
  const save = async (name) => {
    const path = resolve(dir, name);
    await page.waitForTimeout(300);
    await page.screenshot({ path });
    console.log(`wrote ${path} (${Math.round(statSync(path).size / 1024)} KB)`);
  };
  const panel = page.getByTestId("detail-panel");
  await page.getByRole("button", { name: rowButton("Add address autocomplete to checkout") }).first().click();
  await panel.getByRole("button", { name: /^Checks:/ }).waitFor();
  await panel.getByRole("button", { name: "More actions" }).click();
  await panel.getByRole("menuitem", { name: "Close PR" }).waitFor();
  await save("close-detail-menu.png");
  await panel.getByRole("menuitem", { name: "Close PR" }).click();
  const dialog = page.getByRole("dialog", { name: "Close this pull request?" });
  await dialog.getByRole("textbox", { name: "Comment (optional)" }).fill("No longer needed. The branch stays.");
  await save("close-dialog.png");
  await dialog.getByRole("button", { name: "Cancel" }).click();
  await panel.getByRole("button", { name: "Close details" }).click();

  await page.getByRole("checkbox", { name: "Select Add address autocomplete to checkout" }).click();
  await page
    .getByRole("checkbox", { name: "Select feat(checkout): split the payment step into its own route and lazy-load the card form" })
    .click();
  const bar = page.getByRole("region", { name: "Selection actions" });
  await bar.getByRole("button", { name: "More actions" }).click();
  await bar.getByRole("menuitem", { name: "Close 2 PRs" }).click();
  await page.getByRole("dialog", { name: "Close 2 pull requests?" }).waitFor();
  await save("close-bulk.png");
  await page.keyboard.press("Escape");

  await page.setViewportSize({ width: 860, height: 900 });
  await page.waitForTimeout(200);
  if ((await bar.getAttribute("data-layout")) === "full") {
    await page.setViewportSize({ width: 720, height: 900 });
    await page.waitForTimeout(200);
  }
  await bar.getByRole("button", { name: "More actions" }).click();
  await bar.getByRole("menuitem", { name: "Add to project" }).waitFor();
  await bar.getByRole("menuitem", { name: "Close 2 PRs" }).waitFor();
  await save("close-narrow-menu.png");
}

async function mergeQueueShots(browser, dir) {
  mkdirSync(dir, { recursive: true });
  const preview = `http://127.0.0.1:${port}/preview.html?screen=merge-queue`;
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 }, deviceScaleFactor: 2, colorScheme: "dark" });
  page.on("pageerror", (err) => console.error("page error:", err.message));
  await page.goto(preview);
  const latch = page.getByRole("button", { name: rowButton("Tighten the latch") }).first();
  await latch.waitFor();
  await shot(page, resolve(dir, "merge-queue-list.png"));

  await latch.click();
  const panel = page.getByTestId("detail-panel");
  await panel.getByRole("button", { name: "Add to merge queue" }).click();
  await panel.getByRole("group", { name: "Confirm add to merge queue" }).waitFor();
  await shot(page, resolve(dir, "merge-queue-add.png"));
  await panel.getByRole("button", { name: "Cancel" }).click();
  await panel.getByRole("button", { name: "Close details" }).click();

  await page.getByRole("checkbox", { name: "Select Tighten the latch" }).click();
  await page.getByRole("checkbox", { name: "Select Draft the release notes" }).click();
  const bar = page.getByRole("region", { name: "Selection actions" });
  await bar.getByRole("button", { name: "More actions" }).click();
  await bar.getByRole("menuitem", { name: "Add to merge queue" }).click();
  await page.getByRole("dialog", { name: "Add to the merge queue?" }).waitFor();
  await shot(page, resolve(dir, "merge-queue-bulk.png"));
  await page.keyboard.press("Escape");
  await page.getByRole("dialog", { name: "Add to the merge queue?" }).waitFor({ state: "detached" });

  await page.setViewportSize({ width: 480, height: 900 });
  await page.waitForTimeout(300);
  await shot(page, resolve(dir, "merge-queue-narrow.png"));
  await page.close();
}

/** Settings → MCP server → Permissions: collapsed, one category edited, one category off. */
async function agentToolShots(browser, dir) {
  mkdirSync(dir, { recursive: true });
  const page = await browser.newPage({ viewport: { width: 1100, height: 1400 }, deviceScaleFactor: 2, colorScheme: "dark" });
  page.on("pageerror", (err) => console.error("page error:", err.message));
  await page.goto(url);
  await page.getByRole("button", { name: /account menu/ }).click();
  await page.getByRole("menuitem", { name: "Settings" }).click();
  await page.getByRole("tab", { name: "MCP server" }).click();
  const section = page.getByRole("region", { name: "Permissions" });
  await section.getByRole("button", { name: "Reads: 6 of 6 tools" }).waitFor();
  await section.screenshot({ path: resolve(dir, "agents-tools-collapsed.png") });
  console.log("wrote agents-tools-collapsed.png");

  await section.getByRole("button", { name: "Reads: 6 of 6 tools" }).click();
  await section.getByRole("switch", { name: "list_projects", exact: true }).click();
  await section.getByRole("switch", { name: "get_review_stats", exact: true }).click();
  await section.getByRole("button", { name: "Reads: 4 of 6 tools" }).waitFor();
  await section.screenshot({ path: resolve(dir, "agents-tools-reads-expanded.png") });
  console.log("wrote agents-tools-reads-expanded.png");

  await section.getByRole("button", { name: "Local writes: 11 of 11 tools" }).click();
  await section.getByRole("switch", { name: "create_project", exact: true }).click();
  await section.getByRole("switch", { name: "set_pr_note", exact: true }).click();
  await section.getByRole("button", { name: "Local writes: 9 of 11 tools" }).waitFor();
  await section.getByRole("switch", { name: "Local writes", exact: true }).click();
  const greyed = section.getByRole("switch", { name: "create_project", exact: true });
  await greyed.waitFor();
  if (await greyed.isEnabled()) {
    throw new Error("create_project stayed enabled after Local writes was turned off");
  }
  if ((await greyed.getAttribute("aria-checked")) !== "false") {
    throw new Error("create_project lost its off state when the category turned off");
  }
  const kept = section.getByRole("switch", { name: "archive_project", exact: true });
  if ((await kept.getAttribute("aria-checked")) !== "true") {
    throw new Error("archive_project did not stay on while the category is off");
  }
  await section.screenshot({ path: resolve(dir, "agents-tools-writes-off.png") });
  console.log("wrote agents-tools-writes-off.png");
  await page.close();

  const scopes = await browser.newPage({ viewport: { width: 1100, height: 900 }, deviceScaleFactor: 2, colorScheme: "dark" });
  scopes.on("pageerror", (err) => console.error("page error:", err.message));
  await scopes.addInitScript(() => {
    globalThis.__agentScopes = true;
  });
  await scopes.goto(url);
  await scopes.getByRole("button", { name: /account menu/ }).click();
  await scopes.getByRole("menuitem", { name: "Settings" }).click();
  await scopes.getByRole("tab", { name: "MCP server" }).click();
  await scopes.getByRole("switch", { name: "Slack reads" }).click();
  const picker = scopes.getByRole("group", { name: "Workspaces agents may read" });
  const again = picker.getByRole("button", { name: "Sign in again to Acme", exact: true });
  await again.waitFor();
  await picker.getByRole("button", { name: "Sign in again to Acme Labs", exact: true }).waitFor();
  await again.click();
  await picker.getByText(/Slack did not give these permissions/).waitFor();
  await picker.screenshot({ path: resolve(dir, "agents-workspace-signin.png") });
  console.log("wrote agents-workspace-signin.png");
  await scopes.close();
}

/** Merged settings pages, the setup notice, and an add that stays pending on one AI agent. */
async function settingsStoryShots(browser, dir) {
  mkdirSync(dir, { recursive: true });
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 }, deviceScaleFactor: 2, colorScheme: "dark" });
  page.on("pageerror", (err) => console.error("page error:", err.message));
  await page.addInitScript(() => {
    localStorage.removeItem("pr-nexus.settings-section");
    localStorage.removeItem("pr-nexus.setup-notices-dismissed");
  });
  await page.goto(url);
  const banner = page.getByRole("alert").filter({ hasText: "Cursor has an outdated PR Nexus config." });
  await banner.waitFor();
  await page.screenshot({ path: resolve(dir, "settings_notice_banner.png") });
  console.log("wrote settings_notice_banner.png");

  await page.getByRole("button", { name: /account menu/ }).click();
  await page.getByRole("menuitem", { name: "Settings" }).waitFor();
  await page.screenshot({ path: resolve(dir, "settings_notice_badge.png") });
  console.log("wrote settings_notice_badge.png");
  await page.getByRole("menuitem", { name: "Settings" }).click();
  await page.getByRole("heading", { name: "Settings", level: 1 }).waitFor();

  const nav = page.getByRole("navigation", { name: "Settings" });
  await page.getByRole("tab", { name: "Slack" }).click();
  await page.getByRole("heading", { name: "Slack" }).waitFor();
  await nav.screenshot({ path: resolve(dir, "settings_sidebar_slack.png") });
  await page.getByRole("tab", { name: "Profiles" }).hover();
  await page.waitForTimeout(200);
  await nav.screenshot({ path: resolve(dir, "settings_sidebar_hover.png") });
  console.log("wrote settings sidebar");

  await page.setViewportSize({ width: 1280, height: 2200 });
  async function pageShot(tab, heading, file) {
    await page.getByRole("tab", { name: tab }).click();
    await page.getByRole("tabpanel").getByRole("heading", { name: heading }).waitFor();
    await page.waitForTimeout(250);
    await page.screenshot({ path: resolve(dir, file) });
    console.log(`wrote ${file}`);
  }

  await pageShot("Review", "Requests & bumps", "settings_page_review.png");
  await pageShot("Sync", "Schedule", "settings_page_sync.png");
  await pageShot("MCP server", "Use PR Nexus from an AI agent (MCP)", "settings_page_mcp.png");
  await pageShot("Agent activity", "Activity", "settings_page_activity.png");
  await pageShot("About", "PR Nexus 0.1.0", "settings_page_about.png");
  await page.close();

  const pending = await browser.newPage({ viewport: { width: 1100, height: 820 }, deviceScaleFactor: 2, colorScheme: "dark" });
  pending.on("pageerror", (err) => console.error("page error:", err.message));
  await pending.addInitScript(() => {
    localStorage.setItem("pr-nexus.mcp-client-tab", "claude-code");
    globalThis.__mcpAddHang = "claude-code";
  });
  await pending.goto(url);
  await pending.getByRole("button", { name: /account menu/ }).click();
  await pending.getByRole("menuitem", { name: "Settings" }).click();
  await pending.getByRole("tab", { name: "MCP server" }).click();
  const add = pending.getByRole("button", { name: "Add to Claude Code" });
  await add.waitFor();
  await add.click();
  await pending.getByRole("button", { name: "Adding…" }).waitFor();
  await pending.getByRole("tab", { name: "Cursor" }).click();
  const update = pending.getByRole("button", { name: "Update Cursor" });
  await update.waitFor();
  if (await update.isDisabled()) {
    throw new Error("Cursor stayed disabled while Claude Code was adding");
  }
  const tabs = pending.getByRole("tablist", { name: "AI agent", exact: true });
  const tabsBox = await tabs.boundingBox();
  const updateBox = await update.boundingBox();
  if (!tabsBox || !updateBox) {
    throw new Error("could not measure the AI agent tabs");
  }
  await pending.screenshot({
    path: resolve(dir, "settings_mcp_add_pending.png"),
    clip: {
      x: Math.max(0, tabsBox.x - 8),
      y: Math.max(0, tabsBox.y - 28),
      width: Math.max(tabsBox.width, updateBox.x + updateBox.width - tabsBox.x) + 24,
      height: updateBox.y + updateBox.height - tabsBox.y + 44,
    },
  });
  console.log("wrote settings_mcp_add_pending.png");
  await pending.close();
}

/** Settings rail: Slack, Review, MCP server with one category open, and the narrow section menu. */
async function settingsNavShots(browser, dir) {
  mkdirSync(dir, { recursive: true });
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 }, deviceScaleFactor: 2, colorScheme: "dark" });
  page.on("pageerror", (err) => console.error("page error:", err.message));
  await page.addInitScript(() => {
    localStorage.removeItem("pr-nexus.settings-section");
  });
  await page.goto(url);
  await page.getByRole("button", { name: /account menu/ }).click();
  await page.getByRole("menuitem", { name: "Settings" }).click();
  await page.getByRole("heading", { name: "Settings", level: 1 }).waitFor();

  async function section(tab, heading, file) {
    await page.getByRole("tab", { name: tab }).click();
    await page.getByRole("tabpanel").getByRole("heading", { name: heading }).waitFor();
    await page.waitForTimeout(300);
    await page.screenshot({ path: resolve(dir, file) });
    console.log(`wrote ${file}`);
  }

  await section("Slack", "Slack", "settings-nav-slack.png");
  await section("Review", "Requests & bumps", "settings-nav-requests.png");
  await page.getByRole("tab", { name: "MCP server" }).click();
  await page.getByRole("tabpanel").getByRole("heading", { name: "Permissions" }).waitFor();
  await page.getByRole("button", { name: "Reads: 6 of 6 tools" }).click();
  await page.getByRole("switch", { name: "list_projects", exact: true }).waitFor();
  await page.setViewportSize({ width: 1280, height: 1100 });
  await page.waitForTimeout(300);
  await page.screenshot({ path: resolve(dir, "settings-nav-permissions.png") });
  console.log("wrote settings-nav-permissions.png");
  await page.setViewportSize({ width: 940, height: 900 });
  await page.getByRole("combobox", { name: "Settings section" }).waitFor();
  await page.waitForTimeout(300);
  await page.screenshot({ path: resolve(dir, "settings-nav-narrow.png") });
  console.log("wrote settings-nav-narrow.png");
  await page.close();
}

/** Settings → Slack → Connected workspaces: connected, expired, and a refresh in progress. */
async function slackSettingsShots(browser, dir) {
  mkdirSync(dir, { recursive: true });
  for (const scene of ["connected", "expired", "refreshing"]) {
    const page = await browser.newPage({ viewport: { width: 1100, height: 900 }, deviceScaleFactor: 2, colorScheme: "dark" });
    page.on("pageerror", (err) => console.error("page error:", err.message));
    await page.addInitScript((name) => {
      globalThis.__slackScene = name;
    }, scene);
    await page.goto(url);
    await page.getByRole("button", { name: /account menu/ }).click();
    await page.getByRole("menuitem", { name: "Settings" }).click();
    await page.getByRole("tab", { name: "Slack" }).click();
    await page.getByRole("heading", { name: "Slack" }).waitFor();
    const block = page.getByTestId("slack-workspaces");
    await block.waitFor();
    if (scene === "refreshing") {
      await block.getByRole("button", { name: "Refresh" }).waitFor();
      await block.locator("[aria-busy='true']").waitFor();
    }
    if (scene === "expired") {
      await block.getByRole("button", { name: "Reconnect Acme Labs" }).waitFor();
      await block.getByText("Token expired").waitFor();
    }
    if (scene === "connected") {
      await block.getByText("Connected").first().waitFor();
      await expectBusy(block, false);
    }
    await block.scrollIntoViewIfNeeded();
    const name = `slack-workspaces-${scene}.png`;
    await block.screenshot({ path: resolve(dir, name) });
    console.log(`wrote ${name}`);
    await page.close();
  }
}

async function expectBusy(block, busy) {
  const button = block.getByRole("button", { name: "Refresh" });
  if (busy) {
    await button.waitFor();
  }
  const value = await button.getAttribute("aria-busy");
  if ((value === "true") !== busy) {
    throw new Error(`Refresh aria-busy is ${value}, expected ${busy}`);
  }
}

async function sidebarTreeShots(browser, dir) {
  mkdirSync(dir, { recursive: true });
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: 2, colorScheme: "dark" });
  page.on("pageerror", (err) => console.error("page error:", err.message));
  await page.addInitScript(() => {
    globalThis.__sidebarTree = true;
  });
  await page.goto(url);
  await page.getByRole("button", { name: "Drafts", exact: true }).waitFor();
  await page.mouse.move(900, 40);
  const clip = { x: 0, y: 0, width: 320, height: 640 };
  await page.screenshot({ path: resolve(dir, "sidebar-tree.png"), clip });

  await page.getByRole("button", { name: "To review", exact: true }).click();
  const search = page.getByRole("textbox", { name: "Search" });
  await search.fill("team-review-requested:acme/web-platform");
  await page.getByRole("button", { name: "New view" }).click();
  await page.getByRole("combobox", { name: "Parent" }).waitFor();
  await page.getByText("Shows is:open review-requested:@me team-review-requested:acme/web-platform").waitFor();
  const dialog = page.getByRole("dialog", { name: "New view" });
  const box = await dialog.boundingBox();
  await page.screenshot({
    path: resolve(dir, "sidebar-new-view.png"),
    clip: { x: Math.max(0, (box?.x ?? 0) - 24), y: Math.max(0, (box?.y ?? 0) - 24), width: Math.ceil((box?.width ?? 480) + 48), height: Math.ceil((box?.height ?? 360) + 48) },
  });
  await page.keyboard.press("Escape");

  const platform = page.getByRole("button", { name: "Web platform", exact: true });
  await platform.hover();
  await platform.locator("xpath=..").getByRole("button", { name: "View menu" }).click();
  await page.getByRole("menuitem", { name: "Edit filters" }).click();
  await page.getByRole("dialog", { name: "Edit filters" }).waitFor();
  await page.getByText("Shows is:open review-requested:@me team-review-requested:acme/web-platform").waitFor();
  const edit = page.getByRole("dialog", { name: "Edit filters" });
  const editBox = await edit.boundingBox();
  await page.screenshot({
    path: resolve(dir, "sidebar-edit-parent.png"),
    clip: {
      x: Math.max(0, (editBox?.x ?? 0) - 24),
      y: Math.max(0, (editBox?.y ?? 0) - 24),
      width: Math.ceil((editBox?.width ?? 480) + 48),
      height: Math.ceil((editBox?.height ?? 360) + 48),
    },
  });
  await page.keyboard.press("Escape");
  await page.mouse.move(900, 40);

  await page.getByRole("button", { name: "Views", exact: true }).click();
  await page.getByRole("button", { name: "Views", exact: true }).waitFor();
  await page.screenshot({ path: resolve(dir, "sidebar-views-collapsed.png"), clip });
  await page.getByRole("button", { name: "Views", exact: true }).click();
  await page.getByRole("button", { name: "Drafts", exact: true }).waitFor();
  await page.getByRole("button", { name: "Projects", exact: true }).click();
  await page.getByRole("button", { name: "Projects", exact: true }).waitFor();
  await page.screenshot({ path: resolve(dir, "sidebar-projects-collapsed.png"), clip: { x: 0, y: 0, width: 320, height: 900 } });
  console.log("wrote sidebar tree shots");
  await page.close();
}

async function viewDragShots(browser, dir) {
  mkdirSync(dir, { recursive: true });
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: 2, colorScheme: "dark" });
  page.on("pageerror", (err) => console.error("page error:", err.message));
  await page.addInitScript(() => {
    globalThis.__sidebarTree = true;
  });
  await page.goto(url);
  await page.getByRole("button", { name: "Drafts", exact: true }).waitFor();
  await page.mouse.move(900, 40);

  const source = page.locator('[data-view-key="custom:1"]');
  const target = page.locator('[data-view-key="to_review"]');
  const sourceBox = await source.boundingBox();
  const targetBox = await target.boundingBox();
  if (!sourceBox || !targetBox) {
    throw new Error("missing drag rows");
  }
  const from = { x: sourceBox.x + 28, y: sourceBox.y + sourceBox.height / 2 };
  const to = { x: targetBox.x + targetBox.width / 2, y: targetBox.y + targetBox.height / 2 };
  await page.mouse.move(from.x, from.y);
  await page.mouse.down();
  await page.mouse.move(to.x, to.y, { steps: 16 });
  await page.locator('[data-view-key="to_review"][data-drop-target="active"]').waitFor();
  await page.getByTestId("view-drag-ghost").waitFor();
  const side = { x: 0, y: 0, width: 360, height: 560 };
  await page.screenshot({ path: resolve(dir, "view-drag-over.png"), clip: side });
  console.log("wrote view-drag-over.png");
  await page.mouse.up();
  const dialog = page.getByRole("dialog", { name: "Move view" });
  await dialog.waitFor();
  await dialog.getByText("review-requested:@me").waitFor();
  const box = await dialog.boundingBox();
  await page.screenshot({
    path: resolve(dir, "view-drag-confirm.png"),
    clip: {
      x: 0,
      y: 0,
      width: Math.max(420, Math.ceil((box?.x ?? 0) + (box?.width ?? 280) + 24)),
      height: Math.max(560, Math.ceil((box?.y ?? 0) + (box?.height ?? 120) + 24)),
    },
  });
  console.log("wrote view-drag-confirm.png");
  await dialog.getByRole("button", { name: "Move" }).click();
  await dialog.waitFor({ state: "detached" });
  const drafts = page.getByRole("button", { name: "Drafts", exact: true });
  await drafts.waitFor();
  await page.mouse.move(900, 40);
  await page.screenshot({ path: resolve(dir, "view-drag-tree.png"), clip: side });
  console.log("wrote view-drag-tree.png");

  await page.getByRole("button", { name: "Payments", exact: true }).hover();
  await page.locator('[data-view-key="custom:3"]').getByRole("button", { name: "View menu" }).click();
  await page.getByRole("menuitem", { name: "Move to" }).hover();
  await page.getByRole("menu", { name: "Move to" }).waitFor();
  const menu = page.getByRole("menu", { name: "Actions for Payments" });
  const menuBox = await menu.boundingBox();
  const subBox = await page.getByRole("menu", { name: "Move to" }).boundingBox();
  const top = Math.max(0, Math.min(menuBox?.y ?? 0, subBox?.y ?? 0) - 56);
  const right = Math.max((menuBox?.x ?? 0) + (menuBox?.width ?? 0), (subBox?.x ?? 0) + (subBox?.width ?? 0)) + 24;
  const bottom = Math.max((menuBox?.y ?? 0) + (menuBox?.height ?? 0), (subBox?.y ?? 0) + (subBox?.height ?? 0)) + 24;
  const left = 0;
  await page.screenshot({
    path: resolve(dir, "view-drag-menu.png"),
    clip: { x: left, y: top, width: Math.ceil(right - left), height: Math.ceil(bottom - top) },
  });
  console.log("wrote view-drag-menu.png");
  await page.close();
}

async function viewShots(browser, dir) {
  mkdirSync(dir, { recursive: true });
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: 2, colorScheme: "dark" });
  page.on("pageerror", (err) => console.error("page error:", err.message));
  await page.goto(url);
  await page.getByRole("navigation", { name: "Views" }).waitFor();

  async function create(name, query) {
    await page.getByRole("button", { name: "New view" }).click();
    await page.getByRole("textbox", { name: "Name" }).fill(name);
    await page.getByRole("textbox", { name: "Query" }).fill(query);
    await page.getByRole("button", { name: "Create view" }).click();
    await page.getByRole("dialog").waitFor({ state: "detached" });
    await page.getByRole("button", { name, exact: true }).waitFor();
  }

  await create("Needs me", "is:open review-requested:@me");
  await create("Drafts", "is:open is:draft");
  await page.getByRole("button", { name: "My PRs", exact: true }).click();
  await page.mouse.move(900, 80);
  const clip = { x: 0, y: 0, width: 280, height: 520 };
  await page.screenshot({ path: resolve(dir, "views-sidebar.png"), clip });
  console.log("wrote views-sidebar.png");

  await page.getByRole("button", { name: "Needs me", exact: true }).hover();
  await page.screenshot({ path: resolve(dir, "views-sidebar-hover.png"), clip });
  console.log("wrote views-sidebar-hover.png");

  const drafts = page.getByRole("button", { name: "Drafts", exact: true });
  await drafts.locator("xpath=..").getByRole("button", { name: "View menu" }).click();
  await page.getByRole("menu").waitFor();
  await page.screenshot({ path: resolve(dir, "views-menu.png"), clip: { x: 0, y: 0, width: 480, height: 560 } });
  console.log("wrote views-menu.png");
  await page.close();
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

  await page.getByRole("button", { name: /account menu/ }).click();
  await page.getByRole("menuitem", { name: "Settings" }).click();
  await page.getByRole("tab", { name: "Rules" }).click();
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
