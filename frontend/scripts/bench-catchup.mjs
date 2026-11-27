import { chromium } from "@playwright/test";

const BASE = process.env.BENCH_BASE_URL ?? "http://127.0.0.1:4173";
const RUNS = Number(process.argv[2] ?? 5);
const LABEL = process.argv[3] ?? "run";

const browser = await chromium.launch({ headless: true });
const results = [];
for (let run = 0; run < RUNS; run++) {
  const context = await browser.newContext({
    locale: "ja-JP",
    viewport: { width: 1440, height: 900 },
  });
  const page = await context.newPage();
  page.on("pageerror", (e) => console.error("PAGEERROR", e.message.slice(0, 200)));
  await page.goto(BASE, { waitUntil: "domcontentloaded" });
  await page.waitForSelector("[data-app-shell]", { timeout: 30000 });
  const link = page.locator('a[href="/catch-up"]');
  await link.waitFor({ state: "visible", timeout: 30000 });
  const t0 = await page.evaluate(() => performance.now());
  await link.click();
  await page.waitForFunction(
    () => {
      const b = Array.from(document.querySelectorAll("button")).find(
        (x) => (x.textContent ?? "").trim() === "既読にする",
      );
      return b && !b.disabled;
    },
    { timeout: 30000 },
  );
  const tReady = await page.evaluate(() => performance.now());
  const info = await page.evaluate(() => ({
    rows: document.querySelectorAll("[data-message-id]").length,
    error: document.body.innerText.includes("メッセージの取得に失敗"),
  }));
  results.push({ run, enabledMs: Math.round(tReady - t0), ...info });
  await context.close();
}
await browser.close();
const vals = results.map((r) => r.enabledMs);
const sorted = [...vals].sort((a, b) => a - b);
console.log(
  JSON.stringify(
    {
      label: LABEL,
      enabledMs: vals,
      medianMs: sorted[Math.floor(sorted.length / 2)],
      rows: results.map((r) => r.rows),
      errors: results.filter((r) => r.error).length,
    },
    null,
    2,
  ),
);
