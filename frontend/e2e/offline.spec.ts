import { expect, test, type Page } from "@playwright/test";

const api = process.env.VITE_API_BASE_URL ?? "http://localhost:8000";
const userId = process.env.VITE_DEV_USER_ID ?? "00000000-0000-0000-0000-000000000001";
const headers = { "X-Dev-User": userId };

async function createChannel(page: Page, label: string) {
  const response = await page.request.post(`${api}/api/channels`, {
    headers,
    data: { channel_name: `${label}-${crypto.randomUUID().slice(0, 8)}` },
  });
  expect(response.ok()).toBe(true);
  return (await response.json()).data as { id: string };
}

async function postMessage(page: Page, channelId: string, content: string) {
  const response = await page.request.post(`${api}/api/messages`, {
    headers,
    data: { channel_id: channelId, content },
  });
  expect(response.ok()).toBe(true);
  return (await response.json()).data as { id: string };
}

async function postAsOtherUser(page: Page, channelId: string, content: string) {
  const otherId = "00000000-0000-0000-0000-000000000002";
  const otherHeaders = { "X-Dev-User": otherId };
  await page.request.post(`${api}/api/users`, {
    data: { id: otherId, user_name: "offline-other", display_name: "Other User" },
  });
  const joined = await page.request.post(`${api}/api/channels/${channelId}/join`, {
    headers: otherHeaders,
  });
  expect(joined.ok()).toBe(true);
  const response = await page.request.post(`${api}/api/messages`, {
    headers: otherHeaders,
    data: { channel_id: channelId, content },
  });
  expect(response.ok()).toBe(true);
}

async function waitForOfflineDatabase(page: Page) {
  await expect
    .poll(
      () =>
        page.evaluate(async () => {
          const root = await navigator.storage.getDirectory();
          const files: string[] = [];
          for await (const [name, handle] of root.entries()) {
            if (handle.kind === "file" && name.startsWith("zoo-offline-")) files.push(name);
          }
          return files.length;
        }),
      { timeout: 15_000 },
    )
    .toBeGreaterThan(0);
}

test.use({ locale: "ja-JP" });

test("offline reload restores the app shell and persisted messages", async ({ page, context }) => {
  const channel = await createChannel(page, "offline");
  const content = `offline-message-${crypto.randomUUID()}`;
  await postMessage(page, channel.id, content);

  await page.goto(`/channels/${channel.id}`);
  await expect(page.getByText(content, { exact: true })).toBeVisible();
  await expect(page.getByPlaceholder("メッセージを送信")).toBeEnabled();
  await page.evaluate(async () => {
    await navigator.serviceWorker?.ready;
  });
  await waitForOfflineDatabase(page);
  // Let the synced row reach SQLite before cutting the network.
  await page.waitForTimeout(1_500);

  await context.setOffline(true);
  await page.reload();

  await expect(page.getByText(content, { exact: true })).toBeVisible({ timeout: 20_000 });
  await expect(page.getByRole("status").filter({ hasText: "オフラインです" })).toBeVisible();
  const composer = page.getByPlaceholder("オフラインのため送信できません");
  await expect(composer).toBeVisible();
  await expect(composer).toBeDisabled();

  await context.setOffline(false);
  await expect(page.getByRole("status").filter({ hasText: "オフラインです" })).toBeHidden({
    timeout: 10_000,
  });
});

test("switching users opens a separate offline database", async ({ page }) => {
  const databaseFiles = () =>
    page.evaluate(async () => {
      const root = await navigator.storage.getDirectory();
      const files: string[] = [];
      for await (const [name, handle] of root.entries()) {
        if (handle.kind === "file" && name.startsWith("zoo-offline-")) files.push(name);
      }
      return files.sort();
    });

  await page.goto("/");
  await expect.poll(databaseFiles, { timeout: 15_000 }).not.toHaveLength(0);
  const first = await databaseFiles();

  await page.selectOption(
    'select[data-testid="preview-user"]',
    "00000000-0000-0000-0000-000000000002",
  );
  await page.waitForTimeout(2_000);
  await expect
    .poll(async () => (await databaseFiles()).length, { timeout: 20_000 })
    .toBeGreaterThan(first.length);
  const second = await databaseFiles();
  expect(second.filter((name) => name.endsWith(".sqlite") && !first.includes(name))).toHaveLength(
    1,
  );
});

test("a new message does not move a reader who scrolled up", async ({ page }) => {
  const channel = await createChannel(page, "window");
  for (let index = 0; index < 40; index += 1) {
    await postMessage(page, channel.id, `history-${index}`);
  }

  await page.goto(`/channels/${channel.id}`);
  await expect(page.getByText("history-39", { exact: true })).toBeVisible({ timeout: 20_000 });
  await page.waitForTimeout(500);

  const scroller = page.getByTestId("message-scroll");
  await scroller.evaluate((element) => {
    element.scrollTop = 0;
    element.dispatchEvent(new Event("scroll"));
  });
  await expect.poll(() => scroller.evaluate((element) => element.scrollTop)).toBe(0);
  await page.waitForTimeout(300);
  const anchor = page.getByText("history-1", { exact: true });
  await expect(anchor).toBeVisible();
  const before = await anchor.boundingBox();

  // 自分の投稿は最下部へ追従する仕様なので、他ユーザーからの新着で確認する。
  await postAsOtherUser(page, channel.id, "live-new");
  await expect(page.getByRole("button", { name: "新着メッセージ" })).toBeVisible({
    timeout: 20_000,
  });

  const after = await anchor.boundingBox();
  expect(before).not.toBeNull();
  expect(after).not.toBeNull();
  expect(Math.abs(after!.y - before!.y)).toBeLessThanOrEqual(2);
});
