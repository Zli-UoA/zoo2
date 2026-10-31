import { test, expect, type Page } from "@playwright/test";

async function open(
  page: Page,
  implementation: "react" | "compose",
  path = "/channels/c1",
  readyText = "こんにちは Parity",
) {
  await page.addInitScript(() => {
    localStorage.setItem("zoo-locale", "ja");
    localStorage.setItem("zoo-theme", "light");
  });
  await page.clock.setFixedTime(new Date("2026-10-31T01:00:00Z"));
  await page.goto(`${path}?frontend=${implementation}`);
  await expect(page.getByText(readyText, { exact: false })).toBeVisible({
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
  await page
    .getByRole(native ? "button" : "link", { name: "Zoo設定", exact: true })
    .click({ force: native });
  await page.getByRole("tab", { name: "App・連携", exact: true }).click({ force: native });
}
async function closeApps(page: Page, native: boolean) {
  await page
    .getByRole("button", { name: "チャンネル一覧へ戻る", exact: true })
    .click({ force: native });
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

test("Compose entry points render Kotlin settings and preserve the selected frontend", async ({
  page,
  request,
}) => {
  await request.post("https://127.0.0.1:4187/__reset");
  await open(page, "compose");
  await page.goto("/settings?frontend=compose");
  await expect(page.getByRole("heading", { name: "Zoo設定", exact: true })).toBeVisible({
    timeout: 90000,
  });
  await expect(page.locator("[data-compose-shell]")).toBeVisible();
  expect(await page.evaluate(() => localStorage.getItem("zoo-frontend"))).toBe("compose");
  await page.getByRole("tab", { name: "App・連携", exact: true }).click({ force: true });
  await expect(page.getByRole("button", { name: "Appを作成", exact: true })).toBeVisible();
  await page
    .getByRole("button", { name: "チャンネル一覧へ戻る", exact: true })
    .click({ force: true });
  await expect(page.locator("[data-compose-shell]")).toBeVisible({ timeout: 90_000 });
  await expect(page.getByRole("button", { name: "general", exact: true })).toBeVisible();
});

for (const implementation of ["react", "compose"] as const) {
  test(`${implementation}: parity entry points for completion, preferences, roles, Bubble and URL cards`, async ({
    page,
    request,
  }) => {
    const native = implementation === "compose";
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await request.post("https://127.0.0.1:4187/__reset");
    await open(page, implementation);
    const composer = page.getByRole("textbox", { name: "メッセージを送信", exact: true });
    const fill = async (input: ReturnType<Page["getByRole"]>, value: string) =>
      native ? fillNative(page, input, value) : input.fill(value);
    await fill(composer, "#gen");
    await page
      .getByRole(native ? "button" : "option", { name: /^#general/ })
      .click({ force: native });
    if (native) await expect(composer).toHaveText("#general ");
    else await expect(composer).toHaveValue("#general ");
    await fill(composer, "/ec");
    await page
      .getByRole(native ? "button" : "option", { name: /^\/echo.*Meeting Notes/ })
      .click({ force: native });
    if (native) await expect(composer).toHaveText("/echo ");
    else await expect(composer).toHaveValue("/echo ");
    await composer.click({ force: native });
    await page.keyboard.press("End");
    if (native)
      await expect.poll(() => nativeInput(page)).toEqual({ value: "/echo ", start: 6, end: 6 });
    if (native)
      await page.evaluate(
        () =>
          new Promise<void>((resolve) =>
            requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
          ),
      );
    await page.keyboard.insertText("hello");
    if (native) await expect(composer).toHaveText("/echo hello");
    else await expect(composer).toHaveValue("/echo hello");
    await page.getByRole("button", { name: "送信", exact: true }).click({ force: native });
    const state = async () => (await request.get("https://127.0.0.1:4187/__state")).json();
    await expect.poll(async () => (await state()).commandCalls[0]?.values?.text).toBe("hello");
    expect((await state()).messages).toHaveLength(2);
    if (native) await expect(composer).toHaveText("");
    else await expect(composer).toHaveValue("");
    await fill(composer, "https://example.test/delayed");
    await page.getByRole("button", { name: "送信", exact: true }).click({ force: native });
    await expect(
      page.getByRole(native ? "button" : "link", { name: /^Parity URL card/ }),
    ).toBeVisible();
    const preview = (await state()).previewRequests[0];
    expect(preview.url).toBe("https://example.test/delayed");
    for (const key of ["authorization", "devUser", "cookie", "referer"])
      expect(preview[key]).toBeUndefined();
    await expect.poll(async () => (await state()).mediaRequests.length).toBeGreaterThan(0);
    for (const key of ["authorization", "devUser", "cookie", "referer"])
      expect((await state()).mediaRequests[0][key]).toBeUndefined();
    await page
      .getByRole(native ? "button" : "link", { name: "Zoo設定", exact: true })
      .click({ force: native });
    if (native) {
      await page.getByRole("button", { name: /^検索エンジン:/ }).click({ force: true });
      await page.getByRole("button", { name: "DuckDuckGo", exact: true }).click({ force: true });
    } else
      await page
        .getByRole("combobox", { name: "検索エンジン", exact: true })
        .selectOption("duckduckgo");
    await expect.poll(async () => (await state()).preferences.search_engine).toBe("duckduckgo");
    await page.getByRole("tab", { name: "メンバー", exact: true }).click({ force: native });
    await expect(
      page.getByRole("button", { name: "管理者を外す", exact: true }).first(),
    ).toBeDisabled();
    await page.getByRole("button", { name: "管理者にする", exact: true }).click({ force: native });
    await expect(
      page.getByText("Bob の権限を「管理者」に変更します。", { exact: true }),
    ).toBeVisible();
    await page
      .getByRole("button", { name: "管理者にする", exact: true })
      .last()
      .click({ force: native });
    await expect
      .poll(async () => (await state()).admins.includes("00000000-0000-0000-0000-000000000002"))
      .toBe(true);
    await expect(page.getByRole("button", { name: "管理者にする", exact: true })).toHaveCount(0);
    await page
      .getByRole("button", { name: "チャンネル一覧へ戻る", exact: true })
      .click({ force: native });
    await page
      .getByRole(native ? "button" : "link", { name: "Bubble Chat", exact: true })
      .click({ force: native });
    const bubble = page.getByRole("textbox", { name: "リアルタイムメッセージ入力", exact: true });
    await fill(bubble, "temporary shared draft");
    await expect
      .poll(async () =>
        Object.values((await state()).bubbles).some(
          (row: any) => row.content === "temporary shared draft",
        ),
      )
      .toBe(true);
    await bubble.click({ force: native });
    await page.keyboard.press("ControlOrMeta+Enter");
    await expect
      .poll(async () => Object.values((await state()).bubbles).every((row: any) => !row.content), {
        timeout: 10000,
      })
      .toBe(true);
    if (native) await expect(bubble).toHaveText("");
    else await expect(bubble).toHaveValue("");
    await fill(bubble, "clear on pagehide");
    await expect
      .poll(async () =>
        Object.values((await state()).bubbles).some(
          (row: any) => row.content === "clear on pagehide",
        ),
      )
      .toBe(true);
    await page.goto("/?frontend=react");
    await expect
      .poll(async () => Object.values((await state()).bubbles).every((row: any) => !row.content), {
        timeout: 10000,
      })
      .toBe(true);
    expect(errors).toEqual([]);
  });
}

for (const implementation of ["react", "compose"] as const) {
  test(`${implementation}: entry points keep the message end visible after delayed URL cards expand`, async ({
    page,
    request,
  }) => {
    const native = implementation === "compose";
    await request.post("https://127.0.0.1:4187/__reset");
    await request.post("https://127.0.0.1:4187/__seed", {
      data: {
        messages: Array.from({ length: 50 }, (_, i) => ({
          id: `scroll-${String(i).padStart(3, "0")}`,
          content: `Scroll row ${i}`,
        })),
      },
    });
    await page.setViewportSize({ width: 1440, height: 600 });
    await open(page, implementation, "/channels/c1", "Scroll row 49");
    const composer = page.getByRole("textbox", { name: "メッセージを送信", exact: true });
    const image = page.waitForResponse((response) => response.url().includes("/__media?"));
    if (native) await fillNative(page, composer, "https://example.test/delayed");
    else await composer.fill("https://example.test/delayed");
    await page.getByRole("button", { name: "送信", exact: true }).click({ force: native });
    await expect(
      page.getByRole(native ? "button" : "link", { name: /^Parity URL card/ }),
    ).toBeVisible();
    await (await image).finished();
    const data = await (await request.get("https://127.0.0.1:4187/__state")).json();
    const id = data.messages.at(-1).id;
    const row = page.locator(native ? `[id="message:${id}"]` : `[data-message-id="${id}"]`);
    await expect
      .poll(async () => {
        const message = await row.boundingBox();
        const input = await composer.boundingBox();
        return message && input ? message.y + message.height - input.y : Infinity;
      })
      .toBeLessThanOrEqual(2);
    await page.screenshot({
      path: test.info().outputPath(`${implementation}-expanded-url-card.png`),
    });
  });
}

for (const implementation of ["react", "compose"] as const) {
  test(`${implementation}: image attachment entry points support picker, clipboard, retry and threads`, async ({
    page,
    request,
  }) => {
    const native = implementation === "compose";
    await request.post("https://127.0.0.1:4187/__reset");
    await open(page, implementation);
    const state = async () => (await request.get("https://127.0.0.1:4187/__state")).json();
    const image = await (await request.get("http://127.0.0.1:4186/icons/icon-192.png")).body();
    const choose = async (name: string, mimeType = "image/png", buffer = image) => {
      const chooser = page.waitForEvent("filechooser");
      await page
        .getByRole("button", { name: "ファイルを添付", exact: true })
        .last()
        .click({ force: native });
      await (await chooser).setFiles({ name, mimeType, buffer });
    };
    await choose("unsupported.exe", "application/x-msdownload");
    await expect(page.getByText("このファイルは添付できません。", { exact: true })).toBeVisible();
    expect((await state()).uploadRequests).toHaveLength(0);
    await choose("empty.txt", "text/plain", Buffer.alloc(0));
    await expect(page.getByText("空のファイルは添付できません。", { exact: true })).toBeVisible();
    expect((await state()).uploadRequests).toHaveLength(0);
    await choose("remove.png");
    await expect.poll(async () => (await state()).uploadRequests.length).toBe(1);
    await page
      .getByRole("button", { name: "remove.pngを削除", exact: true })
      .click({ force: native });
    await expect(page.getByRole("button", { name: "remove.pngを削除", exact: true })).toBeHidden();
    await request.post("https://127.0.0.1:4187/__attachment-failures", { data: { upload: true } });
    await choose("failure.png");
    await expect(
      page.getByText("ファイルをアップロードできませんでした。", { exact: true }),
    ).toBeVisible();
    await expect(page.getByRole("button", { name: "failure.pngを削除", exact: true })).toBeHidden();
    await choose("upload.png");
    await expect.poll(async () => (await state()).uploadRequests.length).toBe(3);
    await expect(page.getByText("ファイルをアップロード中…", { exact: true }).first()).toBeHidden();
    await request.post("https://127.0.0.1:4187/__attachment-failures", { data: { message: true } });
    await page.getByRole("button", { name: "送信", exact: true }).click({ force: native });
    await expect.poll(async () => (await state()).failMessage).toBe(false);
    await expect(page.getByRole("button", { name: "upload.pngを削除", exact: true })).toBeVisible();
    await expect(page.getByRole("button", { name: "送信", exact: true })).toBeEnabled();
    await page.getByRole("button", { name: "送信", exact: true }).click({ force: native });
    const link = page.getByRole("button", { name: "upload.pngをプレビュー", exact: true });
    await expect(link).toBeVisible({ timeout: 30000 });
    await expect.poll(async () => (await state()).attachments.length).toBe(1);
    expect((await state()).messages.at(-1).content).toBe("");
    for (const key of ["authorization", "devUser", "cookie"])
      expect((await state()).uploadRequests.at(-1)[key]).toBeUndefined();
    await page.clock.setFixedTime(new Date("2026-10-31T01:05:00Z"));
    await request.post("https://127.0.0.1:4187/__attachment-failures", { data: { url: true } });
    await link.click({ force: native });
    await expect(page.getByText("読み込めませんでした", { exact: false }).last()).toBeVisible();
    await page.getByRole("button", { name: "再試行", exact: true }).last().click({ force: native });
    await expect(page.getByRole("img", { name: "upload.png", exact: true })).toBeVisible({
      timeout: 30000,
    });
    const downloadEvent = page.waitForEvent("download");
    await page
      .getByRole("button", {
        name: native ? "ダウンロード" : "upload.pngをダウンロード",
        exact: true,
      })
      .click({ force: native });
    const download = await downloadEvent;
    expect(download.suggestedFilename()).toBe("upload.png");
    await expect
      .poll(async () => (await state()).attachmentUrlRequests.length)
      .toBeGreaterThanOrEqual(2);
    await page.getByRole("button", { name: "閉じる", exact: true }).click({ force: native });
    await page
      .getByRole("button", { name: /1 件の返信（スレッドを開く）/ })
      .click({ force: native });
    const composer = page.getByRole("textbox", { name: "返信を送信", exact: true });
    if (native) await fillNative(page, composer, "");
    else await composer.click();
    const paste = async (name: string) => {
      await composer.evaluate(
        (element, { name, bytes }) => {
          const data = new DataTransfer();
          data.items.add(new File([new Uint8Array(bytes)], name, { type: "image/png" }));
          let active = element.ownerDocument.activeElement;
          while (active?.shadowRoot?.activeElement) active = active.shadowRoot.activeElement;
          active!.dispatchEvent(
            new ClipboardEvent("paste", {
              bubbles: true,
              composed: true,
              cancelable: true,
              clipboardData: data,
            }),
          );
        },
        { name, bytes: Array.from(image) },
      );
    };
    await paste("clipboard.png");
    await expect(
      page.getByRole("button", { name: "clipboard.pngを削除", exact: true }),
    ).toBeVisible();
    await choose("second.png");
    await expect(page.getByRole("button", { name: "second.pngを削除", exact: true })).toBeVisible();
    await expect.poll(async () => (await state()).uploadRequests.length).toBe(5);
    await expect(page.getByText("ファイルをアップロード中…", { exact: true }).first()).toBeHidden();
    await page.getByRole("button", { name: "送信", exact: true }).last().click({ force: native });
    await expect(
      page.getByRole("button", { name: "clipboard.pngをプレビュー", exact: true }),
    ).toBeVisible({ timeout: 30000 });
    await expect(
      page.getByRole("button", { name: "second.pngをプレビュー", exact: true }),
    ).toBeVisible();
    const final = await state();
    expect(final.attachments).toHaveLength(3);
    expect(final.messages.at(-1).parent_id).toBe("m1");
    // A real two-frame GIF verifies decoded raster changes in both browser and Canvas previews.
    const gif = Buffer.from(
      "47494638396101000100800000ff00000000ff21ff0b4e45545343415045322e30030100000021f90404050000002c000000000100010000020244010021f90404050000002c00000000010001000002024c01003b",
      "hex",
    );
    await choose("animation.gif", "image/gif", gif);
    await expect.poll(async () => (await state()).uploadRequests.length).toBe(6);
    await expect(page.getByText("ファイルをアップロード中…", { exact: true }).first()).toBeHidden();
    const animation = page.getByRole("img", { name: "animation.gif", exact: true });
    await expect(animation).toBeVisible();
    const firstFrame = await animation.screenshot();
    await expect.poll(async () => firstFrame.equals(await animation.screenshot())).toBe(false);
    await page
      .getByRole("button", { name: "animation.gifを削除", exact: true })
      .click({ force: native });
    await expect(animation).toBeHidden();
    await choose("notes.pdf", "application/octet-stream", Buffer.from("%PDF-1.7\nfixture"));
    await expect.poll(async () => (await state()).uploadRequests.length).toBe(7);
    await expect(page.getByText("ファイルをアップロード中…", { exact: true }).first()).toBeHidden();
    const fileSend = page.getByRole("button", { name: "送信", exact: true }).last();
    await expect(fileSend).toBeVisible();
    await expect(fileSend).toBeEnabled();
    await fileSend.click({ force: native });
    const fileDownload = page.getByRole("button", { name: "notes.pdfをダウンロード", exact: true });
    await expect(fileDownload).toBeVisible();
    const fileEvent = page.waitForEvent("download");
    await fileDownload.click({ force: native });
    expect((await fileEvent).suggestedFilename()).toBe("notes.pdf");
    expect((await state()).attachments.at(-1).content_type).toBe("application/pdf");

    await page.screenshot({
      path: test.info().outputPath(`${implementation}-image-attachments.png`),
    });
  });
}

test("Frontend switch entry points work without an experimental build flag", async ({
  page,
  request,
}) => {
  await request.post("https://127.0.0.1:4187/__reset");
  await open(page, "react");
  await page.getByRole("button", { name: "Compose版を使う（実験・開発中）", exact: true }).click();
  await expect(page.locator("[data-compose-shell]")).toBeVisible({ timeout: 90_000 });
  await expect(page.getByText("こんにちは Parity", { exact: false })).toBeVisible();
  expect(new URL(page.url()).searchParams.get("frontend")).toBe("compose");
  expect(await page.evaluate(() => localStorage.getItem("zoo-frontend"))).toBe("compose");
  await page.goto("/channels/c1");
  await expect(page.locator("[data-compose-shell]")).toBeVisible({ timeout: 90_000 });
  const back = page.getByRole("button", { name: "React版に戻る", exact: true });
  await expect(back).toBeVisible();
  await back.click({ force: true });
  await expect(page.getByRole("textbox", { name: "メッセージを送信", exact: true })).toBeVisible();
  await expect(page.locator("[data-compose-shell]")).toBeHidden();
  expect(new URL(page.url()).searchParams.get("frontend")).toBe("react");
  expect(await page.evaluate(() => localStorage.getItem("zoo-frontend"))).toBe("react");
});

test.describe("missing Compose assets", () => {
  // Route the missing-file response directly; service-worker fetches bypass Playwright routing.
  test.use({ serviceWorkers: "block" });

  test("Frontend switch entry points recover when Compose assets are unavailable", async ({
    page,
    request,
  }) => {
    await request.post("https://127.0.0.1:4187/__reset");
    await page.route("**/compose/zoo-compose.js", (route) =>
      route.fulfill({ status: 404, body: "missing" }),
    );
    await open(page, "react");
    await page
      .getByRole("button", { name: "Compose版を使う（実験・開発中）", exact: true })
      .click();
    await expect(page.locator("[data-compose-shell]")).toBeVisible({ timeout: 90_000 });
    await expect(page.getByRole("alert")).toContainText("Compose版を起動できませんでした。");
    await page.getByRole("button", { name: "React版に戻る", exact: true }).click();
    await expect(
      page.getByRole("textbox", { name: "メッセージを送信", exact: true }),
    ).toBeVisible();
    expect(await page.evaluate(() => localStorage.getItem("zoo-frontend"))).toBe("react");
  });
});
