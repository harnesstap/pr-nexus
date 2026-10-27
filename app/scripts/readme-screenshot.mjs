// Renders the real app over mocked Tauri IPC (screenshot.html) and saves the
// README screenshot. Usage, from app/: bun run screenshot
// Uses an installed Chrome; set CHROME_PATH to pick another Chromium binary.
import { statSync } from "node:fs";
import { dirname, resolve } from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright-core";
import { createServer } from "vite";

const appDir = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const out = resolve(appDir, "../docs/assets/screenshot-list.png");
const port = 1430;
const url = `http://127.0.0.1:${port}/screenshot.html`;

const vite = await createServer({ root: appDir, server: { port, strictPort: true, host: "127.0.0.1" }, logLevel: "warn" });
await vite.listen();

let browser;
try {
  browser = await chromium.launch(process.env.CHROME_PATH ? { executablePath: process.env.CHROME_PATH } : { channel: "chrome" });
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: 2, colorScheme: "dark" });
  page.on("pageerror", (err) => console.error("page error:", err.message));
  await page.goto(url);
  await page.getByRole("group", { name: "Quick filters" }).waitFor();
  await page.waitForTimeout(500);
  await page.screenshot({ path: out });
  console.log(`wrote ${out} (${Math.round(statSync(out).size / 1024)} KB)`);
} finally {
  await browser?.close();
  await vite.close();
}
