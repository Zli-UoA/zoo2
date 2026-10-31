import { test, expect } from "@playwright/test";

test("built Storybook renders Kotlin samples, updates controls, and isolates state", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  const requests: string[] = [];
  page.on("request", (request) => {
    if (new URL(request.url()).pathname.startsWith("/api/")) requests.push(request.url());
  });
  await page.goto("/iframe.html?id=compose-components--message&viewMode=story");
  const frame = page.frameLocator('iframe[title="Compose component catalog"]');
  await expect(page.locator("[data-compose-catalog-state=ready]")).toBeVisible({ timeout: 90_000 });
  await expect(frame.getByText("こんにちは。今日のリリース", { exact: false })).toBeVisible();
  // The same iframe receives Controls changes without starting a second Compose scene.
  await page.evaluate(() => {
    const channel = (
      window as typeof window & {
        __STORYBOOK_ADDONS_CHANNEL__: { emit: (name: string, payload: unknown) => void };
      }
    ).__STORYBOOK_ADDONS_CHANNEL__;
    channel.emit("updateStoryArgs", {
      storyId: "compose-components--message",
      updatedArgs: { content: "Kotlin Controls update" },
    });
  });
  await expect(frame.getByText("Kotlin Controls update", { exact: false })).toBeVisible();
  await page.goto(
    "/iframe.html?id=compose-components--composer&viewMode=story&globals=locale:en;theme:dark",
  );
  await expect(page.locator("[data-compose-catalog-state=ready]")).toBeVisible({ timeout: 90_000 });
  const composer = frame.getByRole("textbox", { name: "Write a message", exact: true });
  await expect(composer).toBeVisible();
  await composer.click({ force: true });
  await expect
    .poll(() =>
      composer.evaluate((element) => {
        let active = element.ownerDocument.activeElement;
        while (active?.shadowRoot?.activeElement) active = active.shadowRoot.activeElement;
        return active instanceof HTMLTextAreaElement || active instanceof HTMLInputElement;
      }),
    )
    .toBe(true);
  await page.keyboard.insertText("Catalog draft");
  await expect(composer).toHaveText("Catalog draft");
  await page.reload();
  await expect(page.locator("[data-compose-catalog-state=ready]")).toBeVisible({ timeout: 90_000 });
  await expect(composer).toHaveText("");
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/iframe.html?id=compose-components--emoji-picker&viewMode=story");
  await expect(page.locator("[data-compose-catalog-state=ready]")).toBeVisible({ timeout: 90_000 });
  await expect(frame.getByRole("textbox")).toBeVisible();
  await expect(frame.getByText("😀", { exact: true }).first()).toBeVisible({ timeout: 30_000 });
  await expect(page.locator("iframe")).toHaveJSProperty("clientWidth", 390);
  await page.goto("/iframe.html?id=compose-components--rich-text&viewMode=story");
  await expect(page.locator("[data-compose-catalog-state=ready]")).toBeVisible({ timeout: 90_000 });
  await expect(frame.getByText("Compose Multiplatform", { exact: false })).toBeVisible();
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/iframe.html?id=compose-components--react-comparison&viewMode=story");
  await expect(page.locator("[data-compose-catalog-state=ready]")).toBeVisible({ timeout: 90_000 });
  await expect(
    page
      .getByRole("region", { name: "React", exact: true })
      .getByText("こんにちは。今日のリリース", { exact: false }),
  ).toBeVisible();
  await expect(frame.getByText("こんにちは。今日のリリース", { exact: false })).toBeVisible();
  const screenshot = test.info().outputPath("compose-react-comparison.png");
  await page.screenshot({ path: screenshot });
  await test
    .info()
    .attach("compose-react-comparison", { path: screenshot, contentType: "image/png" });
  expect(errors).toEqual([]);
  expect(requests).toEqual([]);
});

test("catalog explains missing Wasm assets", async ({ page }) => {
  // Open the host directly: Storybook's MSW service worker bypasses Playwright request routing.
  await page.route("**/compose/zoo-compose.js", (route) => route.abort());
  await page.goto("/compose-catalog.html");
  await page.evaluate(() =>
    window.postMessage(
      {
        type: "zoo-compose-catalog-options",
        options: { sample: "message", locale: "ja", labels: {} },
      },
      location.origin,
    ),
  );
  await expect(page.getByRole("status")).toContainText("pnpm build:compose");
});
