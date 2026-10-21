import { expect, test, type Page } from "@playwright/test";

test.use({ locale: "ja-JP" });

const mockIdpPort = process.env.MOCK_IDP_PORT ?? "4100";
const mockIdpOrigin = `http://127.0.0.1:${mockIdpPort}`;
const apiBaseUrl = process.env.VITE_API_BASE_URL ?? "http://localhost:8000";

/**
 * Shape の初期同期をプロトコルどおりに最後まで辿り、受け取った行を文字列で返す。
 * `offset=-1` の応答はスナップショットの終端ヘッダだけを返すことがあるため、
 * `electric-handle` / `electric-offset` で続きを取得する（Electric の仕様）。
 */
async function readShape(page: Page, url: string, bearer: string): Promise<string> {
  let handle: string | null = null;
  let offset = "-1";
  let body = "";
  for (let attempt = 0; attempt < 10; attempt += 1) {
    const requestUrl = new URL(url);
    requestUrl.searchParams.set("offset", offset);
    if (handle) {
      requestUrl.searchParams.set("handle", handle);
    } else {
      requestUrl.searchParams.set("log", "full");
    }
    const response = await page.request.get(requestUrl.toString(), {
      headers: { Authorization: bearer },
    });
    expect(response.ok()).toBe(true);
    body += await response.text();
    if (response.headers()["electric-up-to-date"] === "true") {
      return body;
    }
    handle = response.headers()["electric-handle"] ?? handle;
    offset = response.headers()["electric-offset"] ?? offset;
    if (!handle) {
      return body;
    }
  }
  return body;
}

/**
 * モック IdP を使った OIDC PKCE の通し確認:
 *   未ログイン → モック IdP → ログイン → Bearer で API/Shape → チャンネル作成 → 送信
 *   → 自分のメッセージだけに編集/削除が出る → ログアウト → 別ユーザーでログイン
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

  // 前回の実行が残した sub に依存しないよう、ログイン前にユーザー A を固定する。
  await page.request.post(`${mockIdpOrigin}/__mock/user`, {
    data: {
      sub: "user-a",
      preferredUsername: "smoke",
      name: "Smoke User",
      email: "smoke@example.com",
    },
  });

  // 未ログインではアプリを描画せず、モック IdP のログイン画面へリダイレクトする。
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Mock IdP" })).toBeVisible();
  await page.screenshot({ path: testInfo.outputPath("01-mock-idp-login.png") });

  await page.getByRole("button", { name: "ログイン" }).click();

  // コールバック処理後、シェルが描画される（Shape の購読には Bearer が必要）。
  await expect(page.getByRole("heading", { name: "チャンネル", exact: true })).toBeVisible();
  await page.screenshot({ path: testInfo.outputPath("02-app-shell.png") });

  // Dev ビルドの TanStack Devtools 起動ボタンが右下の送信ボタン等に重なるため、
  // 操作の前に非表示にする（アプリ本体の挙動確認には影響しない）。
  await page.addStyleTag({
    content: 'button[aria-label="Open TanStack Devtools"]{display:none !important}',
  });

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
  const bearer = bearerRequests.filter((header) => header.startsWith("Bearer ")).slice(-1)[0];
  expect(bearer).toBeTruthy();

  // 画面の反映が楽観更新だけに依存しないよう、Shape プロキシの応答にも
  // 送信した行が含まれることを Bearer 付きで確認する（live=true は使わない）。
  const channelId = new URL(page.url()).pathname.split("/").slice(-1)[0];
  await expect
    .poll(
      () => readShape(page, `${apiBaseUrl}/api/shapes/messages?channel_id=${channelId}`, bearer!),
      { timeout: 15_000 },
    )
    .toContain(message);

  // 現在ユーザー（/api/users/me）が解決され、自分のメッセージには編集/削除が出る。
  const ownRow = page.getByRole("listitem").filter({ hasText: message });
  await ownRow.hover();
  await expect(ownRow.getByRole("button", { name: "編集" })).toBeVisible();
  await expect(ownRow.getByRole("button", { name: "削除" })).toBeVisible();
  await page.screenshot({ path: testInfo.outputPath("04-own-message-controls.png") });

  // ダークテーマでも操作群が出る（証跡用）。以降の画面はライトに戻す。
  await page.getByRole("button", { name: "テーマを切り替える" }).click();
  await page.getByRole("menuitemradio", { name: "ダーク" }).click();
  await expect(page.locator("html")).toHaveClass(/dark/);
  await expect(page.getByRole("menuitemradio", { name: "ダーク" })).toBeHidden();
  await ownRow.hover();
  await page.screenshot({ path: testInfo.outputPath("05-own-message-controls-dark.png") });
  await page.getByRole("button", { name: "テーマを切り替える" }).click();
  await page.getByRole("menuitemradio", { name: "ライト" }).click();
  await expect(page.locator("html")).not.toHaveClass(/dark/);
  await expect(page.getByRole("menuitemradio", { name: "ライト" })).toBeHidden();

  // 別ユーザーへ切り替えてログアウト → 再ログインする。
  await page.request.post(`${mockIdpOrigin}/__mock/user`, {
    data: { sub: "user-b", preferredUsername: "bee", name: "Bee", email: "bee@example.com" },
  });
  await page.getByRole("button", { name: "ログアウト" }).click();
  await expect(page.getByRole("heading", { name: "ログアウトしました" })).toBeVisible();
  await page.screenshot({ path: testInfo.outputPath("06-logged-out.png") });

  await page.getByRole("button", { name: "ログイン", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Mock IdP" })).toBeVisible();
  await page.getByRole("button", { name: "ログイン" }).click();
  await expect(page.getByRole("heading", { name: "チャンネル", exact: true })).toBeVisible();

  // ログアウト → 再ログインはページ全体の遷移なので、Devtools 非表示をやり直す。
  await page.addStyleTag({
    content: 'button[aria-label="Open TanStack Devtools"]{display:none !important}',
  });

  // サイドバーは参加中のみなので、「チャンネルを探す」から参加して開く。
  await page.getByRole("button", { name: "チャンネルを探す" }).first().click();
  await page.getByRole("searchbox", { name: "チャンネル名で検索" }).fill(channelName);
  const joinResponse = page.waitForResponse(
    (response) => response.request().method() === "POST" && response.url().endsWith("/join"),
  );
  await page.getByRole("button", { name: "参加する", exact: true }).click();
  await joinResponse;
  await expect(page.getByRole("heading", { name: "メッセージ" })).toBeVisible();
  expect(new URL(page.url()).pathname).toBe(`/channels/${channelId}`);
  // 参加したチャンネルはサイドバーにも出る。
  await expect(page.getByRole("link", { name: new RegExp(channelName) })).toBeVisible();
  const otherRow = page.getByRole("listitem").filter({ hasText: message });
  await expect(otherRow).toBeVisible();
  await otherRow.hover();
  await expect(otherRow.getByRole("button", { name: "編集" })).toHaveCount(0);
  await expect(otherRow.getByRole("button", { name: "削除" })).toHaveCount(0);
  await page.screenshot({ path: testInfo.outputPath("07-other-users-message.png") });

  // 自分のメッセージには改めて編集/削除が出る。
  const ownMessage = `hello from bee ${channelName}`;
  await page.getByPlaceholder("メッセージを送信").fill(ownMessage);
  await page.getByRole("button", { name: "送信" }).click();
  const beeRow = page.getByRole("listitem").filter({ hasText: ownMessage });
  await beeRow.hover();
  await expect(beeRow.getByRole("button", { name: "編集" })).toBeVisible();
  await expect(beeRow.getByRole("button", { name: "削除" })).toBeVisible();
  await page.screenshot({ path: testInfo.outputPath("08-own-message-second-user.png") });

  // 退出: 退出の本人確認は /api/users/me で解決した id で行われる（#49）。
  // メンバーペインは既定で閉じているため、ヘッダの「メンバー」から開く。
  // 参加は上の「チャンネルを探す」で済んでいる。
  await page.getByRole("button", { name: /メンバー/ }).click();
  await expect(page.getByRole("button", { name: "退出", exact: true })).toBeVisible();
  await expect(page.getByText("あなた", { exact: true })).toBeVisible();
  const leaveResponse = page.waitForResponse(
    (response) => response.request().method() === "POST" && response.url().endsWith("/leave"),
  );
  await page.getByRole("button", { name: "退出", exact: true }).click();
  await leaveResponse;
  await expect(page.getByRole("button", { name: "参加", exact: true })).toBeVisible();
  await expect(page.getByText("あなた", { exact: true })).toHaveCount(0);
  await page.screenshot({ path: testInfo.outputPath("09-join-leave.png") });
});
