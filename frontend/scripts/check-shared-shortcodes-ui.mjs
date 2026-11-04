import { chromium, expect } from "@playwright/test";
import { mkdir } from "node:fs/promises";
const origin = process.env.STORYBOOK_URL ?? "http://127.0.0.1:6006";
const browser = await chromium.launch({ headless: true });
await mkdir("test-results/shared-shortcodes", { recursive: true });
try {
  for (const implementation of ["react", "kotlin"]) {
    for (const width of [1440, 390]) {
      const context = await browser.newContext({
        viewport: { width, height: 900 },
        locale: "ja-JP",
      });
      const page = await context.newPage();
      const errors = [];
      page.on("pageerror", (error) => errors.push(error.message));
      try {
        await page.goto(
          `${origin}/iframe.html?id=features-chat-messagecontent--shared-shortcodes&viewMode=story&globals=implementation:${implementation};locale:ja;theme:light`,
        );
        const root =
          implementation === "kotlin"
            ? page.frameLocator('iframe[title="Kotlin component preview"]')
            : page;
        await expect(root.getByText("👩‍💻", { exact: false }).first()).toBeVisible({
          timeout: 90_000,
        });
        await expect(root.getByRole("img", { name: ":smile:", exact: true })).toBeVisible();
        await expect(root.getByText(":service_gemoji:", { exact: false }).first()).toBeVisible();
        await expect(
          root.getByText(":unknown_service_emoji:", { exact: false }).first(),
        ).toBeVisible();
        if (errors.length) throw Error(errors.join("\n"));
        console.log(
          `${implementation} ${width}: service / custom precedence / literal code / unknown passed`,
        );
      } finally {
        await page.screenshot({
          path: `test-results/shared-shortcodes/${implementation}-${width}.png`,
          fullPage: true,
        });
        await context.close();
      }
    }
  }
} finally {
  await browser.close();
}
