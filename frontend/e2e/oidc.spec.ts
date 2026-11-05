import { expect, test, type Page } from "@playwright/test";

test.use({ locale: "ja-JP" });

const mockIdpPort = process.env.MOCK_IDP_PORT ?? "4100";
const mockIdpOrigin = `http://127.0.0.1:${mockIdpPort}`;
const SHAPE_SYNC_TIMEOUT_MS = 15_000;
const apiBaseUrl = `https://localhost:${process.env.E2E_HTTP2_PORT ?? "8443"}`;

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
  // 最初のアクセスでは dev サーバがモジュールを変換するので、CI の負荷によっては数十秒かかる。
  await expect(page.getByRole("heading", { name: "Mock IdP" })).toBeVisible({ timeout: 30_000 });
  await page.screenshot({ path: testInfo.outputPath("01-mock-idp-login.png") });

  await page.getByRole("button", { name: "ログイン" }).click();

  // コールバック処理後、シェルが描画される（Shape の購読には Bearer が必要）。
  await expect(page.getByRole("heading", { name: "チャンネル", exact: true })).toBeVisible({
    timeout: 30_000,
  });
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
  const created = page.waitForResponse(
    (response) =>
      new URL(response.url()).pathname === "/api/channels" &&
      response.request().method() === "POST",
  );
  await page.getByRole("button", { name: "チャンネル作成" }).click();
  expect((await created).ok()).toBe(true);
  // The API acknowledgement precedes Electric delivery. The dialog closes after
  // the channel transaction settles; the sidebar also needs its membership Shape.
  await expect(page.getByPlaceholder("新しいチャンネル名")).toBeHidden({
    timeout: SHAPE_SYNC_TIMEOUT_MS,
  });

  const channelLink = page.getByRole("link", { name: new RegExp(channelName) });
  await expect(channelLink).toBeVisible({ timeout: SHAPE_SYNC_TIMEOUT_MS });
  await channelLink.click();
  await expect(page.getByPlaceholder("メッセージを送信")).toBeVisible();

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
      { timeout: SHAPE_SYNC_TIMEOUT_MS },
    )
    .toContain(message);

  // 現在ユーザー（/api/users/me）が解決され、自分のメッセージには編集/削除が出る。
  const ownRow = page.getByRole("listitem").filter({ hasText: message });
  await ownRow.hover();
  await expect(ownRow.getByRole("button", { name: "編集" })).toBeVisible();
  await expect(ownRow.getByRole("button", { name: "削除" })).toBeVisible();
  await page.screenshot({ path: testInfo.outputPath("04-own-message-controls.png") });

  // ダークテーマでも操作群が出る（証跡用）。以降の画面はライトに戻す。
  await page.getByRole("link", { name: "Zoo設定" }).click();
  await page
    .getByRole("group", { name: "テーマ", exact: true })
    .getByRole("button", { name: "ダーク", exact: true })
    .click();
  await expect(page.locator("html")).toHaveClass(/dark/);
  await page.getByRole("button", { name: "チャンネル一覧へ戻る" }).click();
  await ownRow.hover();
  await page.screenshot({ path: testInfo.outputPath("05-own-message-controls-dark.png") });
  await page.getByRole("link", { name: "Zoo設定" }).click();
  await page
    .getByRole("group", { name: "テーマ", exact: true })
    .getByRole("button", { name: "ライト", exact: true })
    .click();
  await expect(page.locator("html")).not.toHaveClass(/dark/);

  await page.getByRole("button", { name: "チャンネル一覧へ戻る" }).click();

  // 別ユーザーへ切り替えてログアウト → 再ログインする。
  await page.request.post(`${mockIdpOrigin}/__mock/user`, {
    data: { sub: "user-b", preferredUsername: "bee", name: "Bee", email: "bee@example.com" },
  });
  await page.getByRole("button", { name: "ログアウト" }).click();
  await expect(page.getByRole("heading", { name: "ログアウトしました" })).toBeVisible();
  expect(new URL(page.url()).searchParams.get("state")).toBeTruthy();
  await page.screenshot({ path: testInfo.outputPath("06-logged-out.png") });

  await page.getByRole("button", { name: "ログイン", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Mock IdP" })).toBeVisible();
  await page.getByRole("button", { name: "ログイン" }).click();
  await expect(page.getByRole("heading", { name: "チャンネル", exact: true })).toBeVisible();

  // ログアウト → 再ログインはページ全体の遷移なので、Devtools 非表示をやり直す。
  await page.addStyleTag({
    content: 'button[aria-label="Open TanStack Devtools"]{display:none !important}',
  });

  // 未参加の公開チャンネルは閲覧でき、参加は送信欄で行う。
  await page.getByRole("button", { name: "チャンネルを探す" }).first().click();
  await page.getByRole("searchbox", { name: "チャンネル名で検索" }).fill(channelName);
  const browserJoinResponse = page.waitForResponse(
    (response) => response.request().method() === "POST" && response.url().endsWith("/join"),
  );
  await page
    .getByRole("dialog")
    .getByRole("link", { name: new RegExp(channelName) })
    .click();
  await expect(page.getByText(message, { exact: true })).toBeVisible();
  await expect(page.getByPlaceholder("メッセージを送信")).toHaveCount(0);
  await page.getByRole("button", { name: "参加して投稿", exact: true }).click();
  await browserJoinResponse;
  await expect(page.getByPlaceholder("メッセージを送信")).toBeVisible();
  expect(new URL(page.url()).pathname).toBe(`/channels/${channelId}`);
  // 参加したチャンネルはサイドバーにも出る。
  await expect(page.getByRole("link", { name: new RegExp(channelName) })).toBeVisible();

  // 同じ公開チャンネルを開くと、他人のメッセージには編集/削除が出ない。
  const otherRow = page.getByRole("listitem").filter({ hasText: message });
  await expect(otherRow).toBeVisible();
  await otherRow.hover();
  await expect(otherRow.getByRole("button", { name: "編集" })).toHaveCount(0);
  await expect(otherRow.getByRole("button", { name: "削除" })).toHaveCount(0);
  await page.screenshot({ path: testInfo.outputPath("07-other-users-message.png") });

  // 退出すると、未参加の公開チャンネルでは composer の代わりに参加導線が出る（#92）。
  await page.getByRole("button", { name: /メンバー/ }).click();
  const leaveResponse = page.waitForResponse(
    (response) => response.request().method() === "POST" && response.url().endsWith("/leave"),
  );
  await page.getByRole("button", { name: "退出", exact: true }).click();
  await leaveResponse;
  await expect(page.getByRole("button", { name: "参加して投稿" })).toBeVisible();
  await expect(page.getByPlaceholder("メッセージを送信")).toHaveCount(0);
  await page.screenshot({ path: testInfo.outputPath("08-join-to-post.png") });

  // 参加して投稿から再参加し、自分のメッセージには編集/削除が出る。
  const rejoinResponse = page.waitForResponse(
    (response) => response.request().method() === "POST" && response.url().endsWith("/join"),
  );
  await page.getByRole("button", { name: "参加して投稿" }).click();
  await rejoinResponse;
  await expect(page.getByPlaceholder("メッセージを送信")).toBeVisible();

  const ownMessage = `hello from bee ${channelName}`;
  await page.getByPlaceholder("メッセージを送信").fill(ownMessage);
  await page.getByRole("button", { name: "送信" }).click();
  const beeRow = page.getByRole("listitem").filter({ hasText: ownMessage });
  await beeRow.hover();
  await expect(beeRow.getByRole("button", { name: "編集" })).toBeVisible();
  await expect(beeRow.getByRole("button", { name: "削除" })).toBeVisible();
  await page.screenshot({ path: testInfo.outputPath("09-own-message-second-user.png") });

  // 退出の本人確認は /api/users/me で解決した id で行われる（#49）。
  // 1 回目の退出でメンバーペインは開いたままなので、そのまま退出する。
  const finalLeaveResponse = page.waitForResponse(
    (response) => response.request().method() === "POST" && response.url().endsWith("/leave"),
  );
  await page.getByRole("button", { name: "退出", exact: true }).click();
  await finalLeaveResponse;
  await expect(page.getByRole("button", { name: "参加", exact: true })).toBeVisible();
  await expect(page.getByText("あなた", { exact: true })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "参加して投稿" })).toBeVisible();
  await expect(page.getByPlaceholder("メッセージを送信")).toHaveCount(0);
  await page.screenshot({ path: testInfo.outputPath("10-join-leave.png") });
});
