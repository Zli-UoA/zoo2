import { expect, test } from "@playwright/test";

test("起動 → チャンネル作成 → メッセージ送信 → 反映", async ({ page }) => {
  const channelName = `smoke-${Date.now()}`;
  const message = `hello ${channelName}`;

  // 起動: トップ画面（チャンネル一覧）が表示される。
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "チャンネル" })).toBeVisible();

  // チャンネルを作成し、一覧に反映される。
  await page.getByPlaceholder("新しいチャンネル名").fill(channelName);
  await page.getByRole("button", { name: "チャンネル作成" }).click();

  const channelLink = page.getByRole("link", { name: new RegExp(channelName) });
  await expect(channelLink).toBeVisible();

  // メッセージ画面へ遷移する。
  await channelLink.click();
  await expect(page.getByRole("heading", { name: "メッセージ" })).toBeVisible();

  // 送信し、Electric 経由で反映される（楽観更新 + txid ハンドシェイク）。
  await page.getByPlaceholder("メッセージを送信").fill(message);
  await page.getByRole("button", { name: "送信" }).click();
  await expect(page.getByText(message)).toBeVisible();
});
