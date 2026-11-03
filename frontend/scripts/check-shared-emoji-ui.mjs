import { chromium, expect } from "@playwright/test";
import { mkdir } from "node:fs/promises";
const origin = process.env.STORYBOOK_URL ?? "http://127.0.0.1:6006";
const browser = await chromium.launch({ headless: true });
await mkdir("test-results/shared-emoji", { recursive: true });
try {
  for (const implementation of ["react", "kotlin"]) {
    for (const [suffix, width] of [
      ["shared-emoji-service", 1440],
      ["shared-emoji-service-mobile", 390],
      ["shared-emoji-service-error", 1440],
    ]) {
      const context = await browser.newContext({
        viewport: { width, height: 900 },
        locale: "ja-JP",
      });
      const page = await context.newPage();
      const errors = [];
      page.on("pageerror", (error) => errors.push(error.message));
      try {
        await page.goto(
          `${origin}/iframe.html?id=pages-zoosettings--${suffix}&viewMode=story&globals=implementation:${implementation};locale:ja;theme:light`,
        );
        let root = page;
        if (implementation === "kotlin")
          root = page.frameLocator('iframe[title="Kotlin component preview"]');
        if (suffix.endsWith("error"))
          await expect(root.getByRole("alert").first()).toBeVisible({ timeout: 90_000 });
        else {
          const link = root.getByRole("link", { name: "絵文字管理サービスを開く", exact: true });
          await expect(link).toBeVisible({ timeout: 90_000 });
          await expect(link).toHaveAttribute("href", "https://emoji.example.test");
          await expect(root.getByRole("textbox", { name: "名前", exact: true })).toHaveCount(0);
        }
        await page.screenshot({
          path: `test-results/shared-emoji/${implementation}-${suffix}.png`,
          fullPage: true,
        });
        if (errors.length) throw Error(errors.join("\n"));
        console.log(`${implementation}: ${suffix} passed`);
      } finally {
        await context.close();
      }
    }
  }
} finally {
  await browser.close();
}
