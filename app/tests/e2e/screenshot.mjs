// Quick visual check: node tests/e2e/screenshot.mjs <url> <out.png> (preview server must run)
import { chromium } from "@playwright/test";
const [url, out] = process.argv.slice(2);
const browser = await chromium.launch({ executablePath: process.env.PLAYWRIGHT_CHROMIUM_PATH || undefined });
const page = await browser.newPage({ viewport: { width: 1400, height: 900 } });
const errors = [];
page.on("pageerror", (e) => errors.push(String(e)));
page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
await page.goto(url);
await page.waitForTimeout(800);
await page.screenshot({ path: out });
console.log(errors.length ? errors.join("\n") : "no errors");
await browser.close();
