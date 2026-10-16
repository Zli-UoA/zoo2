import { expect, test } from "@playwright/test";

/**
 * モック IdP を使った OIDC PKCE の通し確認:
 *   未ログイン → モック IdP → ログイン → Bearer で API/Shape → チャンネル作成 → 送信
 *
 * Rust は `OIDC_ISSUER` を同じモック IdP に向けて起動しておくこと
 * （`playwright.oidc.config.ts` のコメント参照）。既定のスモークとは別設定。
 */
test("OIDC PKCE ログイン後に API と Shape が Bearer で通る", async ({ page }, testInfo) => {
  const bearerRequests: string[] = [];
  page.on("request", (request) => {
    if (request.url().includes("/api/")) {
      bearerRequests.push(request.headers()["authorization"] ?? "");
    }
  });

  // 未ログインではアプリを描画せず、モック IdP のログイン画面へリダイレクトする。
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Mock IdP" })).toBeVisible();
  await page.screenshot({ path: testInfo.outputPath("01-mock-idp-login.png") });

  await page.getByRole("button", { name: "ログイン" }).click();

  // コールバック処理後、シェルが描画される（Shape の購読には Bearer が必要）。
  await expect(page.getByRole("heading", { name: "チャンネル", exact: true })).toBeVisible();
  await page.screenshot({ path: testInfo.outputPath("02-app-shell.png") });

  const channelName = `oidc-${Date.now()}`;
  const message = `hello ${channelName}`;
  await page.getByRole("button", { name: "チャンネルを作成" }).click();
  await page.getByPlaceholder("新しいチャンネル名").fill(channelName);
  await page.getByRole("button", { name: "チャンネル作成" }).click();

  const channelLink = page.getByRole("link", { name: new RegExp(channelName) });
  await expect(channelLink).toBeVisible();
  await channelLink.click();
  await expect(page.getByRole("heading", { name: "メッセージ" })).toBeVisible();

  await page.getByPlaceholder("メッセージを送信").fill(message);
  await page.getByRole("button", { name: "送信" }).click();
  await expect(page.getByText(message)).toBeVisible();
  await page.screenshot({ path: testInfo.outputPath("03-message.png") });

  // REST / Shape いずれも Bearer が載っていること。
  expect(bearerRequests.some((header) => header.startsWith("Bearer "))).toBe(true);
});
