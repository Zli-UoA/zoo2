import { test, expect, type Page } from "@playwright/test";

async function open(page: Page, implementation: "react" | "compose", path = "/channels/c1") {
  await page.addInitScript(() => {
    localStorage.setItem("zoo-locale", "ja");
    localStorage.setItem("zoo-theme", "light");
  });
  await page.clock.setFixedTime(new Date("2026-10-31T01:00:00Z"));
  await page.goto(`${path}?frontend=${implementation}`);
  await expect(page.getByText("こんにちは Parity", { exact: false })).toBeVisible({
    timeout: 90_000,
  });
  if (implementation === "compose")
    await expect(page.locator("[data-compose-shell]")).toBeVisible();
  await page.evaluate(async () => {
    await document.fonts.ready;
  });
}

async function channelAction(page: Page, native: boolean, action: "edit" | "webhook") {
  if (native) {
    await page
      .getByRole("button", { name: "チャンネルの管理", exact: true })
      .click({ force: true });
    await page
      .getByRole("button", {
        name: action === "edit" ? "チャンネルを編集" : "Webhook設定",
        exact: true,
      })
      .click({ force: true });
    return;
  }
  const settings = page.getByRole("dialog", { name: "チャンネル設定", exact: true });
  if (!(await settings.isVisible()))
    await page.getByRole("button", { name: "チャンネル設定", exact: true }).click();
  await settings
    .getByRole("button", { name: action === "edit" ? "編集" : "開く", exact: true })
    .click();
}
async function closeChannelSettings(page: Page, native: boolean) {
  await page.keyboard.press("Escape");
  if (!native) {
    await expect(page.getByRole("dialog", { name: "Webhook設定", exact: true })).toBeHidden();
    await page.keyboard.press("Escape");
  }
}
async function openApps(page: Page, native: boolean) {
  if (native)
    await page.getByRole("button", { name: "App・連携", exact: true }).click({ force: true });
  else {
    await page.getByRole("link", { name: "Zoo設定", exact: true }).click();
    await page.getByRole("tab", { name: "App・連携", exact: true }).click();
  }
}
async function closeApps(page: Page, native: boolean) {
  if (native) await page.keyboard.press("Escape");
  else await page.getByRole("link", { name: "チャンネル", exact: true }).click();
}

async function nativeInput(page: Page) {
  return page.evaluate(() => {
    let active = document.activeElement;
    while (active?.shadowRoot?.activeElement) active = active.shadowRoot.activeElement;
    return active instanceof HTMLTextAreaElement || active instanceof HTMLInputElement
      ? { value: active.value, start: active.selectionStart, end: active.selectionEnd }
      : null;
  });
}

async function fillNative(page: Page, locator: ReturnType<Page["getByRole"]>, value: string) {
  const existing = (await locator.textContent()) ?? "";
  await locator.click({ force: true });
  // Compose applies focus and selection to its backing input asynchronously.
  await expect.poll(async () => (await nativeInput(page))?.value).toBe(existing);
  await page.keyboard.press("ControlOrMeta+A");
  // Compose's Web input processor commits queued key events on the next animation frame.
  await page.evaluate(
    () =>
      new Promise<void>((resolve) =>
        requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
      ),
  );
  await expect
    .poll(() => nativeInput(page))
    .toEqual({ value: existing, start: 0, end: existing.length });
  await page.keyboard.press("Backspace");
  await expect(locator).toHaveText("");
  await page.keyboard.insertText(value);
  await expect(locator).toHaveText(value);
}

for (const implementation of ["react", "compose"] as const) {
  test(`${implementation}: post, edit, thread, notification, App and Webhook entry points`, async ({
    page,
    request,
  }) => {
    await request.post("https://127.0.0.1:4187/__reset");
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await open(page, implementation);
    const composer = page.getByRole("textbox", { name: "メッセージを送信", exact: true });
    const fill = async (locator: ReturnType<Page["getByRole"]>, value: string) => {
      if (implementation === "react") await locator.fill(value);
      else {
        await fillNative(page, locator, value);
      }
    };
    await fill(composer, "送信した本文");
    await page
      .getByRole("button", { name: "送信", exact: true })
      .click({ force: implementation === "compose" });
    if (implementation === "compose") await expect(composer).toHaveText("");
    await expect(page.getByText("送信した本文", { exact: true })).toBeVisible();
    if (implementation === "react") await page.getByText("送信した本文", { exact: true }).hover();
    const item = page
      .locator(implementation === "react" ? "[data-message-id]" : '[id^="message:"]')
      .filter({ hasText: "送信した本文" });
    if (implementation === "compose") {
      const rect = await item.boundingBox();
      if (rect) await page.mouse.move(rect.x + rect.width - 20, rect.y + 10);
    }
    const more = item.getByRole("button", { name: "その他の操作" });
    if (implementation === "react") await more.press("Enter");
    else await more.click({ force: true });
    await page
      .getByRole(implementation === "react" ? "menuitem" : "button", { name: "編集", exact: true })
      .click({ force: implementation === "compose" });
    await fill(
      page.getByRole("textbox", { name: "メッセージを編集", exact: true }),
      "編集した本文",
    );
    await page
      .getByRole("button", { name: "保存", exact: true })
      .click({ force: implementation === "compose" });
    await expect(page.getByRole("textbox", { name: "メッセージを編集", exact: true })).toBeHidden();
    await expect(page.getByText("編集した本文", { exact: true })).toBeVisible();
    await page
      .getByRole("button", { name: /1 件の返信（スレッドを開く）/ })
      .click({ force: implementation === "compose" });
    await expect(page.getByText("返信の本文", { exact: true })).toBeVisible();
    const replyComposer = page.getByRole("textbox", { name: "返信を送信", exact: true });
    await fill(replyComposer, "スレッドへの返信");
    await page
      .getByRole("button", { name: "送信", exact: true })
      .last()
      .click({ force: implementation === "compose" });
    if (implementation === "compose") await expect(replyComposer).toHaveText("");
    await expect(page.getByText("スレッドへの返信", { exact: true })).toBeVisible();
    await expect(page.getByRole("button", { name: "スレッドを閉じる" })).toBeVisible();
    await page
      .getByRole("button", { name: "スレッドを閉じる" })
      .click({ force: implementation === "compose" });
    await channelAction(page, implementation === "compose", "webhook");
    const webhookDialog =
      implementation === "react"
        ? page.getByRole("dialog", { name: "Webhook設定", exact: true })
        : page.getByText("Webhook設定", { exact: true });
    await expect(webhookDialog).toBeVisible();
    await closeChannelSettings(page, implementation === "compose");
    await expect(webhookDialog).toBeHidden();
    await openApps(page, implementation === "compose");
    const createApp = page.getByRole("button", { name: "Appを作成", exact: true });
    await expect(createApp).toBeVisible();
    await closeApps(page, implementation === "compose");
    await expect(createApp).toBeHidden();
    await page
      .getByRole(implementation === "react" ? "link" : "button", {
        name: implementation === "react" ? /^未読通知 / : "通知",
        exact: true,
      })
      .click({ force: implementation === "compose" });
    await expect(
      implementation === "react"
        ? page.getByRole("heading", { name: "通知", exact: true })
        : page.getByText("通知", { exact: true }),
    ).toBeVisible();
    expect(errors).toEqual([]);
  });
}

for (const implementation of ["react", "compose"] as const) {
  test(`${implementation}: channel, Webhook and App CRUD`, async ({ page, request }) => {
    await request.post("https://127.0.0.1:4187/__reset");
    await open(page, implementation);
    const native = implementation === "compose";
    const click = async (name: string) =>
      page.getByRole("button", { name, exact: true }).click({ force: native });
    const fill = async (name: string, value: string) => {
      const input = page.getByRole("textbox", { name, exact: true });
      if (!native) await input.fill(value);
      else {
        await fillNative(page, input, value);
      }
    };
    const state = async () => (await request.get("https://127.0.0.1:4187/__state")).json();
    await click("チャンネルを作成");
    await fill("新しいチャンネル名", "native-parity");
    await click("チャンネル作成");
    await expect.poll(async () => (await state()).channels.length).toBe(3);
    const channelId = (await state()).channels.find(
      (c: { channel_name: string }) => c.channel_name === "native-parity",
    ).id as string;
    await expect(
      page.getByRole("textbox", { name: "新しいチャンネル名", exact: true }),
    ).toBeHidden();
    await page
      .getByRole(native ? "button" : "link", { name: "native-parity", exact: true })
      .click({ force: native });
    await expect(page).toHaveURL(new RegExp(`/channels/${channelId}`));
    await expect
      .poll(async () =>
        (await state()).channel_users.some(
          (m: { channel_id: string }) => m.channel_id === channelId,
        ),
      )
      .toBe(true);
    await channelAction(page, native, "edit");
    await fill("新しいチャンネル名", "renamed-parity");
    await click("保存");
    await expect
      .poll(
        async () =>
          (await state()).channels.find((c: { id: string }) => c.id === channelId)?.channel_name,
      )
      .toBe("renamed-parity");
    await expect(
      page.getByRole("textbox", { name: "新しいチャンネル名", exact: true }),
    ).toBeHidden();
    await channelAction(page, native, "webhook");
    await fill("Webhook名", "Parity hook");
    await click("作成");
    await expect(page.getByText("Parity hook", { exact: true })).toBeVisible();
    await click("Webhook URLを再発行");
    await click("実行");
    await expect.poll(async () => (await state()).hooks[0]?.secret).toBe("rotated-secret");
    await expect(page.getByRole("button", { name: "実行", exact: true })).toBeHidden();
    await click("削除");
    await click("実行");
    await expect(page.getByText("Webhookはまだありません。", { exact: true })).toBeVisible();
    await closeChannelSettings(page, native);
    await expect(page.getByRole("textbox", { name: "Webhook名", exact: true })).toBeHidden();
    await openApps(page, native);
    await click("Appを作成");
    await fill("App名", "Parity App");
    await fill("説明", "Kotlinと既存フロントの共通操作");
    await click("保存");
    await expect.poll(async () => (await state()).apps[0]?.name).toBe("Parity App");
    await expect(page.getByRole("textbox", { name: "App名", exact: true })).toBeHidden();
    await click("編集");
    await fill("App名", "Updated App");
    await click("保存");
    await expect.poll(async () => (await state()).apps[0]?.name).toBe("Updated App");
    await expect(page.getByRole("textbox", { name: "App名", exact: true })).toBeHidden();
    await closeApps(page, native);
    await expect(page.getByRole("button", { name: "Appを作成", exact: true })).toBeHidden();
  });
}

test("Compose storage failure can be retried without losing the native session", async ({
  page,
  request,
}) => {
  await request.post("https://127.0.0.1:4187/__reset");
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.addInitScript(() => {
    const OriginalWorker = globalThis.Worker;
    const state = globalThis as unknown as { zooStorageFault: boolean };
    state.zooStorageFault = true;
    globalThis.Worker = class extends OriginalWorker {
      constructor(script: string | URL, options?: WorkerOptions) {
        if (state.zooStorageFault && String(script).includes("opfs-worker"))
          throw new DOMException("test quota", "QuotaExceededError");
        super(script, options);
      }
    };
  });
  await open(page, "compose");
  await expect(page.getByText("test quota", { exact: false })).toBeVisible();
  await page.evaluate(() => {
    (globalThis as unknown as { zooStorageFault: boolean }).zooStorageFault = false;
  });
  await page.getByRole("button", { name: "再試行", exact: true }).click({ force: true });
  await expect(page.getByText("test quota", { exact: false })).toBeHidden();
  await expect(page.getByRole("textbox", { name: "メッセージを送信", exact: true })).toBeVisible();
  await expect(page.getByText("こんにちは Parity", { exact: false })).toBeVisible();
  expect(errors).toEqual([]);
});

test("Compose panes preserve the existing desktop and mobile UI", async ({ browser, request }) => {
  for (const viewport of [
    { width: 1440, height: 900 },
    { width: 390, height: 844 },
  ]) {
    const images: Buffer[] = [];
    for (const implementation of ["react", "compose"] as const) {
      await request.post("https://127.0.0.1:4187/__reset");
      const context = await browser.newContext({
        viewport,
        ignoreHTTPSErrors: true,
        locale: "ja-JP",
        timezoneId: "Asia/Tokyo",
      });
      const page = await context.newPage();
      await open(page, implementation);
      await page.mouse.move(0, 0);
      images.push(
        await page.screenshot({
          animations: "disabled",
          caret: "hide",
          mask: [page.locator("[data-frontend-switch], #frontend-switch")],
        }),
      );
      await context.close();
    }
    for (const [index, body] of images.entries())
      await test.info().attach(`${index === 0 ? "react" : "compose"}-${viewport.width}`, {
        body,
        contentType: "image/png",
      });
    expect.soft(images[1].equals(images[0]), `UI differs at ${viewport.width}px`).toBe(true);
  }
});

test("Compose entry points keep React settings accessible and restore the selected frontend", async ({
  page,
  request,
}) => {
  await request.post("https://127.0.0.1:4187/__reset");
  await open(page, "compose");
  await page.goto("/settings?frontend=compose");
  await expect(page.getByRole("heading", { name: "Zoo設定", exact: true })).toBeVisible();
  await expect(page.locator("[data-compose-shell]")).toHaveCount(0);
  expect(await page.evaluate(() => localStorage.getItem("zoo-frontend"))).toBe("compose");
  await page.getByRole("tab", { name: "App・連携", exact: true }).click();
  await expect(page.getByRole("button", { name: "Appを作成", exact: true })).toBeVisible();
  await page.getByRole("link", { name: "チャンネル", exact: true }).click();
  await expect(page.locator("[data-compose-shell]")).toBeVisible({ timeout: 90_000 });
  await expect(page.getByRole("button", { name: "general", exact: true })).toBeVisible();
});
