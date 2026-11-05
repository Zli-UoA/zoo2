import { clickControl, fillControl } from "./compose-control-input";
import { composeVisualColorThreshold } from "./compose-visual";
import { test, expect, type Page } from "@playwright/test";
import { mkdir, writeFile } from "node:fs/promises";
import { dirname } from "node:path";

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
  const target = new URL(path, "http://compose.invalid");
  target.searchParams.set("frontend", implementation);
  await page.goto(target.pathname + target.search + target.hash);
  if (readyText)
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
  const settings = page.getByRole("dialog", { name: "チャンネル設定", exact: true });
  if (!(await settings.isVisible())) {
    const trigger = page.getByRole("button", { name: "チャンネル設定", exact: true });
    await expect(trigger).toBeVisible();
    await clickControl(trigger, native ? "kotlin" : "react");
  }
  await expect(settings).toBeVisible();
  const button = settings.getByRole("button", {
    name: action === "edit" ? "編集" : "開く",
    exact: true,
  });
  await expect(button).toBeVisible();
  await clickControl(button, native ? "kotlin" : "react");
}
async function closeChannelSettings(page: Page, _native: boolean) {
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog", { name: "Webhook設定", exact: true })).toBeHidden();
  if (await page.getByRole("dialog", { name: "チャンネル設定", exact: true }).isVisible())
    await page.keyboard.press("Escape");
}
async function openApps(page: Page, native: boolean) {
  await page.getByRole("link", { name: "Zoo設定", exact: true }).click({ force: native });
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
  await fillControl(page, locator, value, "kotlin");
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
      .getByRole("menuitem", { name: "編集", exact: true })
      .click({ force: implementation === "compose" });
    await fill(
      page.getByRole("textbox", { name: "メッセージを編集", exact: true }),
      "編集した本文",
    );
    // 編集欄は Cmd/Ctrl+Enter で保存できる（macOS は Cmd、それ以外は Ctrl）。
    await page.keyboard.press("ControlOrMeta+Enter");
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
    const notifications = page.getByRole("link", { name: /^未読通知 /, exact: true });
    await expect(notifications).toBeVisible();
    await clickControl(notifications, implementation === "compose" ? "kotlin" : "react");
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
    const click = async (name: string) => {
      const button = page.getByRole("button", { name, exact: true });
      await expect(button).toBeVisible();
      await clickControl(button, native ? "kotlin" : "react");
    };
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
    await page.getByRole("link", { name: "native-parity", exact: true }).click({ force: native });
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
    // The API acknowledgement precedes the Electric update and header rerender.
    await expect(page.getByRole("heading", { name: "renamed-parity", exact: true })).toBeVisible();
    await channelAction(page, native, "webhook");
    await fill("Webhook名", "Parity hook");
    await click("作成");
    const webhookName = page.getByRole("textbox", { name: "Webhook名", exact: true });
    if (native) await expect(webhookName).toHaveText("Webhook");
    else await expect(webhookName).toHaveValue("Webhook");
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
  await expect(page.getByRole("textbox", { name: "メッセージを送信", exact: true })).toBeVisible({
    timeout: 90_000,
  });
  await expect(page.getByText("こんにちは Parity", { exact: false })).toBeVisible();
  expect(errors).toEqual([]);
});

test("Compose panes preserve the existing desktop and mobile UI", async ({ browser, request }) => {
  for (const viewport of [
    { width: 1440, height: 900 },
    { width: 390, height: 844 },
  ]) {
    const images: Buffer[] = [];
    const areas: Record<string, { x: number; y: number; width: number; height: number }>[] = [];
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
      await page.mouse.move(-100, -100);
      const layout: Record<string, { x: number; y: number; width: number; height: number }> = {};
      for (const [name, element] of [
        ["navigation", page.getByRole("navigation", { name: "メインナビゲーション", exact: true })],
        ["main", page.getByRole("main")],
      ] as const) {
        await expect(element).toBeVisible();
        const bounds = await element.boundingBox();
        expect(bounds, `${implementation} ${name} has visible bounds`).not.toBeNull();
        layout[name] = bounds!;
      }
      areas.push(layout);
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
    // Compare the actual shell geometry; font AA differences are reviewed in the paired captures.
    for (const name of ["navigation", "main"]) {
      for (const dimension of ["x", "y", "width", "height"] as const)
        expect(
          Math.abs(areas[0][name][dimension] - areas[1][name][dimension]),
          `${name}.${dimension} at viewport ${viewport.width}`,
        ).toBeLessThanOrEqual(2);
    }
    if (process.env.STORYBOOK_PARITY_VISUAL === "strict") {
      const snapshot = `panes-${viewport.width}.png`;
      const referencePath = test.info().snapshotPath(snapshot);
      await mkdir(dirname(referencePath), { recursive: true });
      await writeFile(referencePath, images[0]);
      expect(images[1]).toMatchSnapshot(snapshot, {
        threshold: composeVisualColorThreshold,
        maxDiffPixels: 0,
      });
    }
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
  await expect(page.getByRole("link", { name: "general", exact: true })).toBeVisible();
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
    await page.getByRole("option", { name: /^#general/ }).click({ force: native });
    if (native) await expect(composer).toHaveText("#general ");
    else await expect(composer).toHaveValue("#general ");
    await fill(composer, "/ec");
    await page.getByRole("option", { name: /^\/echo.*Meeting Notes/ }).click({ force: native });
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
    await expect(page.getByRole("link", { name: /^Parity URL card/ })).toBeVisible();
    const preview = (await state()).previewRequests[0];
    expect(preview.url).toBe("https://example.test/delayed");
    for (const key of ["authorization", "devUser", "cookie", "referer"])
      expect(preview[key]).toBeUndefined();
    await expect.poll(async () => (await state()).mediaRequests.length).toBeGreaterThan(0);
    for (const key of ["authorization", "devUser", "cookie", "referer"])
      expect((await state()).mediaRequests[0][key]).toBeUndefined();
    await page.getByRole("link", { name: "Zoo設定", exact: true }).click({ force: native });
    if (native) {
      await page
        .getByRole("combobox", { name: "検索エンジン", exact: true })
        .click({ force: true });
      await page.getByRole("option", { name: "DuckDuckGo", exact: true }).click({ force: true });
    } else
      await page
        .getByRole("combobox", { name: "検索エンジン", exact: true })
        .selectOption("duckduckgo");
    await expect.poll(async () => (await state()).preferences.search_engine).toBe("duckduckgo");
    const membersTab = page.getByRole("tab", { name: "メンバー", exact: true });
    await expect(membersTab).toBeVisible();
    // Preference updates can replace the native tab projection during a click.
    // Selecting the same tab is idempotent; wait for the committed selection.
    await expect(async () => {
      await clickControl(membersTab, native ? "kotlin" : "react");
      await expect(membersTab).toHaveAttribute("aria-selected", "true", { timeout: 1000 });
    }).toPass({ timeout: 10_000, intervals: [100, 250, 500] });
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
    await page.getByRole("link", { name: "Bubble Chat", exact: true }).click({ force: native });
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
    await expect(page.getByRole("link", { name: /^Parity URL card/ })).toBeVisible();
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
    await expect(async () => {
      const send = page.getByRole("button", { name: "送信", exact: true });
      await expect(send).toBeVisible({ timeout: 1000 });
      await send.click({ force: native, timeout: 1000 });
    }).toPass({ timeout: 5000 });
    await expect.poll(async () => (await state()).failMessage).toBe(false);
    await expect(page.getByRole("button", { name: "upload.pngを削除", exact: true })).toBeVisible();
    await expect(page.getByRole("button", { name: "送信", exact: true })).toBeEnabled();
    await expect(async () => {
      const send = page.getByRole("button", { name: "送信", exact: true });
      await expect(send).toBeVisible({ timeout: 1000 });
      await send.click({ force: native, timeout: 1000 });
    }).toPass({ timeout: 5000 });
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
        name: "upload.pngをダウンロード",
        exact: true,
      })
      .click({ force: native });
    const download = await downloadEvent;
    expect(download.suggestedFilename()).toBe("upload.png");
    await expect
      .poll(async () => (await state()).attachmentUrlRequests.length)
      .toBeGreaterThanOrEqual(2);
    await page.getByRole("button", { name: "閉じる", exact: true }).click({ force: native });
    if (native)
      await expect(page.getByRole("img", { name: "upload.png", exact: true })).toBeHidden();
    else await expect(page.getByRole("dialog")).toBeHidden();
    const replyButton = page.getByRole("button", { name: /1 件の返信（スレッドを開く）/ });
    await expect(replyButton).toBeVisible();
    await replyButton.click({ force: native });
    const composer = page.getByRole("textbox", { name: "返信を送信", exact: true });
    await expect(composer).toBeVisible();
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
    // Avoid sampling the same phase of this GIF's 50ms frames / 100ms loop.
    await expect
      .poll(async () => firstFrame.equals(await animation.screenshot()), {
        intervals: [17, 29, 43, 71, 113],
      })
      .toBe(false);
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
    // Upload completion can replace Compose's accessibility projection. Settle its
    // target before the forced canvas click, which skips Playwright's stability check.
    await fileSend.scrollIntoViewIfNeeded();
    await expect(fileSend).toBeInViewport({ ratio: 1 });
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
  await expect(page.getByText("こんにちは Parity", { exact: false })).toBeVisible({
    timeout: 90_000,
  });
  expect(new URL(page.url()).searchParams.get("frontend")).toBe("compose");
  expect(await page.evaluate(() => localStorage.getItem("zoo-frontend"))).toBe("compose");
  await page.goto("/channels/c1");
  await expect(page.locator("[data-compose-shell]")).toBeVisible({ timeout: 90_000 });
  // The loading screen has the same label and disappears when Compose mounts.
  // Wait for the native switch so this exercises the loaded shell's return path.
  const back = page
    .getByRole("navigation", { name: "メインナビゲーション", exact: true })
    .getByRole("button", { name: "React版に戻る", exact: true });
  await expect(back).toBeVisible({ timeout: 90_000 });
  await clickControl(back, "kotlin");
  await expect(page.getByRole("textbox", { name: "メッセージを送信", exact: true })).toBeVisible({
    timeout: 90_000,
  });
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
    await expect(page.getByRole("textbox", { name: "メッセージを送信", exact: true })).toBeVisible({
      timeout: 90_000,
    });
    expect(await page.evaluate(() => localStorage.getItem("zoo-frontend"))).toBe("react");
  });
});

for (const implementation of ["react", "compose"] as const) {
  test(`${implementation}: mobile members direct URL, close and history preserve channel context`, async ({
    page,
    request,
  }) => {
    await request.post("https://127.0.0.1:4187/__reset");
    await page.setViewportSize({ width: 390, height: 844 });
    await open(page, implementation, "/channels/c1?members=true&message=m1", "");
    const native = implementation === "compose";
    const members = page.getByRole("heading", { name: "メンバー", exact: true, level: 1 });
    await expect(members).toBeVisible({ timeout: 90_000 });
    const initialIndex = await page.evaluate(() => window.history.state?.__TSR_index);
    await page
      .getByRole("button", { name: "チャンネルに戻る", exact: true })
      .click({ force: native });
    await expect(members).toBeHidden();
    await expect.poll(() => new URL(page.url()).searchParams.get("members")).toBeNull();
    expect(new URL(page.url()).searchParams.get("message")).toBe("m1");
    expect(await page.evaluate(() => window.history.state?.__TSR_index)).toBe(initialIndex);
    await page.getByRole("button", { name: /^メンバー / }).click({ force: native });
    await expect(members).toBeVisible();
    expect(new URL(page.url()).searchParams.get("members")).toBe("true");
    expect(await page.evaluate(() => window.history.state?.zooMembersParent)).toBe("c1");
    await page
      .getByRole("button", { name: "チャンネルに戻る", exact: true })
      .click({ force: native });
    await expect(members).toBeHidden();
    expect(await page.evaluate(() => window.history.state?.__TSR_index)).toBe(initialIndex);
    await page.goForward();
    await expect(members).toBeVisible();
    await page.goBack();
    await expect(members).toBeHidden();
    await expect(page.getByRole("heading", { name: "general", exact: true })).toBeVisible();
    expect(new URL(page.url()).searchParams.get("message")).toBe("m1");
  });

  test(`${implementation}: desktop members stay local and close when layout changes`, async ({
    page,
    request,
  }) => {
    await request.post("https://127.0.0.1:4187/__reset");
    await page.setViewportSize({ width: 1440, height: 900 });
    await open(page, implementation, "/channels/c1?members=true&message=m1");
    const native = implementation === "compose";
    await expect.poll(() => new URL(page.url()).searchParams.get("members")).toBeNull();
    expect(new URL(page.url()).searchParams.get("message")).toBe("m1");
    const initialIndex = await page.evaluate(() => window.history.state?.__TSR_index);
    const members = page.getByRole("heading", { name: "メンバー", exact: true, level: 3 });
    await expect(members).toBeHidden();
    const toggle = page.getByRole("button", { name: /^メンバー / });
    await toggle.click({ force: native });
    await expect(members).toBeVisible();
    expect(new URL(page.url()).searchParams.get("members")).toBeNull();
    expect(await page.evaluate(() => window.history.state?.__TSR_index)).toBe(initialIndex);
    await toggle.click({ force: native });
    await expect(members).toBeHidden();
    await toggle.click({ force: native });
    await expect(members).toBeVisible();
    await page.setViewportSize({ width: 390, height: 844 });
    await expect(members).toBeHidden();
    await expect(page.getByRole("heading", { name: "メンバー", level: 1 })).toBeHidden();
    await expect(page.getByRole("heading", { name: "general", exact: true })).toBeVisible();
    expect(new URL(page.url()).searchParams.get("members")).toBeNull();
  });
}

for (const implementation of ["react", "compose"] as const) {
  test(`${implementation}: nested thread roots remain visible and accept replies`, async ({
    page,
    request,
  }) => {
    await request.post("https://127.0.0.1:4187/__reset");
    await open(page, implementation, "/channels/c1?thread=m2");
    await expect(page.getByText("返信の本文", { exact: true })).toBeVisible();
    const editor = page.getByRole("textbox", { name: "返信を送信", exact: true });
    await expect(editor).toBeVisible();
    if (implementation === "compose") await fillNative(page, editor, "入れ子の返信");
    else await editor.fill("入れ子の返信");
    await expect(async () => {
      const send = page.getByRole("button", { name: "送信", exact: true }).last();
      await expect(send).toBeVisible({ timeout: 1000 });
      await send.click({ force: implementation === "compose", timeout: 1000 });
    }).toPass({ timeout: 5000 });
    await expect(page.getByText("入れ子の返信", { exact: true })).toBeVisible();
    const state = await (await request.get("https://127.0.0.1:4187/__state")).json();
    expect(state.messages.at(-1)).toMatchObject({ parent_id: "m2", content: "入れ子の返信" });
    await page.reload();
    await expect(page.getByRole("heading", { name: "general", exact: true })).toBeVisible({
      timeout: 90000,
    });
    await expect(page.getByText("返信の本文", { exact: true })).toBeVisible();
    await expect(page.getByText("入れ子の返信", { exact: true })).toBeVisible();
  });
}

for (const implementation of ["react", "compose"] as const) {
  test(`${implementation}: joined display keeps pending reactions and reply counts visible until rollback`, async ({
    page,
    request,
  }) => {
    const native = implementation === "compose";
    await request.post("https://127.0.0.1:4187/__reset");
    await request.post("https://127.0.0.1:4187/__seed", {
      data: {
        messages: [
          {
            id: "optimistic-display-root",
            content: "Joined optimistic display",
            created_at: "2026-10-31 00:00:00.000",
            display_data: {
              author: {
                id: "00000000-0000-0000-0000-000000000001",
                user_name: "alice",
                display_name: "Alice",
              },
              emojis: {},
              attachments: [],
              reply_count: 1,
              reactions: [
                {
                  id: "peer-reaction",
                  message_id: "optimistic-display-root",
                  user_id: "00000000-0000-0000-0000-000000000002",
                  emoji: "👍",
                  created_at: "2026-10-30 09:00:00.000",
                  user_name: "bob",
                  display_name: "Bob",
                },
              ],
            },
          },
        ],
      },
    });
    await open(page, implementation, "/channels/c1", "Joined optimistic display");
    const one = page.getByRole("button", { name: "👍 1 件", exact: true });
    await expect(one).toBeVisible();
    let releaseReaction!: () => void;
    const reactionPending = new Promise<void>((resolve) => {
      releaseReaction = resolve;
    });
    await page.route("**/api/messages/optimistic-display-root/reactions", async (route) => {
      if (route.request().method() !== "POST") return route.continue();
      await reactionPending;
      await route.fulfill({
        status: 500,
        contentType: "application/json",
        headers: { "access-control-allow-origin": "*" },
        body: JSON.stringify({ error: "Rejected for rollback test" }),
      });
    });
    try {
      await clickControl(one, native ? "kotlin" : "react");
      await expect(
        page.getByRole("button", { name: "👍 2 件（自分もリアクション済み）", exact: true }),
      ).toBeVisible();
    } finally {
      releaseReaction();
    }
    await expect(one).toBeVisible();
    if (native) {
      const error = page.getByText(
        "サーバーに接続できませんでした。接続を確認してもう一度お試しください。",
        { exact: true },
      );
      await expect(error).toBeVisible();
      // Toasts intentionally pause on hover; keep the pointer away so the error
      // can expire before clicking the reply composer underneath it.
      await page.mouse.move(0, 0);
      await expect(error).toBeHidden({ timeout: 10_000 });
    }
    const root = page.locator(
      native
        ? '[id="message:optimistic-display-root"]'
        : '[data-message-id="optimistic-display-root"]',
    );
    await clickControl(
      root.getByRole("button", { name: "1 件の返信（スレッドを開く）", exact: true }),
      native ? "kotlin" : "react",
    );
    const reply = page.getByRole("textbox", { name: "返信を送信", exact: true });
    await fillControl(page, reply, "Pending joined reply", native ? "kotlin" : "react");
    let releaseReply!: () => void;
    const replyPending = new Promise<void>((resolve) => {
      releaseReply = resolve;
    });
    await page.route("**/api/messages", async (route) => {
      if (route.request().method() !== "POST") return route.continue();
      await replyPending;
      await route.fulfill({
        status: 500,
        contentType: "application/json",
        headers: { "access-control-allow-origin": "*" },
        body: JSON.stringify({ error: "Rejected for rollback test" }),
      });
    });
    try {
      await clickControl(
        page.getByRole("button", { name: "送信", exact: true }).last(),
        native ? "kotlin" : "react",
      );
      await expect(
        root.getByRole("button", { name: "2 件の返信（スレッドを開く）", exact: true }),
      ).toBeVisible();
    } finally {
      releaseReply();
    }
    await expect(
      root.getByRole("button", { name: "1 件の返信（スレッドを開く）", exact: true }),
    ).toBeVisible();
    if (native) await expect(reply).toHaveText("Pending joined reply");
    else await expect(reply).toHaveValue("Pending joined reply");
  });
}

for (const implementation of ["react", "compose"] as const) {
  test(`${implementation}: pending custom reaction keeps picker image dimensions before sync`, async ({
    page,
    request,
  }) => {
    const native = implementation === "compose";
    await request.post("https://127.0.0.1:4187/__reset");
    await request.post("https://127.0.0.1:4187/__seed", {
      data: {
        messages: [
          {
            id: "custom-pending-root",
            content: "Pending custom reaction",
            display_data: {
              author: {
                id: "00000000-0000-0000-0000-000000000001",
                user_name: "alice",
                display_name: "Alice",
              },
              emojis: {},
              attachments: [],
              reactions: [],
              reply_count: 0,
            },
          },
        ],
      },
    });
    await page.route("**/api/emojis", (route) =>
      route.fulfill({
        status: 200,
        contentType: "application/json",
        headers: { "access-control-allow-origin": "*" },
        body: JSON.stringify([
          {
            name: "wide",
            url: "/api/emoji-images/wide",
            width: 120,
            height: 20,
            aliases: [],
            category: "Test",
          },
        ]),
      }),
    );
    await page.route("**/api/emoji-images/wide", (route) =>
      route.fulfill({
        status: 200,
        contentType: "image/png",
        headers: { "access-control-allow-origin": "*" },
        body: Buffer.from(
          "iVBORw0KGgoAAAANSUhEUgAAAAMAAAABCAYAAAAb4BS0AAAADklEQVR4nGP438DwH4YBMWYHe7X3S+QAAAAASUVORK5CYII=",
          "base64",
        ),
      }),
    );
    await open(page, implementation, "/channels/c1", "Pending custom reaction");
    const root = page.locator(
      native ? '[id="message:custom-pending-root"]' : '[data-message-id="custom-pending-root"]',
    );
    if (!native) await root.hover();
    await clickControl(
      root.getByRole("button", { name: "リアクションを追加", exact: true }).first(),
      native ? "kotlin" : "react",
    );
    await clickControl(
      page.getByRole("tab", { name: "カスタム絵文字", exact: true }),
      native ? "kotlin" : "react",
    );
    const choice = page.getByRole("button", {
      name: native ? /^:wide:(?:, :wide:)?$/ : ":wide:",
      exact: true,
    });
    await expect(choice).toBeVisible();
    // The picker has fetched metadata. The pending chip must reuse it even if
    // any subsequent registry request fails while the mutation is held.
    await page.route("**/api/emojis", (route) => route.abort());
    let release!: () => void;
    const pending = new Promise<void>((resolve) => {
      release = resolve;
    });
    await page.route("**/api/messages/custom-pending-root/reactions", async (route) => {
      await pending;
      await route.fulfill({
        status: 500,
        contentType: "application/json",
        headers: { "access-control-allow-origin": "*" },
        body: '{"error":"rollback"}',
      });
    });
    try {
      await clickControl(choice, native ? "kotlin" : "react");
      const chip = root.getByRole("button", {
        name: /^:wide: 1 件（自分もリアクション済み）/,
      });
      await expect(chip).toBeVisible();
      if (native) {
        // Compose merges the image's semantics into its clickable parent. Its
        // actual Canvas bounds must reserve the six-to-one image, not text.
        await expect
          .poll(async () => (await chip.boundingBox())?.width ?? 0)
          .toBeGreaterThanOrEqual(120);
      } else {
        const image = chip.getByRole("img", { name: ":wide:", exact: true });
        await expect(image).toBeVisible();
        await expect(image).toHaveCSS("width", "120px");
        await expect(image).toHaveCSS("height", "20px");
      }
    } finally {
      release();
    }
    await expect(
      root.getByRole("button", { name: /^:wide: 1 件（自分もリアクション済み）/ }),
    ).toBeHidden();
  });
}

test("Compose cached rich text follows live system theme changes", async ({ page, request }) => {
  await request.post("https://127.0.0.1:4187/__reset");
  await request.post("https://127.0.0.1:4187/__seed", {
    data: {
      messages: [
        {
          id: "memo-theme",
          content: "Cache rendering #general",
          created_at: "2026-11-01T00:00:00Z",
        },
      ],
    },
  });
  await page.emulateMedia({ colorScheme: "light" });
  await page.addInitScript(() => {
    localStorage.setItem("zoo-locale", "ja");
    localStorage.setItem("zoo-theme", "system");
  });
  await page.goto("/channels/c1?frontend=compose");
  const row = page.locator('[id="message:memo-theme"]');
  await expect(row).toBeVisible({ timeout: 90_000 });
  const composer = page.getByRole("textbox", { name: "メッセージを送信", exact: true });
  await fillNative(page, composer, "kept while changing theme");
  const send = page.getByRole("button", { name: "送信", exact: true });
  await expect(send).toBeEnabled();

  // Read the actual Canvas pixels; DOM/CSS theme checks cannot detect stale
  // AnnotatedString colors retained by an incomplete remember key.
  async function linkPixels(expected: number[]) {
    const png = (await row.screenshot()).toString("base64");
    return page.evaluate(
      async ({ png, expected }) => {
        const bytes = Uint8Array.from(atob(png), (character) => character.charCodeAt(0));
        const bitmap = await createImageBitmap(new Blob([bytes], { type: "image/png" }));
        const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
        const context = canvas.getContext("2d")!;
        context.drawImage(bitmap, 0, 0);
        bitmap.close();
        const { data } = context.getImageData(0, 0, canvas.width, canvas.height);
        let count = 0;
        for (let pixel = 0; pixel < data.length; pixel += 4) {
          if (expected.every((channel, index) => Math.abs(data[pixel + index] - channel) <= 15))
            count++;
        }
        return count;
      },
      { png, expected },
    );
  }
  await expect.poll(() => linkPixels([29, 78, 216]), { timeout: 10_000 }).toBeGreaterThan(5);
  await page.emulateMedia({ colorScheme: "dark" });
  await expect.poll(() => linkPixels([138, 180, 255]), { timeout: 10_000 }).toBeGreaterThan(5);
  await expect(composer).toHaveText("kept while changing theme");
  await expect(send).toBeEnabled();
  await page.emulateMedia({ colorScheme: "light" });
  await expect.poll(() => linkPixels([29, 78, 216]), { timeout: 10_000 }).toBeGreaterThan(5);
});

test("compose: channel notes open from a direct URL", async ({ page, request }) => {
  await request.post("https://127.0.0.1:4187/__reset");
  let note = {
    id: "note-1",
    channel_id: "c1",
    title: "Release notes",
    content: "# Hello",
    version: 1,
  };
  let update: { title: string; content: string; version: number } | undefined;
  await page.route("**/api/channels/c1/notes**", async (route) => {
    const url = new URL(route.request().url());
    if (route.request().method() === "PATCH") {
      update = route.request().postDataJSON();
      note = { ...note, ...update!, version: note.version + 1 };
    }
    await route.fulfill({
      headers: { "Access-Control-Allow-Origin": "*" },
      json: { data: url.pathname.endsWith("/note-1") ? note : [note], txid: "1" },
    });
  });
  await open(page, "compose", "/channels/c1?note=note-1", "");
  await expect(page.getByText("Release notes", { exact: true }).first()).toBeVisible();
  await expect(page.getByText("Hello", { exact: true }).first()).toBeVisible();
  await page.getByRole("button", { name: "編集", exact: true }).click({ force: true });
  await fillControl(
    page,
    page.getByRole("textbox", { name: "本文", exact: true }),
    "# Updated",
    "kotlin",
  );
  await page.getByRole("button", { name: "ノートを保存", exact: true }).click({ force: true });
  await expect(page.getByText("Updated", { exact: true }).first()).toBeVisible();
  expect(update).toEqual({ title: "Release notes", content: "# Updated", version: 1 });
});

test("compose: slow note list is not cancelled by polling", async ({ page, request }) => {
  await request.post("https://127.0.0.1:4187/__reset");
  let lists = 0;
  const note = {
    id: "slow-note",
    channel_id: "c1",
    title: "Slow note",
    content: "Slow content",
    version: 1,
  };
  await page.route("**/api/channels/c1/notes**", async (route) => {
    if (new URL(route.request().url()).pathname.endsWith("/notes")) {
      lists++;
      await new Promise((resolve) => setTimeout(resolve, 6500));
    }
    await route.fulfill({
      headers: { "Access-Control-Allow-Origin": "*" },
      json: {
        data: new URL(route.request().url()).pathname.endsWith("/notes") ? [note] : note,
        txid: "1",
      },
    });
  });
  await open(page, "compose", "/channels/c1?note=slow-note", "");
  await expect(page.getByText("Slow content", { exact: true })).toBeVisible({ timeout: 15000 });
  expect(lists).toBe(1);
});

test("compose: slow note refresh does not replace an editor draft", async ({ page, request }) => {
  await request.post("https://127.0.0.1:4187/__reset");
  let detailRequests = 0;
  let refreshDone = false;
  const note = {
    id: "draft-note",
    channel_id: "c1",
    title: "Draft note",
    content: "Original",
    version: 1,
  };
  await page.route("**/api/channels/c1/notes**", async (route) => {
    const isList = new URL(route.request().url()).pathname.endsWith("/notes");
    if (!isList && ++detailRequests === 2) {
      await new Promise((resolve) => setTimeout(resolve, 3000));
      refreshDone = true;
    }
    await route.fulfill({
      headers: { "Access-Control-Allow-Origin": "*" },
      json: { data: isList ? [note] : note, txid: "1" },
    });
  });
  await open(page, "compose", "/channels/c1?note=draft-note", "");
  await expect(page.getByText("Original", { exact: true })).toBeVisible();
  await expect.poll(() => detailRequests, { timeout: 10000 }).toBeGreaterThanOrEqual(2);
  await page.getByRole("button", { name: "編集", exact: true }).click({ force: true });
  const field = page.getByRole("textbox", { name: "本文", exact: true });
  await fillControl(page, field, "Draft in progress", "kotlin");
  await expect.poll(() => refreshDone, { timeout: 10000 }).toBe(true);
  await expect(field).toHaveText("Draft in progress");
});

test("compose: unsaved note draft survives tab and route cancellation", async ({
  page,
  request,
}) => {
  await request.post("https://127.0.0.1:4187/__reset");
  const note = {
    id: "draft-note",
    channel_id: "c1",
    title: "Draft note",
    content: "Original",
    version: 1,
  };
  await page.route("**/api/channels/c1/notes**", async (route) => {
    const isList = new URL(route.request().url()).pathname.endsWith("/notes");
    await route.fulfill({
      headers: { "Access-Control-Allow-Origin": "*" },
      json: { data: isList ? [note] : note, txid: "1" },
    });
  });
  await open(page, "compose", "/channels/c1?note=draft-note", "");
  await expect(page.getByText("Original", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "編集", exact: true }).click({ force: true });
  const field = page.getByRole("textbox", { name: "本文", exact: true });
  await fillControl(page, field, "Unsaved draft", "kotlin");
  await page.getByRole("tab", { name: "メッセージ", exact: true }).click({ force: true });
  await expect(page.getByText("変更が保存されていません。この画面を離れますか？")).toBeVisible();
  await page.getByRole("button", { name: "キャンセル", exact: true }).last().click({ force: true });
  await expect(field).toHaveText("Unsaved draft");
  const dialog = page.waitForEvent("dialog").then((event) => event.dismiss());
  await Promise.all([dialog, page.getByRole("link", { name: "Home", exact: true }).press("Enter")]);
  await expect(page).toHaveURL(/note=draft-note/);
  await expect(field).toHaveText("Unsaved draft");
});

test("compose: removed note is no longer displayed after refresh", async ({ page, request }) => {
  await request.post("https://127.0.0.1:4187/__reset");
  let removed = false;
  const note = {
    id: "gone-note",
    channel_id: "c1",
    title: "Gone note",
    content: "Sensitive body",
    version: 1,
  };
  await page.route("**/api/channels/c1/notes**", async (route) => {
    const isList = new URL(route.request().url()).pathname.endsWith("/notes");
    await route.fulfill({
      headers: { "Access-Control-Allow-Origin": "*" },
      json: { data: isList ? (removed ? [] : [note]) : note, txid: "1" },
    });
  });
  await open(page, "compose", "/channels/c1?note=gone-note", "");
  await expect(page.getByText("Sensitive body", { exact: true })).toBeVisible();
  removed = true;
  await expect(page.getByText("ノートが見つかりません。").first()).toBeVisible({ timeout: 15000 });
  await expect(page.getByText("Sensitive body", { exact: true })).toBeHidden();
  await expect(page.getByRole("button", { name: "編集", exact: true })).toBeHidden();
});

test("compose: todo shows saving until its patch finishes", async ({ page, request }) => {
  await request.post("https://127.0.0.1:4187/__reset");
  let note = {
    id: "task-note",
    channel_id: "c1",
    title: "Task note",
    content: "- [ ] Do thing",
    version: 1,
  };
  await page.route("**/api/channels/c1/notes**", async (route) => {
    const isList = new URL(route.request().url()).pathname.endsWith("/notes");
    if (route.request().method() === "PATCH") {
      await new Promise((resolve) => setTimeout(resolve, 2000));
      note = { ...note, ...route.request().postDataJSON(), version: 2 };
    }
    await route.fulfill({
      headers: { "Access-Control-Allow-Origin": "*" },
      json: { data: isList ? [note] : note, txid: "1" },
    });
  });
  await open(page, "compose", "/channels/c1?note=task-note", "");
  const checkbox = page.getByRole("checkbox").first();
  await expect(checkbox).toBeVisible();
  await checkbox.click({ force: true });
  await expect(page.getByText("保存中…", { exact: true })).toBeVisible();
  await expect(page.getByText("未保存", { exact: true })).toBeHidden();
  await expect(page.getByText("保存中…", { exact: true })).toBeHidden({ timeout: 10000 });
  expect(note.content).toContain("[x]");
});

test("compose: desktop tab drag sends the shared order", async ({ page, request }) => {
  await request.post("https://127.0.0.1:4187/__reset");
  const notes = [
    { id: "note-a", channel_id: "c1", title: "First note", content: "First body", version: 1 },
    { id: "note-b", channel_id: "c1", title: "Second note", content: "Second body", version: 1 },
  ];
  let order: { note_ids: string[]; expected_note_ids: string[] } | undefined;
  await page.route("**/api/channels/c1/notes**", async (route) => {
    const path = new URL(route.request().url()).pathname;
    if (path.endsWith("/order")) order = route.request().postDataJSON();
    const data = path.endsWith("/notes")
      ? notes
      : path.endsWith("/order")
        ? []
        : notes.find((note) => path.endsWith(note.id));
    await route.fulfill({
      headers: { "Access-Control-Allow-Origin": "*" },
      json: { data, txid: "1" },
    });
  });
  await open(page, "compose", "/channels/c1?note=note-a", "");
  const first = page.getByRole("tab", { name: "First note", exact: true });
  const second = page.getByRole("tab", { name: "Second note", exact: true });
  await expect(first).toBeVisible();
  const from = (await first.boundingBox())!;
  const to = (await second.boundingBox())!;
  await page.mouse.move(from.x + from.width / 2, from.y + from.height / 2);
  await page.mouse.down();
  await page.mouse.move(to.x + to.width * 0.9, to.y + to.height / 2, { steps: 12 });
  await page.mouse.up();
  await expect
    .poll(() => order)
    .toEqual({ note_ids: ["note-b", "note-a"], expected_note_ids: ["note-a", "note-b"] });
});

test("compose: note conflict keeps draft until latest is reviewed", async ({ page, request }) => {
  await request.post("https://127.0.0.1:4187/__reset");
  let note = {
    id: "conflict-note",
    channel_id: "c1",
    title: "Conflict note",
    content: "Original",
    version: 1,
  };
  let conflict = true;
  let saved: { title: string; content: string; version: number } | undefined;
  await page.route("**/api/channels/c1/notes**", async (route) => {
    const isList = new URL(route.request().url()).pathname.endsWith("/notes");
    if (route.request().method() === "PATCH") {
      if (conflict) {
        conflict = false;
        note = { ...note, content: "Remote update", version: 2 };
        await route.fulfill({
          status: 409,
          headers: { "Access-Control-Allow-Origin": "*" },
          json: { error: "conflict" },
        });
        return;
      }
      saved = route.request().postDataJSON();
      note = { ...note, ...saved!, version: 3 };
    }
    await route.fulfill({
      headers: { "Access-Control-Allow-Origin": "*" },
      json: { data: isList ? [note] : note, txid: "1" },
    });
  });
  await open(page, "compose", "/channels/c1?note=conflict-note", "");
  await expect(page.getByText("Original", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "編集", exact: true }).click({ force: true });
  const field = page.getByRole("textbox", { name: "本文", exact: true });
  await fillControl(page, field, "My draft", "kotlin");
  await page.getByRole("button", { name: "ノートを保存", exact: true }).click({ force: true });
  await expect(field).toHaveText("My draft");
  await expect(page.getByText("Remote update", { exact: true })).toBeVisible();
  await page
    .getByRole("button", { name: "最新版を確認して下書きで上書きする", exact: true })
    .click({ force: true });
  await expect(page.getByText("My draft", { exact: true })).toBeVisible();
  expect(saved).toEqual({ title: "Conflict note", content: "My draft", version: 2 });
});

test("compose: retry loads latest after conflict fetch fails", async ({ page, request }) => {
  await request.post("https://127.0.0.1:4187/__reset");
  let note = {
    id: "retry-note",
    channel_id: "c1",
    title: "Retry note",
    content: "Original",
    version: 1,
  };
  let failLatest = false;
  await page.route("**/api/channels/c1/notes**", async (route) => {
    const isList = new URL(route.request().url()).pathname.endsWith("/notes");
    if (route.request().method() === "PATCH") {
      note = { ...note, content: "Latest server text", version: 2 };
      failLatest = true;
      await route.fulfill({
        status: 409,
        headers: { "Access-Control-Allow-Origin": "*" },
        json: { error: "conflict" },
      });
      return;
    }
    if (!isList && failLatest) {
      failLatest = false;
      await route.fulfill({
        status: 503,
        headers: { "Access-Control-Allow-Origin": "*" },
        json: { error: "offline" },
      });
      return;
    }
    await route.fulfill({
      headers: { "Access-Control-Allow-Origin": "*" },
      json: { data: isList ? [note] : note, txid: "1" },
    });
  });
  await open(page, "compose", "/channels/c1?note=retry-note", "");
  await expect(page.getByText("Original", { exact: true })).toBeVisible();
  await clickControl(page.getByRole("button", { name: "編集", exact: true }), "kotlin");
  const field = page.getByRole("textbox", { name: "本文", exact: true });
  await fillControl(page, field, "Unsaved draft", "kotlin");
  await page.getByRole("button", { name: "ノートを保存", exact: true }).click({ force: true });
  await page.getByRole("button", { name: "再試行", exact: true }).first().click({ force: true });
  await expect(page.getByText("Latest server text", { exact: true })).toBeVisible();
  await expect(field).toHaveText("Unsaved draft");
});
