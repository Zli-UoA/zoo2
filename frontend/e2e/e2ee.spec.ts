import { execFileSync } from "node:child_process";
import { randomBytes } from "node:crypto";
import { fileURLToPath } from "node:url";
import {
  expect,
  test,
  type Browser,
  type BrowserContext,
  type Page,
  type TestInfo,
} from "@playwright/test";

/**
 * E2EE の通し確認（#363）。2 人のユーザー（Alice、Bob）がそれぞれ 2 端末を使い、次を最後まで通す。
 *
 *   1. 初回設定（1 台目。パスキーのスロットとリカバリーキー）
 *   2. 解錠（2 台目。リカバリーキー）
 *   3. 暗号化 DM の送受信（4 台すべてで復号できる）
 *   4. 絵文字 SAS 検証（#368。Alice の 1 台目と Bob の 1 台目）
 *   5. 失効（Alice の 1 台目が、パスキーで SSSS を開け直して 2 台目を失効する）
 *   6. 退出（Bob が DM を退出したあとのメッセージは、Bob の端末に鍵を配らない）
 *   7. DB に平文が残っていない
 *
 * 「端末」はブラウザーのコンテキストで分ける。コンテキストごとに CDP の仮想認証器（`hasPrf: true`）を
 * 付ける。zoo2 はパスキーを登録しない（登録は AuthZ が行う）ので、AuthZ の代わりにページから
 * `navigator.credentials.create()`（RP ID は issuer のホスト名の `localhost`、`prf: {}`）で登録する。
 * 仮想認証器のクレデンシャルの PRF の秘密は別の認証器へ写せないので、2 台目はリカバリーキーで解錠し、
 * パスキーでの解錠（既存のスロットを開ける）は 1 台目の失効の前の SSSS の開け直しで確かめる。
 */

const mockIdpOrigin = `http://localhost:${process.env.MOCK_IDP_PORT ?? "4101"}`;
const composeFile =
  process.env.E2E_COMPOSE_FILE ??
  fileURLToPath(new URL("../../infra/db/compose.yml", import.meta.url));
const DEVTOOLS_HIDDEN = 'button[aria-label="Open TanStack Devtools"]{display:none !important}';

/** 送った本文が DB に出てこないことを確かめるため、実行ごとに一意の小文字の英数字を本文に入れる。 */
const runId = randomBytes(6).toString("hex");
const secret = (label: string) => `e2eeplain${label}${runId}`;

interface MockUser {
  sub: string;
  preferredUsername: string;
  name: string;
  email: string;
}

const alice: MockUser = {
  sub: `e2ee-alice-${runId}`,
  preferredUsername: `alice${runId}`,
  name: `Alice ${runId}`,
  email: `alice-${runId}@example.com`,
};
const bob: MockUser = {
  sub: `e2ee-bob-${runId}`,
  preferredUsername: `bob${runId}`,
  name: `Bob ${runId}`,
  email: `bob-${runId}@example.com`,
};

interface Device {
  label: string;
  user: MockUser;
  context: BrowserContext;
  page: Page;
}

/**
 * モック IdP のログインは、そのときの「今のユーザー」で認可コードを出す。端末ごとにユーザーが違うので、
 * ユーザーの切り替えからログインの完了（認可コードの発行）までを 1 つずつ行う。
 */
let idpQueue: Promise<unknown> = Promise.resolve();
function withIdp<T>(task: () => Promise<T>): Promise<T> {
  const run = idpQueue.then(task, task);
  idpQueue = run.catch(() => undefined);
  return run;
}

async function setMockUser(page: Page, user: MockUser) {
  const response = await page.request.post(`${mockIdpOrigin}/__mock/user`, { data: user });
  expect(response.ok()).toBe(true);
}

/**
 * E2EE の直前の再認証（`max_age=0`）の別ウィンドウで、その端末のユーザーとしてログインする。
 * 別ウィンドウは認可コードを元のページに返して自分で閉じる。
 */
async function approveReauth(device: Device, popup: Page) {
  await withIdp(async () => {
    await popup.waitForURL((url) => url.pathname.endsWith("/authorize"), { timeout: 30_000 });
    await setMockUser(device.page, device.user);
    await popup.getByRole("button", { name: "ログイン" }).click();
    await popup.waitForURL((url) => url.pathname.endsWith("/auth/reauth-callback.html"), {
      timeout: 30_000,
      waitUntil: "commit",
    });
  }).catch((error: unknown) => {
    // 閉じたあとの待ちの失敗は無視する。ログインできなければ、元のページの表示の確認で落ちる。
    if (!popup.isClosed()) console.warn(`[${device.label}] reauth popup:`, error);
  });
}

async function openDevice(browser: Browser, label: string, user: MockUser): Promise<Device> {
  const context = await browser.newContext({
    baseURL: test.info().project.use.baseURL,
    ignoreHTTPSErrors: true,
    locale: "ja-JP",
  });
  const page = await context.newPage();
  const device: Device = { label, user, context, page };
  page.on("pageerror", (error) => console.warn(`[${label}] pageerror:`, error.message));
  // 再認証の別ウィンドウ（window.open）は同じコンテキストの新しいページになる。
  context.on("page", (popup) => {
    if (popup !== page) void approveReauth(device, popup);
  });

  // 端末ごとの仮想認証器。PRF（hmac-secret）に対応し、利用者の確認は自動で通す。
  const cdp = await context.newCDPSession(page);
  await cdp.send("WebAuthn.enable", { enableUI: false });
  await cdp.send("WebAuthn.addVirtualAuthenticator", {
    options: {
      protocol: "ctap2",
      ctap2Version: "ctap2_1",
      transport: "internal",
      hasResidentKey: true,
      hasUserVerification: true,
      isUserVerified: true,
      hasPrf: true,
      automaticPresenceSimulation: true,
    },
  });
  return device;
}

async function hideDevtools(page: Page) {
  await page.addStyleTag({ content: DEVTOOLS_HIDDEN });
}

async function login(device: Device) {
  const { page } = device;
  await withIdp(async () => {
    await setMockUser(page, device.user);
    await page.goto("/");
    // 最初のアクセスでは dev サーバーがモジュールを変換するので、CI では数十秒かかる。
    await expect(page.getByRole("heading", { name: "Mock IdP" })).toBeVisible({ timeout: 90_000 });
    await page.getByRole("button", { name: "ログイン" }).click();
    await page.waitForURL((url) => !url.pathname.startsWith("/auth/"), { timeout: 60_000 });
  });
  await expect(page.getByRole("heading", { name: "チャンネル", exact: true })).toBeVisible({
    timeout: 60_000,
  });
  await hideDevtools(page);
}

async function openSettings(device: Device, tab: "encryption" | "devices") {
  await device.page.goto(`/settings?tab=${tab}`);
  await hideDevtools(device.page);
}

/** AuthZ の代わりに、仮想認証器へ PRF を有効にしたパスキーを登録する（RP ID は localhost）。 */
async function registerPasskey(device: Device) {
  const prfEnabled = await device.page.evaluate(async (user) => {
    const credential = (await navigator.credentials.create({
      publicKey: {
        rp: { id: "localhost", name: "AuthZ (E2E)" },
        user: {
          id: crypto.getRandomValues(new Uint8Array(16)),
          name: user.preferredUsername,
          displayName: user.name,
        },
        challenge: crypto.getRandomValues(new Uint8Array(32)),
        pubKeyCredParams: [{ type: "public-key", alg: -7 }],
        authenticatorSelection: { residentKey: "required", userVerification: "required" },
        extensions: { prf: {} } as AuthenticationExtensionsClientInputs,
      },
    })) as PublicKeyCredential | null;
    const results = credential?.getClientExtensionResults() as
      | { prf?: { enabled?: boolean } }
      | undefined;
    return results?.prf?.enabled === true;
  }, device.user);
  expect(prfEnabled).toBe(true);
}

/** 1 台目の初回設定。パスキーのスロットを作り、表示したリカバリーキーを返す。 */
async function setUpEncryption(device: Device, testInfo: TestInfo): Promise<string> {
  const { page } = device;
  await openSettings(device, "encryption");
  await expect(page.getByText("暗号化はまだ設定していません")).toBeVisible({ timeout: 60_000 });
  await page.getByRole("button", { name: "設定を始める" }).click();

  const recoveryKeyText = page.locator('p[aria-label="リカバリーキー"]');
  await expect(recoveryKeyText).toBeVisible();
  const recoveryKey = (await recoveryKeyText.textContent())?.trim() ?? "";
  expect(recoveryKey).not.toBe("");
  await page.getByLabel("リカバリーキーを控えました").check();

  await page.getByRole("button", { name: "パスキーを追加" }).click();
  await expect(page.getByText("パスキーを追加しました")).toBeVisible();
  await page.screenshot({ path: testInfo.outputPath(`${device.label}-01-setup.png`) });

  // 保存の前に、別のウィンドウで再認証する（approveReauth）。
  await page.getByRole("button", { name: "保存する", exact: true }).click();
  await expect(page.getByText("この端末は解錠済みです")).toBeVisible({ timeout: 60_000 });
  await page.screenshot({ path: testInfo.outputPath(`${device.label}-02-unlocked.png`) });
  return recoveryKey;
}

/** 2 台目の解錠（リカバリーキー）。 */
async function unlockWithRecoveryKey(device: Device, recoveryKey: string, testInfo: TestInfo) {
  const { page } = device;
  await openSettings(device, "encryption");
  await expect(page.getByText("この端末はまだ解錠していません")).toBeVisible({ timeout: 60_000 });
  await page.getByLabel("リカバリーキー", { exact: true }).fill(recoveryKey);
  await page.getByRole("button", { name: "リカバリーキーで解錠" }).click();
  await expect(page.getByText("この端末は解錠済みです")).toBeVisible({ timeout: 60_000 });
  await page.screenshot({ path: testInfo.outputPath(`${device.label}-01-unlocked.png`) });
}

async function openChannel(device: Device, channelId: string) {
  await device.page.goto(`/channels/${channelId}`);
  await hideDevtools(device.page);
  await expect(device.page.getByPlaceholder("メッセージを送信")).toBeVisible({ timeout: 60_000 });
}

async function send(device: Device, text: string) {
  const { page } = device;
  await page.getByPlaceholder("メッセージを送信").fill(text);
  await page.getByRole("button", { name: "送信", exact: true }).click();
  await expect(page.getByText(text, { exact: true })).toBeVisible();
}

async function expectDecrypted(device: Device, text: string) {
  await expect(device.page.getByText(text, { exact: true })).toBeVisible({ timeout: 60_000 });
}

/** compose の postgres に psql で問い合わせ、1 列目の値を行ごとに返す。 */
function sql(query: string): string[] {
  const output = execFileSync(
    "docker",
    [
      "compose",
      "-f",
      composeFile,
      "exec",
      "-T",
      "postgres",
      "psql",
      "-U",
      "root",
      "-d",
      "zoo",
      "-At",
      "-v",
      "ON_ERROR_STOP=1",
      "-c",
      query,
    ],
    { encoding: "utf8" },
  );
  return output.split("\n").filter((line) => line !== "");
}

function literal(value: string): string {
  return `'${value.replace(/'/g, "''")}'`;
}

function userIdOf(user: MockUser): string {
  const [id] = sql(`SELECT user_id FROM user_identities WHERE sub = ${literal(user.sub)}`);
  expect(id).toBeTruthy();
  return id!;
}

function devicesOf(userId: string): { deviceId: string; deleted: boolean }[] {
  return sql(
    `SELECT device_id || '|' || (deleted_at IS NOT NULL) FROM e2ee_devices
       WHERE user_id = ${literal(userId)} ORDER BY created_at, device_id`,
  ).map((row) => {
    const [deviceId, deleted] = row.split("|");
    return { deviceId: deviceId!, deleted: deleted === "true" };
  });
}

function dbNow(): string {
  return sql("SELECT now()")[0]!;
}

/**
 * 端末あてに送った to-device の数。受信の ack で消える `e2ee_to_device_messages` ではなく、送信と同じ
 * トランザクションで保存し、ack では消えない送信の記録（`e2ee_to_device_txns`、30 日残る）を数える。
 * 宛先の端末が受け取って ack したあとでも、配ったことを見逃さないため。
 */
function toDeviceSentCount(
  recipientDeviceIds: string[],
  range: { after: string } | { before: string },
): number {
  const ids = recipientDeviceIds.map(literal).join(", ");
  const time =
    "after" in range
      ? `created_at > ${literal(range.after)}::timestamptz`
      : `created_at <= ${literal(range.before)}::timestamptz`;
  return Number(
    sql(
      `SELECT count(*) FROM e2ee_to_device_txns
         WHERE recipient_device_id IN (${ids}) AND ${time}`,
    )[0],
  );
}

test("2 人 × 2 端末で初回設定・解錠・暗号化 DM・SAS・失効・退出を通し、DB に平文が残らない", async ({
  browser,
}, testInfo) => {
  const a1 = await openDevice(browser, "alice-1", alice);
  const a2 = await openDevice(browser, "alice-2", alice);
  const b1 = await openDevice(browser, "bob-1", bob);
  const b2 = await openDevice(browser, "bob-2", bob);
  const devices: Device[] = [a1, a2, b1, b2];

  try {
    // ログイン（端末の登録はログインのあとに自動で行われる）。
    for (const device of devices) await login(device);

    // 1. 初回設定（1 台目）。
    await registerPasskey(a1);
    await registerPasskey(b1);
    const aliceRecoveryKey = await setUpEncryption(a1, testInfo);
    const bobRecoveryKey = await setUpEncryption(b1, testInfo);

    // 2. 解錠（2 台目）。
    await unlockWithRecoveryKey(a2, aliceRecoveryKey, testInfo);
    await unlockWithRecoveryKey(b2, bobRecoveryKey, testInfo);

    const aliceId = userIdOf(alice);
    const bobId = userIdOf(bob);
    // 初回設定で、パスキーとリカバリーキーの鍵スロットを 1 つずつ保存した。
    for (const userId of [aliceId, bobId]) {
      expect(
        sql(
          `SELECT kind FROM e2ee_secret_storage_key_slots
             WHERE user_id = ${literal(userId)} ORDER BY kind`,
        ),
      ).toEqual(["passkey", "recovery"]);
    }
    const aliceDevices = devicesOf(aliceId);
    const bobDevices = devicesOf(bobId);
    expect(aliceDevices).toHaveLength(2);
    expect(bobDevices).toHaveLength(2);
    // 2 台目の解錠で、自己署名鍵による端末への署名が保存されている。
    for (const [userId, list] of [
      [aliceId, aliceDevices],
      [bobId, bobDevices],
    ] as const) {
      for (const { deviceId } of list) {
        expect(
          Number(
            sql(
              `SELECT count(*) FROM e2ee_key_signatures
                 WHERE target_user_id = ${literal(userId)}
                   AND target_key_id IN (${literal(deviceId)}, ${literal(`ed25519:${deviceId}`)})`,
            )[0],
          ),
        ).toBeGreaterThan(0);
      }
    }

    // 3. 暗号化 DM の送受信。
    await a1.page.goto("/");
    await hideDevtools(a1.page);
    await a1.page.getByRole("button", { name: "新しいダイレクトメッセージ", exact: true }).click();
    const dialog = a1.page.getByRole("dialog");
    await dialog.getByRole("combobox", { name: "名前で検索" }).fill(bob.name);
    await dialog.getByRole("option", { name: new RegExp(bob.name) }).click();
    await dialog.getByLabel("エンドツーエンドで暗号化する").check();
    await dialog.getByRole("button", { name: "会話を始める", exact: true }).click();
    await expect(a1.page).toHaveURL(/\/channels\/[0-9a-f-]+$/);
    const dmId = new URL(a1.page.url()).pathname.split("/").at(-1)!;
    await hideDevtools(a1.page);
    await expect(a1.page.getByPlaceholder("メッセージを送信")).toBeVisible();

    const fromAlice = secret("fromalice");
    await send(a1, fromAlice);
    for (const device of [a2, b1, b2]) {
      await openChannel(device, dmId);
      await expectDecrypted(device, fromAlice);
    }
    const fromBob = secret("frombob");
    await send(b2, fromBob);
    for (const device of [a1, a2, b1]) await expectDecrypted(device, fromBob);
    await a1.page.screenshot({ path: testInfo.outputPath("alice-1-03-dm.png") });
    await b1.page.screenshot({ path: testInfo.outputPath("bob-1-03-dm.png") });

    // 送信端末の検証（docs/e2ee.md の「受信時の検証」の手順 4）。Alice の端末から見て、自分の別の
    // 端末のメッセージも、送る前に TOFU で信頼した Bob（2 台目は 1 台目が置いたピンを採用する）の
    // メッセージも、「未検証の送信者」にならない。
    for (const device of [a1, a2]) {
      await expect(device.page.getByText("未検証の送信者", { exact: true })).toHaveCount(0);
    }

    // 新しい端末での解錠（docs/e2ee.md の「初回設定と解錠の流れ」の 5）。Bob の端末が鍵バックアップに
    // 上げたセッションを、あとから解錠した 3 台目が取り込み、解錠より前のメッセージを読める。
    const backedUp = () =>
      Number(
        sql(`SELECT count(*) FROM e2ee_key_backup_sessions WHERE user_id = ${literal(bobId)}`)[0],
      );
    await expect.poll(backedUp, { timeout: 60_000 }).toBeGreaterThanOrEqual(2);
    const b3 = await openDevice(browser, "bob-3", bob);
    devices.push(b3);
    await login(b3);
    await unlockWithRecoveryKey(b3, bobRecoveryKey, testInfo);
    await openChannel(b3, dmId);
    await expectDecrypted(b3, fromAlice);
    await expectDecrypted(b3, fromBob);
    // バックアップ由来のセッションで復号したので、送信者は確かめられない（仕様どおり）。
    await expect(b3.page.getByText("送信者を確かめられません", { exact: true })).toHaveCount(2);
    await b3.page.screenshot({ path: testInfo.outputPath("bob-3-03-backup.png") });
    // 以降の SAS、失効、退出は 2 人 × 2 端末で確かめるので、3 台目は閉じる（登録した端末は残る）。
    await b3.context.close();

    // 4. 絵文字 SAS 検証（Alice の 1 台目 → Bob）。Bob の端末すべてに要求が届き、1 台目で受け入れる。
    await openSettings(a1, "devices");
    const bobRow = a1.page.getByRole("listitem").filter({ hasText: bob.name });
    await expect(bobRow).toBeVisible({ timeout: 60_000 });
    await bobRow.getByRole("button", { name: "絵文字で検証" }).click();
    await expect(b1.page.getByText(`${alice.name} さんが検証を求めています`)).toBeVisible({
      timeout: 60_000,
    });
    await expect(b2.page.getByText(`${alice.name} さんが検証を求めています`)).toBeVisible({
      timeout: 60_000,
    });
    await b1.page.getByRole("button", { name: "受け入れる" }).click();
    const aliceEmoji = a1.page.getByRole("list", { name: "検証の絵文字" });
    const bobEmoji = b1.page.getByRole("list", { name: "検証の絵文字" });
    await expect(aliceEmoji).toBeVisible({ timeout: 60_000 });
    await expect(bobEmoji).toBeVisible({ timeout: 60_000 });
    const emoji = (await aliceEmoji.innerText()).trim();
    expect(emoji).not.toBe("");
    expect((await bobEmoji.innerText()).trim()).toBe(emoji);
    await a1.page.screenshot({ path: testInfo.outputPath("alice-1-04-sas.png") });
    // 一致すると答えると、それぞれ再認証の別ウィンドウが開く（approveReauth が 1 つずつログインする）。
    await a1.page.getByRole("button", { name: "一致する" }).click();
    await b1.page.getByRole("button", { name: "一致する" }).click();
    await expect(a1.page.getByText(`${bob.name} さんを検証しました`)).toBeVisible({
      timeout: 60_000,
    });
    await expect(b1.page.getByText(`${alice.name} さんを検証しました`)).toBeVisible({
      timeout: 60_000,
    });
    await a1.page.screenshot({ path: testInfo.outputPath("alice-1-05-sas-done.png") });
    // Bob の 2 台目には、別の端末で検証が始まったことを出す。
    await expect(b2.page.getByText("あなたの別の端末で検証が始まりました。")).toBeVisible({
      timeout: 60_000,
    });
    for (const device of [a1, b1, b2]) {
      // フッターの「閉じる」（ダイアログの右上の×も同じ名前）。
      const dialog = device.page.getByRole("dialog");
      await dialog.getByRole("button", { name: "閉じる", exact: true }).first().click();
      await expect(dialog).toBeHidden();
    }
    // 互いのマスター鍵に、自分のユーザー署名鍵で署名した。
    await expect(bobRow).toContainText("検証済み");
    for (const [signer, target] of [
      [aliceId, bobId],
      [bobId, aliceId],
    ]) {
      expect(
        Number(
          sql(
            `SELECT count(*) FROM e2ee_key_signatures
               WHERE signer_user_id = ${literal(signer!)} AND target_user_id = ${literal(target!)}`,
          )[0],
        ),
      ).toBeGreaterThan(0);
    }

    // 5. 失効（Alice の 1 台目が 2 台目を失効する。端末だけ）。始める前にパスキーで SSSS を開け直す。
    await openSettings(a1, "devices");
    await a1.page.getByRole("button", { name: "失効を始める" }).click();
    await a1.page.getByRole("checkbox").first().check();
    await a1.page.getByRole("radio", { name: /端末だけ/ }).check();
    await a1.page.getByLabel("AuthZ でセッションとサービス接続を停止した").check();
    await a1.page.getByRole("button", { name: "次へ" }).click();
    await a1.page.getByRole("button", { name: "パスキーで解錠" }).click();
    await a1.page.getByRole("button", { name: "失効を実行" }).click();
    await expect(a1.page.getByText("失効が終わりました。")).toBeVisible({ timeout: 120_000 });
    await a1.page.screenshot({ path: testInfo.outputPath("alice-1-06-revoked.png") });
    await a1.page.getByRole("button", { name: "閉じる" }).click();
    const revokedAt = dbNow();
    // 選べたのは今の端末以外の 1 台（2 台目）だけなので、Alice の端末のうち 1 台だけが削除された。
    const aliceDevicesAfter = devicesOf(aliceId);
    expect(aliceDevicesAfter.filter((d) => d.deleted)).toHaveLength(1);
    const revokedDeviceId = aliceDevicesAfter.find((d) => d.deleted)!.deviceId;

    // 失効のあとのメッセージは、失効した端末には届かない。
    await openChannel(a1, dmId);
    await openChannel(b1, dmId);
    const afterRevocation = secret("afterrevocation");
    await send(b1, afterRevocation);
    await expectDecrypted(a1, afterRevocation);
    await expectDecrypted(b2, afterRevocation);
    const replyAfterRevocation = secret("replyafterrevocation");
    await send(a1, replyAfterRevocation);
    await expectDecrypted(b1, replyAfterRevocation);
    // 失効の前は、失効した端末にも鍵を配っていた（数え方が配布を見つけられることの確認）。
    expect(toDeviceSentCount([revokedDeviceId], { before: revokedAt })).toBeGreaterThan(0);
    expect(toDeviceSentCount([revokedDeviceId], { after: revokedAt })).toBe(0);
    // 失効した端末には鍵が届かず、失効のあとの 2 件は復号できない（失効の前のメッセージは読める）。
    await expect(
      a2.page.getByText("このメッセージを復号する鍵がまだ届いていません。", { exact: true }),
    ).toHaveCount(2);
    await expect(a2.page.getByText(fromBob, { exact: true })).toBeVisible();
    await expect(a2.page.getByText(afterRevocation, { exact: true })).toHaveCount(0);
    await expect(a2.page.getByText(replyAfterRevocation, { exact: true })).toHaveCount(0);
    await a2.page.screenshot({ path: testInfo.outputPath("alice-2-07-after-revocation.png") });

    // 6. 退出（Bob が DM を退出する）。退出のあとのメッセージの鍵は、Bob の端末に配らない。
    const aliceDmLink = b1.page.getByRole("link", { name: new RegExp(alice.name) });
    await expect(aliceDmLink).toBeVisible();
    await b1.page.getByRole("button", { name: "ダイレクトメッセージのメニュー" }).click();
    await b1.page.getByRole("menuitem", { name: "退出", exact: true }).click();
    const leaveResponse = b1.page.waitForResponse(
      (response) => response.request().method() === "POST" && response.url().endsWith("/leave"),
    );
    await b1.page
      .getByRole("alertdialog")
      .getByRole("button", { name: "退出", exact: true })
      .click();
    expect((await leaveResponse).ok()).toBe(true);
    await expect(aliceDmLink).toHaveCount(0);
    const leftAt = dbNow();
    const afterLeave = secret("afterleave");
    await send(a1, afterLeave);
    const bobDeviceIds = bobDevices.map((d) => d.deviceId);
    expect(toDeviceSentCount(bobDeviceIds, { before: leftAt })).toBeGreaterThan(0);
    expect(toDeviceSentCount(bobDeviceIds, { after: leftAt })).toBe(0);
    // Bob の端末には、退出のあとのメッセージが出ない（DM の一覧からも消える）。
    await expect(b2.page.getByRole("link", { name: new RegExp(alice.name) })).toHaveCount(0);
    for (const device of [b1, b2]) {
      await expect(device.page.getByText(afterLeave, { exact: true })).toHaveCount(0);
    }
    await a1.page.screenshot({ path: testInfo.outputPath("alice-1-08-after-leave.png") });

    // 退出でメンバーの epoch が進み、Alice は新しい Megolm セッションで送った。
    const rows = sql(
      `SELECT user_id || '|' || epoch || '|' || (ciphertext->>'session_id') FROM messages
         WHERE channel_id = ${literal(dmId)} ORDER BY created_at, id`,
    ).map((row) => {
      const [userId, epoch, sessionId] = row.split("|");
      return { userId: userId!, epoch: Number(epoch), sessionId: sessionId! };
    });
    expect(rows).toHaveLength(5);
    const [, , , beforeLeave, last] = rows;
    expect(beforeLeave!.userId).toBe(aliceId);
    expect(last!.userId).toBe(aliceId);
    expect(last!.epoch).toBeGreaterThan(beforeLeave!.epoch);
    expect(last!.sessionId).not.toBe(beforeLeave!.sessionId);

    // 7. DB に平文が残っていない。暗号化した列（messages.ciphertext）だけを使い、本文の列は空。
    expect(
      Number(
        sql(
          `SELECT count(*) FROM messages
             WHERE channel_id = ${literal(dmId)} AND (content <> '' OR ciphertext IS NULL)`,
        )[0],
      ),
    ).toBe(0);
    // DB 全体のダンプのどこにも、送った本文が出てこない（検索用の列、通知、Shape の元の表を含む）。
    const dump = execFileSync(
      "docker",
      [
        "compose",
        "-f",
        composeFile,
        "exec",
        "-T",
        "postgres",
        "pg_dump",
        "-U",
        "root",
        "-d",
        "zoo",
        "--data-only",
      ],
      { encoding: "utf8", maxBuffer: 512 * 1024 * 1024 },
    ).toLowerCase();
    // ダンプが空でなく、平文で保存する値（DM の ID、ユーザー名）は見つかることを先に確かめる。
    expect(dump).toContain(dmId);
    expect(dump).toContain(alice.preferredUsername);
    for (const text of [fromAlice, fromBob, afterRevocation, replyAfterRevocation, afterLeave]) {
      expect(dump, `DB に平文 ${text} が残っている`).not.toContain(text);
    }
  } catch (error) {
    // 端末は fixture の page ではないので、失敗したときの画面を端末ごとに残す。
    for (const device of devices) {
      await device.page
        .screenshot({ path: testInfo.outputPath(`${device.label}-failure.png`) })
        .catch(() => undefined);
    }
    throw error;
  } finally {
    // 途中で落ちたときに、元の失敗を後片付けの失敗で隠さない。
    for (const device of devices) await device.context.close().catch(() => undefined);
  }
});
