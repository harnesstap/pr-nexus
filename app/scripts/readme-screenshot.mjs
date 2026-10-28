// Renders the real app over mocked Tauri IPC (screenshot.html) and saves the
// README screenshot. Usage, from app/: bun run screenshot
// `bun run screenshot -- --detail <path>` opens #479's detail panel and saves
// that view to <path> instead; `--draft <path>` opens draft #488, which has
// no Slack request yet.
// `--width <px>` sets the viewport width (default 1440); `--out <path>` saves the
// list view somewhere other than the README asset.
// Uses an installed Chrome; set CHROME_PATH to pick another Chromium binary.
import { statSync } from "node:fs";
import { dirname, resolve } from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright-core";
import { createServer } from "vite";

const appDir = resolve(dirname(fileURLToPath(import.meta.url)), "..");
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
const port = 1430;
const url = `http://127.0.0.1:${port}/screenshot.html`;

const vite = await createServer({ root: appDir, server: { port, strictPort: true, host: "127.0.0.1" }, logLevel: "warn" });
await vite.listen();

let browser;
try {
  browser = await chromium.launch(process.env.CHROME_PATH ? { executablePath: process.env.CHROME_PATH } : { channel: "chrome" });
  const page = await browser.newPage({ viewport: { width, height: 900 }, deviceScaleFactor: 2, colorScheme: "dark" });
  page.on("pageerror", (err) => console.error("page error:", err.message));
  await page.goto(url);
  await page.getByRole("group", { name: "Quick filters" }).waitFor();
  if (detailOut) {
    await page.getByRole("button", { name: draftOut ? /Show saved cards first/ : /Add address autocomplete to checkout/ }).first().click();
    await page.getByTestId("detail-panel").getByRole("button", { name: /^Checks:/ }).waitFor();
    if (draftOut) {
      await page.getByTestId("detail-panel").getByRole("button", { name: "Request review on Slack" }).waitFor();
    } else {
      await page.getByTestId("detail-panel").getByRole("button", { name: /^Checks:/ }).click();
    }
  }
  await page.waitForTimeout(500);
  await page.screenshot({ path: out });
  console.log(`wrote ${out} (${Math.round(statSync(out).size / 1024)} KB)`);
} finally {
  await browser?.close();
  await vite.close();
}
