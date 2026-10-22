import { expect, test, type Page } from "@playwright/test";

const api = process.env.VITE_API_BASE_URL ?? "http://localhost:8000";
const userId = process.env.VITE_DEV_USER_ID ?? "00000000-0000-0000-0000-000000000001";
const headers = { "X-Dev-User": userId };
const imagePattern = "https://www.gstatic.com/android/keyboard/emojikitchen/**";
const png = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aH0kAAAAASUVORK5CYII=",
  "base64",
);

async function chooseEmoji(page: Page, label: string, name: string) {
  await page.getByRole("combobox", { name: label }).fill(name);
  await page.getByRole("option", { name, exact: true }).click();
}

async function openMessage(page: Page) {
  const channelResponse = await page.request.post(`${api}/api/channels`, {
    headers,
    data: { channel_name: `kitchen-${crypto.randomUUID().slice(0, 8)}` },
  });
  expect(channelResponse.ok()).toBe(true);
  const { data: channel } = await channelResponse.json();
  const messageResponse = await page.request.post(`${api}/api/messages`, {
    headers,
    data: { channel_id: channel.id, content: "Emoji Kitchen test" },
  });
  expect(messageResponse.ok()).toBe(true);
  const { data: message } = await messageResponse.json();
  // External images are deterministic in CI; the published URL is checked separately.
  await page.route(imagePattern, (route) => route.fulfill({ contentType: "image/png", body: png }));
  await page.goto(`/channels/${channel.id}`);
  await expect(page.getByText("Emoji Kitchen test", { exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "リアクションを追加", exact: true })).toBeEnabled();
  return { channel, message };
}

test("Kitchen reaction persists, receives live updates and removes only my reaction", async ({
  page,
}) => {
  const { channel, message } = await openMessage(page);
  await page.getByRole("button", { name: "リアクションを追加", exact: true }).click();
  await expect(page.getByRole("img", { name: "組み合わせのプレビュー" })).toHaveAttribute(
    "src",
    "https://www.gstatic.com/android/keyboard/emojikitchen/20230301/u1f600/u1f600_u1f431.png",
  );
  await expect(page.getByRole("button", { name: "この組み合わせを追加" })).toBeEnabled();
  const saved = page.waitForResponse(
    (response) =>
      response.url().endsWith(`/api/messages/${message.id}/reactions`) &&
      response.request().method() === "POST",
  );
  await page.getByRole("button", { name: "この組み合わせを追加" }).click();
  expect((await saved).ok()).toBe(true);
  const mine = page.getByRole("button", { name: /1件（解除）/ });
  await expect(mine).toHaveAttribute("aria-pressed", "true");
  await page.reload();
  await expect(mine).toBeVisible();
  await expect(mine.getByRole("img")).toHaveAttribute("src", /u1f600_u1f431\.png$/);

  const otherId = crypto.randomUUID();
  const otherHeaders = { "X-Dev-User": otherId };
  expect(
    (
      await page.request.post(`${api}/api/users`, {
        data: {
          id: otherId,
          user_name: `kitchen-${otherId.slice(0, 8)}`,
          display_name: "Other User",
        },
      })
    ).ok(),
  ).toBe(true);
  expect(
    (
      await page.request.post(`${api}/api/channels/${channel.id}/join`, { headers: otherHeaders })
    ).ok(),
  ).toBe(true);
  expect(
    (
      await page.request.post(`${api}/api/messages/${message.id}/reactions`, {
        headers: otherHeaders,
        data: { emoji: "kitchen:😀_🐱" },
      })
    ).ok(),
  ).toBe(true);
  await expect(page.getByRole("button", { name: /2件（解除）/ })).toBeVisible();
  await page.getByRole("button", { name: /2件（解除）/ }).click();
  const remaining = page.getByRole("button", { name: /1件（追加）/ });
  await expect(remaining).toHaveAttribute("aria-pressed", "false");
  await page.reload();
  await expect(remaining).toBeVisible();
});

test("failed writes roll back and catalog loading can be retried", async ({ page }) => {
  let catalogAttempts = 0;
  await page.route("**/emoji-kitchen*.json", (route) =>
    ++catalogAttempts === 1 ? route.fulfill({ status: 503 }) : route.continue(),
  );
  await openMessage(page);
  await page.getByRole("button", { name: "リアクションを追加", exact: true }).click();
  await expect(page.getByText("絵文字の一覧を読み込めませんでした。")).toBeVisible();
  await page.getByRole("button", { name: "再試行" }).click();
  await expect(page.getByRole("button", { name: "この組み合わせを追加" })).toBeEnabled();
  await chooseEmoji(page, "1つ目の絵文字", "joy");
  await chooseEmoji(page, "2つ目の絵文字", "lacrosse");
  await expect(page.getByText("この組み合わせは未対応です。")).toBeVisible();
  await expect(page.getByRole("button", { name: "この組み合わせを追加" })).toBeDisabled();
  await page.unroute(imagePattern);
  await page.route(imagePattern, (route) => route.abort());
  await chooseEmoji(page, "2つ目の絵文字", "heart eyes");
  await expect(
    page.getByText("画像を読み込めませんでした。別の組み合わせをお試しください。"),
  ).toBeVisible();
  await expect(page.getByRole("button", { name: "この組み合わせを追加" })).toBeDisabled();
  await page.route("**/api/messages/*/reactions", (route) =>
    route.fulfill({
      status: 403,
      contentType: "application/json",
      body: JSON.stringify({ error: "forbidden" }),
    }),
  );
  await page.getByRole("button", { name: "👍", exact: true }).click();
  await expect(page.getByText("操作に失敗しました", { exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: /👍 1件/ })).toHaveCount(0);
});
