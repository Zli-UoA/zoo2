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
  await page.goto("/iframe.html?id=features-compose-chat--message&viewMode=story");
  const frame = page.frameLocator('iframe[title="Compose component catalog"]');
  await expect(page.locator("[data-compose-catalog-state=ready]")).toBeVisible({ timeout: 90_000 });
  await expect(frame.getByText("こんにちは。今日のリリース", { exact: false })).toBeVisible();
  const originalFrame = await page.locator("iframe").elementHandle();
  // The same iframe receives Controls changes without starting a second Compose scene.
  await page.evaluate(() => {
    const channel = (
      window as typeof window & {
        __STORYBOOK_ADDONS_CHANNEL__: { emit: (name: string, payload: unknown) => void };
      }
    ).__STORYBOOK_ADDONS_CHANNEL__;
    channel.emit("updateStoryArgs", {
      storyId: "features-compose-chat--message",
      updatedArgs: { content: "Kotlin Controls update" },
    });
  });
  await expect(frame.getByText("Kotlin Controls update", { exact: false })).toBeVisible();
  expect(await originalFrame!.evaluate((element) => element.isConnected)).toBe(true);
  await page.goto(
    "/iframe.html?id=features-compose-chat--composer&viewMode=story&globals=locale:en;theme:dark",
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
  await page.goto("/iframe.html?id=features-compose-chat--emoji-picker&viewMode=story");
  await expect(page.locator("[data-compose-catalog-state=ready]")).toBeVisible({ timeout: 90_000 });
  await expect(frame.getByRole("textbox")).toBeVisible();
  await expect(frame.getByText("😀", { exact: true }).first()).toBeVisible({ timeout: 30_000 });
  await expect(page.locator("iframe")).toHaveJSProperty("clientWidth", 390);
  await page.goto("/iframe.html?id=features-compose-chat--long-message&viewMode=story");
  await expect(page.locator("[data-compose-catalog-state=ready]")).toBeVisible({ timeout: 90_000 });
  await expect(frame.getByText("長い日本語の本文とURL", { exact: false }).first()).toBeVisible();
  await page.screenshot({ path: test.info().outputPath("compose-long-message-mobile.png") });
  await page.goto("/iframe.html?id=features-compose-chat--rich-text&viewMode=story");
  await expect(page.locator("[data-compose-catalog-state=ready]")).toBeVisible({ timeout: 90_000 });
  await expect(frame.getByText("Compose Multiplatform", { exact: false })).toBeVisible();
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/iframe.html?id=features-compose-chat--react-comparison&viewMode=story");
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

test("native parity catalog renders real settings, cards, Bubble and notification states", async ({
  page,
}) => {
  const frame = page.frameLocator('iframe[title="Compose component catalog"]');
  const open = async (story: string) => {
    await page.goto(`/iframe.html?id=features-compose-chat--${story}&viewMode=story`);
    await expect(page.locator("[data-compose-catalog-state=ready]")).toBeVisible({
      timeout: 90_000,
    });
  };
  await open("link-preview");
  await expect(frame.getByRole("button", { name: /^Kotlin link preview/ })).toBeVisible();
  await open("integration-cards");
  await expect(frame.getByText("Deployment to storybook-pr-271", { exact: true })).toBeVisible();
  await expect(frame.getByText("Completed", { exact: true })).toBeVisible();
  await page.screenshot({ path: test.info().outputPath("compose-integration-card-wide.png") });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({ path: test.info().outputPath("compose-integration-card-mobile.png") });
  await page.setViewportSize({ width: 1440, height: 900 });
  await open("app-card-block");
  await expect(frame.getByText("Deployment to storybook-pr-271", { exact: true })).toHaveCount(1);
  await expect(frame.getByText("この本文はカードと二重表示しません", { exact: true })).toHaveCount(
    0,
  );
  await open("settings");
  await expect(frame.getByRole("heading", { name: "Zoo設定", exact: true })).toBeVisible();
  await expect(frame.getByRole("button", { name: /^検索エンジン:/ })).toBeVisible();
  const general = frame.getByRole("tab", { name: "一般", exact: true });
  await expect(general).toHaveAttribute("aria-selected", "true");
  await general.click({ force: true });
  await page.keyboard.press("End");
  await expect(frame.getByRole("tab", { name: "メンバー", exact: true })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  await expect(frame.getByRole("button", { name: "管理者を外す", exact: true })).toBeDisabled();
  await page.keyboard.press("Home");
  await expect(general).toHaveAttribute("aria-selected", "true");
  await frame.getByRole("button", { name: /^テーマ:/ }).click({ force: true });
  await frame.getByRole("button", { name: "ダーク", exact: true }).click({ force: true });
  await expect(frame.getByRole("button", { name: "テーマ: ダーク", exact: true })).toBeVisible();
  await page.screenshot({ path: test.info().outputPath("compose-settings-dark-wide.png") });
  await open("workspace-members");
  await expect(frame.getByRole("button", { name: "管理者を外す", exact: true })).toBeDisabled();
  await page.setViewportSize({ width: 390, height: 844 });
  await open("bubble-chat");
  await expect(
    frame.getByRole("textbox", { name: "リアルタイムメッセージ入力", exact: true }),
  ).toBeVisible();
  await open("notifications-loading");
  await expect(frame.getByText("読み込み中…", { exact: true }).first()).toBeVisible();
  await open("notifications");
  await expect(frame.getByText("こんにちは。今日のリリース", { exact: false })).toBeVisible();
  await page.screenshot({ path: test.info().outputPath("compose-notifications-mobile.png") });
});
