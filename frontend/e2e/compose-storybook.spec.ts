import { test, expect } from "@playwright/test";

const nativeQuery = "viewMode=story&globals=implementation:kotlin";

test("existing stories switch React and Kotlin in the top toolbar and share Controls", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.goto("/?path=/story/features-chat-messagerow--default");
  const preview = page.frameLocator("#storybook-preview-iframe");
  await expect(preview.getByText("こんにちは。今日のリリース", { exact: false })).toBeVisible();
  await page
    .getByRole("button")
    .filter({ hasText: /^React$/ })
    .click();
  await page.getByText("Kotlin (Compose)", { exact: true }).click();
  await expect(preview.locator("[data-compose-catalog-state=ready]")).toBeVisible({
    timeout: 90_000,
  });
  const native = preview.frameLocator('iframe[title="Kotlin component preview"]');
  await expect(native.getByText("こんにちは。今日のリリース", { exact: false })).toBeVisible();
  const originalFrame = await preview
    .locator('iframe[title="Kotlin component preview"]')
    .elementHandle();
  await page
    .frameLocator("#storybook-preview-iframe")
    .locator("body")
    .evaluate(() => {
      const channel = (
        window as typeof window & {
          __STORYBOOK_ADDONS_CHANNEL__: { emit: (name: string, payload: unknown) => void };
        }
      ).__STORYBOOK_ADDONS_CHANNEL__;
      channel.emit("updateStoryArgs", {
        storyId: "features-chat-messagerow--default",
        updatedArgs: {
          item: {
            message: {
              id: "message-1",
              user_id: "user-1",
              user_name: "alice",
              display_name: "Alice",
              content: "Shared Controls update",
              created_at: "2026-01-02T09:00:00Z",
              parent_id: null,
            },
            isGroupStart: true,
            isGroupEnd: true,
            showDateSeparator: false,
          },
        },
      });
    });
  await expect(native.getByText("Shared Controls update", { exact: false })).toBeVisible();
  expect(await originalFrame!.evaluate((e) => e.isConnected)).toBe(true);
  await page
    .getByRole("button")
    .filter({ hasText: /^Kotlin \(Compose\)$/ })
    .click();
  await page.getByText("React", { exact: true }).click();
  await expect(preview.locator('iframe[title="Kotlin component preview"]')).toHaveCount(0);
  await expect(preview.getByText("Shared Controls update", { exact: true })).toBeVisible();
  expect(new URL(page.url()).searchParams.get("path")).toBe(
    "/story/features-chat-messagerow--default",
  );
  const index = await (await page.request.get("/index.json")).json();
  expect(Object.keys(index.entries).some((id) => id.startsWith("features-compose-"))).toBe(false);
  expect(errors).toEqual([]);
});

test("Kotlin previews preserve locale, theme, draft isolation and mobile layouts", async ({
  page,
}) => {
  const frame = page.frameLocator('iframe[title="Kotlin component preview"]');
  await page.goto(
    `/iframe.html?id=features-chat-composer--default&${nativeQuery};locale:en;theme:dark`,
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
  await page.goto(`/iframe.html?id=features-chat-reactionpicker--default&${nativeQuery}`);
  await expect(page.locator("[data-compose-catalog-state=ready]")).toBeVisible({ timeout: 90_000 });
  await expect(frame.getByRole("textbox")).toBeVisible();
  await expect(frame.getByText("😀", { exact: true }).first()).toBeVisible({ timeout: 30_000 });
  await expect(page.locator('iframe[title="Kotlin component preview"]')).toHaveJSProperty(
    "clientWidth",
    390,
  );
  await page.goto(`/iframe.html?id=features-chat-messagerow--long-message&${nativeQuery}`);
  await expect(page.locator("[data-compose-catalog-state=ready]")).toBeVisible({ timeout: 90_000 });
  await expect(
    frame.getByText("リリース手順の確認項目です", { exact: false }).first(),
  ).toBeVisible();
  await page.screenshot({ path: test.info().outputPath("kotlin-long-message-mobile.png") });
  await page.goto(`/iframe.html?id=features-chat-messagecontent--code-and-math&${nativeQuery}`);
  await expect(page.locator("[data-compose-catalog-state=ready]")).toBeVisible({ timeout: 90_000 });
  await expect(frame.getByText("const zoo = true", { exact: false })).toBeVisible();
});

test("unregistered Kotlin stories are explicit and never render React as Kotlin", async ({
  page,
}) => {
  await page.goto(`/iframe.html?id=components-button--default&${nativeQuery}`);
  await expect(page.locator('[data-kotlin-story-status="unavailable"]')).toContainText("未登録");
  await expect(page.locator('iframe[title="Kotlin component preview"]')).toHaveCount(0);
  await expect(page.getByRole("button")).toHaveCount(0);
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
  const frame = page.frameLocator('iframe[title="Kotlin component preview"]');
  const open = async (story: string) => {
    const ids: Record<string, string> = {
      "link-preview": "features-chat-linkpreviews--landscape",
      "integration-cards": "features-chat-messagecard--deployment",
      "app-card-block": "features-apps-blocks--card",
      "composer-send-error": "features-chat-composer--send-error",
      "with-attachments": "features-chat-messagerow--with-attachments",
      settings: "pages-zoosettings--general",
      "workspace-members": "pages-zoosettings--members",
      "bubble-chat": "pages-bubblechat--empty",
      "notifications-loading": "pages-notifications--loading",
      notifications: "pages-notifications--default",
    };
    await page.goto(`/iframe.html?id=${ids[story]}&${nativeQuery}`);
    await expect(page.locator("[data-compose-catalog-state=ready]")).toBeVisible({
      timeout: 90_000,
    });
  };
  await open("link-preview");
  await expect(frame.getByRole("button", { name: /^Zooのリリース情報/ })).toBeVisible();
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
  await open("composer-send-error");
  const failingComposer = frame.getByRole("textbox", { name: "メッセージを送信", exact: true });
  await failingComposer.click({ force: true });
  await expect
    .poll(() =>
      failingComposer.evaluate((element) => {
        let active = element.ownerDocument.activeElement;
        while (active?.shadowRoot?.activeElement) active = active.shadowRoot.activeElement;
        return active instanceof HTMLTextAreaElement || active instanceof HTMLInputElement;
      }),
    )
    .toBe(true);
  await page.keyboard.insertText("restore after failure");
  await expect(failingComposer).toHaveText("restore after failure");
  await frame.getByRole("button", { name: "送信", exact: true }).click({ force: true });
  await expect(frame.getByText("操作に失敗しました", { exact: true })).toBeVisible();
  await expect(failingComposer).toHaveText("restore after failure");
  await expect(frame.getByRole("button", { name: "送信", exact: true })).toBeEnabled();
  await open("with-attachments");
  await expect(
    frame.getByRole("button", { name: "screenshot.pngをプレビュー", exact: true }),
  ).toBeVisible();
  await expect(
    frame.getByRole("button", { name: "diagram.pngをプレビュー", exact: true }),
  ).toBeVisible();
  await expect(frame.getByText("release-notes.pdf", { exact: true })).toBeVisible();
  await page.screenshot({ path: test.info().outputPath("compose-attachment-grid-wide.png") });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({ path: test.info().outputPath("compose-attachment-grid-mobile.png") });
  await frame
    .getByRole("button", { name: "screenshot.pngをプレビュー", exact: true })
    .click({ force: true });
  await expect(frame.getByRole("img", { name: "screenshot.png", exact: true })).toBeVisible();
  await page.screenshot({ path: test.info().outputPath("compose-attachment-preview-mobile.png") });
  await frame.getByRole("button", { name: "閉じる", exact: true }).click({ force: true });
  await expect(frame.getByRole("img", { name: "screenshot.png", exact: true })).toBeHidden();

  await page.setViewportSize({ width: 1440, height: 900 });
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
  await expect(frame.getByText("Alice", { exact: true })).toBeVisible();
  await expect(frame.getByRole("button", { name: "管理者を外す", exact: true })).toHaveCount(0);
  await page.keyboard.press("Home");
  await expect(general).toHaveAttribute("aria-selected", "true");
  await frame.getByRole("button", { name: /^テーマ:/ }).click({ force: true });
  await frame.getByRole("button", { name: "ダーク", exact: true }).click({ force: true });
  await expect(frame.getByRole("button", { name: "テーマ: ダーク", exact: true })).toBeVisible();
  await page.screenshot({ path: test.info().outputPath("compose-settings-dark-wide.png") });
  await open("workspace-members");
  await expect(
    frame.getByRole("button", { name: "管理者を外す", exact: true }).first(),
  ).toBeVisible();
  await page.setViewportSize({ width: 390, height: 844 });
  await open("bubble-chat");
  await expect(
    frame.getByRole("textbox", { name: "リアルタイムメッセージ入力", exact: true }),
  ).toBeVisible();
  await open("notifications-loading");
  await expect(frame.getByText("読み込み中…", { exact: true }).first()).toBeVisible();
  await open("notifications");
  await expect(frame.getByText("今日のリリースは 18:00", { exact: false })).toBeVisible();
  await page.screenshot({ path: test.info().outputPath("compose-notifications-mobile.png") });
});
