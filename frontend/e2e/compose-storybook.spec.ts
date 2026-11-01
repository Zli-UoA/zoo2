import { test, expect, type Page } from "@playwright/test";

const nativeQuery = "viewMode=story&globals=implementation:kotlin";

async function waitForStoryPlay(page: Page) {
  await page.waitForFunction(() => {
    const preview = window as typeof window & {
      __STORYBOOK_PREVIEW__?: { currentRender?: { phase: string } };
    };
    return preview.__STORYBOOK_PREVIEW__?.currentRender?.phase === "finished";
  });
}

async function openNativeStory(page: Page, story: string) {
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
}

test("existing reaction-user stories render Kotlin names on hover and in the mobile list", async ({
  page,
}) => {
  const frame = page.frameLocator('iframe[title="Kotlin component preview"]');
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(`/iframe.html?id=features-chat-messagerow--reaction-users&${nativeQuery}`);
  await expect(page.locator("[data-compose-catalog-state=ready]")).toBeVisible({ timeout: 90_000 });
  // The existing story's play opens the list; wait before checking its result.
  await waitForStoryPlay(page);
  await expect(frame.getByText(/A{80} \(@longname\)/)).toBeVisible();
  await expect(frame.getByText(/Bob \(@bob\)/)).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(frame.getByText(/A{80} \(@longname\)/)).toBeHidden();
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(`/iframe.html?id=features-chat-messagerow--reaction-users-hover&${nativeQuery}`);
  await expect(page.locator("[data-compose-catalog-state=ready]")).toBeVisible({ timeout: 90_000 });
  await frame.getByRole("button", { name: /👍 3 件/ }).hover({ force: true });
  await expect(frame.getByText(/Alice \(@alice\), Bob \(@bob\)/)).toBeVisible();
});

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

test("shared button variants have the same geometry in React and Kotlin", async ({ page }) => {
  const boxes: { x: number; y: number; width: number; height: number }[][] = [];
  for (const implementation of ["react", "kotlin"]) {
    await page.goto(
      `/iframe.html?id=components-button--variants&viewMode=story&globals=implementation:${implementation}`,
    );
    const root =
      implementation === "kotlin"
        ? page.frameLocator('iframe[title="Kotlin component preview"]')
        : page;
    if (implementation === "kotlin")
      await expect(page.locator("[data-compose-catalog-state=ready]")).toBeVisible({
        timeout: 90_000,
      });
    await page.mouse.move(0, 0);
    const result = [];
    for (const variant of ["default", "outline", "secondary", "ghost", "destructive", "link"]) {
      const button = root.getByRole("button", { name: variant, exact: true });
      await expect(button).toBeVisible();
      await button.evaluate(async (element) => {
        await element.ownerDocument.fonts.ready;
      });
      result.push((await button.boundingBox())!);
    }
    boxes.push(result);
    await page.screenshot({
      path: test.info().outputPath(`button-variants-${implementation}.png`),
    });
  }
  await test
    .info()
    .attach("button-geometry", { body: JSON.stringify(boxes), contentType: "application/json" });
  for (let i = 0; i < boxes[0].length; i++) {
    for (const key of ["x", "y", "width", "height"] as const)
      expect
        .soft(Math.abs(boxes[0][i][key] - boxes[1][i][key]), `button ${i} ${key}`)
        .toBeLessThanOrEqual(1);
  }
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

test("native cards share metadata and restore failed composer drafts", async ({ page }) => {
  const frame = page.frameLocator('iframe[title="Kotlin component preview"]');
  const open = (story: string) => openNativeStory(page, story);
  await open("link-preview");
  await expect(frame.getByRole("link", { name: /^Zooのリリース情報/ })).toBeVisible();
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
  await waitForStoryPlay(page);
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
  await page.keyboard.press("ControlOrMeta+A");
  await page.keyboard.insertText("restore after failure");
  await expect(failingComposer).toHaveText("restore after failure");
  await frame.getByRole("button", { name: "送信", exact: true }).click({ force: true });
  // The shared onSend callback returns false; both implementations restore the draft.
  await expect(failingComposer).toHaveText("restore after failure");
  await expect(frame.getByRole("button", { name: "送信", exact: true })).toBeEnabled();
});

test("native attachments render from the existing shared story", async ({ page }) => {
  const frame = page.frameLocator('iframe[title="Kotlin component preview"]');
  const open = (story: string) => openNativeStory(page, story);
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
  const previewImage = frame.getByRole("button", {
    name: "screenshot.pngをプレビュー",
    exact: true,
  });
  await expect(previewImage).toBeVisible();
  await previewImage.click({ force: true });
  await expect(frame.getByRole("img", { name: "screenshot.png", exact: true })).toBeVisible();
  await page.screenshot({ path: test.info().outputPath("compose-attachment-preview-mobile.png") });
  await frame.getByRole("button", { name: "閉じる", exact: true }).click({ force: true });
  await expect(frame.getByRole("img", { name: "screenshot.png", exact: true })).toBeHidden();
});

test("native settings, Bubble and notifications use the existing story fixtures", async ({
  page,
}) => {
  const frame = page.frameLocator('iframe[title="Kotlin component preview"]');
  const open = (story: string) => openNativeStory(page, story);
  await open("settings");
  await expect(frame.getByRole("heading", { name: "Zoo設定", exact: true })).toBeVisible();
  await expect(frame.getByRole("combobox", { name: "検索エンジン", exact: true })).toBeVisible();
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
  await frame.getByRole("button", { name: "ダーク", exact: true }).click({ force: true });
  await expect(frame.getByRole("button", { name: "ダーク", exact: true })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
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

test.describe("Kotlin message touch gestures", () => {
  test.use({
    baseURL: process.env.STORYBOOK_PARITY_URL ?? "http://127.0.0.1:6008",
    viewport: { width: 390, height: 844 },
    hasTouch: true,
    isMobile: true,
    permissions: ["clipboard-read", "clipboard-write"],
  });
  async function openTouchMessage(page: Page) {
    await page.addInitScript(() => {
      const state = window as typeof window & {
        __messageMenuOverlay?: boolean;
        __messageMenuCallbacks: { name: string; args: unknown[] }[];
      };
      state.__messageMenuCallbacks = [];
      addEventListener("message", (event) => {
        if (event.origin !== location.origin) return;
        if (event.data?.type === "zoo-compose-catalog-overlay")
          state.__messageMenuOverlay = event.data.open === true;
        if (event.data?.type === "zoo-compose-catalog-callback")
          state.__messageMenuCallbacks.push(JSON.parse(String(event.data.payload)));
      });
    });
    await page.goto(`/iframe.html?id=features-chat-messagerow--default&${nativeQuery};locale:ja`);
    await expect(page.locator("[data-compose-catalog-state=ready]")).toBeVisible({
      timeout: 90_000,
    });
    const frame = page.frames().find((item) => item.url().includes("/compose-catalog.html"));
    if (!frame) throw new Error("Native message frame is missing");
    await frame.evaluate(() => {
      const state = window as typeof window & { __messageMenuReplies: { ok: boolean }[] };
      state.__messageMenuReplies = [];
      addEventListener("message", (event) => {
        if (
          event.origin === location.origin &&
          event.data?.type === "zoo-compose-catalog-callback-result"
        )
          state.__messageMenuReplies.push(event.data);
      });
    });
    const body = frame.getByText(/こんにちは/).last();
    await expect(body).toBeVisible();
    const session = await page.context().newCDPSession(page);
    const dispatch = async (
      type: "touchStart" | "touchMove" | "touchEnd",
      point?: { x: number; y: number },
    ) => {
      if (type !== "touchEnd" && !point) throw new Error("Touch coordinates are missing");
      await session.send("Input.dispatchTouchEvent", {
        type,
        touchPoints: type === "touchEnd" ? [] : [{ x: point!.x, y: point!.y, id: 1 }],
      });
    };
    const tap = async (target: import("@playwright/test").Locator) => {
      const bounds = await target.boundingBox();
      if (!bounds) throw new Error("Native touch target has no bounds");
      await dispatch("touchStart", {
        x: bounds.x + bounds.width / 2,
        y: bounds.y + bounds.height / 2,
      });
      await dispatch("touchEnd");
    };
    const hold = async () => {
      const bounds = await body.boundingBox();
      if (!bounds) throw new Error("Native message has no bounds");
      await dispatch("touchStart", { x: bounds.x + 20, y: bounds.y + 8 });
      await expect(frame.getByRole("menu")).toBeVisible();
      await dispatch("touchEnd");
      await page.evaluate(
        () =>
          new Promise<void>((resolve) =>
            requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
          ),
      );
      await expect
        .poll(() =>
          page.evaluate(
            () =>
              (window as typeof window & { __messageMenuOverlay?: boolean }).__messageMenuOverlay,
          ),
        )
        .toBe(true);
      await expect(frame.getByRole("menuitem", { name: "リアクションを追加" })).toBeVisible();
    };
    const pickReaction = async () => {
      await tap(frame.getByRole("menuitem", { name: "リアクションを追加" }));
      await expect
        .poll(() =>
          page.evaluate(
            () =>
              (
                window as typeof window & {
                  __messageMenuCallbacks: { name: string; args: unknown[] }[];
                }
              ).__messageMenuCallbacks,
          ),
        )
        .toContainEqual({ name: "onPickReaction", args: [] });
      await expect
        .poll(() =>
          frame.evaluate(() =>
            (
              window as typeof window & { __messageMenuReplies: { ok: boolean }[] }
            ).__messageMenuReplies.some((reply) => reply.ok),
          ),
        )
        .toBe(true);
      await expect(frame.getByRole("menu")).toBeHidden();
    };
    return { frame, body, session, dispatch, tap, hold, pickReaction };
  }

  test("a held message menu remains usable after releasing the touch", async ({ page }) => {
    const message = await openTouchMessage(page);
    await message.hold();
    await message.pickReaction();
    await message.session.detach();
  });
  test("Canvas text supports real mouse selection and keyboard copying", async ({ page }) => {
    const message = await openTouchMessage(page);
    const bounds = await message.body.boundingBox();
    if (!bounds) throw new Error("Native selection target has no bounds");
    await page.mouse.move(bounds.x + 2, bounds.y + 10);
    await page.mouse.down();
    await page.mouse.move(bounds.x + 180, bounds.y + 10, { steps: 20 });
    await page.mouse.up();
    // Select the real Canvas text, not its intentionally user-select:none AX proxy.
    await message.frame.evaluate(() => navigator.clipboard.writeText(""));
    await page.keyboard.press("Control+c");
    await expect
      .poll(() => message.frame.evaluate(() => navigator.clipboard.readText()))
      .toContain("こんにちは");
    const selected = await message.frame.evaluate(() => navigator.clipboard.readText());
    expect(await message.body.textContent()).toContain(selected);
    await message.session.detach();
  });
  test("dragging or holding an inline link does not open the message menu", async ({ page }) => {
    for (const gesture of ["drag", "link"] as const) {
      const message = await openTouchMessage(page);
      const target =
        gesture === "link"
          ? message.frame.getByRole("link", { name: "example.com/docs", exact: true })
          : message.body;
      const bounds = await target.boundingBox();
      if (!bounds) throw new Error("Native gesture target has no bounds");
      const point = { x: bounds.x + 20, y: bounds.y + 8 };
      await message.dispatch("touchStart", point);
      if (gesture === "drag") await message.dispatch("touchMove", { x: point.x + 24, y: point.y });
      // An actual negative hold must outlast the product's 500ms gesture deadline.
      await page.waitForTimeout(650);
      await expect(message.frame.getByRole("menu")).toBeHidden();
      await message.dispatch("touchEnd");
      await expect(message.frame.getByRole("menu")).toBeHidden();
      await expect
        .poll(() =>
          page.evaluate(
            () =>
              (window as typeof window & { __messageMenuOverlay?: boolean }).__messageMenuOverlay,
          ),
        )
        .toBe(false);
      await message.session.detach();
    }
  });
  test("copying a message writes its original source into the real clipboard", async ({ page }) => {
    const message = await openTouchMessage(page);
    await message.hold();
    // The actual touch focuses the iframe before using its real Clipboard API.
    await message.frame.evaluate(() => navigator.clipboard.writeText(""));
    await message.tap(message.frame.getByRole("menuitem", { name: "原文をコピー" }));
    await expect
      .poll(() => message.frame.evaluate(() => navigator.clipboard.readText()))
      .toBe(
        "こんにちは。今日のリリースは 18:00 です。詳細は https://example.com/docs を参照してください。",
      );
    await message.session.detach();
  });
});
