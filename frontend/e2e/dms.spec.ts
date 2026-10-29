import { expect, request, test } from "@playwright/test";

test.use({ locale: "ja-JP" });

const apiBaseUrl = process.env.VITE_API_BASE_URL ?? "http://localhost:8000";

async function createPartner(displayName: string, userName: string) {
  const context = await request.newContext();
  try {
    const id = crypto.randomUUID();
    const response = await context.post(`${apiBaseUrl}/api/users`, {
      data: { id, user_name: userName, display_name: displayName },
    });
    expect(response.ok()).toBeTruthy();
    return id;
  } finally {
    await context.dispose();
  }
}

async function hideDevtools(page: import("@playwright/test").Page) {
  await page.addStyleTag({
    content: 'button[aria-label="Open TanStack Devtools"]{display:none !important}',
  });
}

async function openDmDialog(page: import("@playwright/test").Page) {
  await page.getByRole("button", { name: "新しいダイレクトメッセージ", exact: true }).click();
  await expect(page.getByRole("dialog")).toBeVisible();
}

async function selectPartner(page: import("@playwright/test").Page, displayName: string) {
  await page.getByRole("combobox", { name: "名前で検索" }).fill(displayName);
  await page.getByRole("option", { name: new RegExp(displayName) }).click();
}

async function choosePartner(page: import("@playwright/test").Page, displayName: string) {
  await selectPartner(page, displayName);
  await page.getByRole("button", { name: "会話を始める", exact: true }).click();
}

test("1対1 DM の作成・重複・退出・再オープン", async ({ page }) => {
  const suffix = Date.now();
  const displayName = `DM Partner ${suffix}`;
  const userName = `dm_partner_${suffix}`;
  await createPartner(displayName, userName);

  await page.goto("/");
  await hideDevtools(page);
  await expect(page.getByRole("heading", { name: "チャンネル", exact: true })).toBeVisible();

  await openDmDialog(page);
  await choosePartner(page, displayName);

  await expect(page).toHaveURL(/\/channels\/[0-9a-f-]+$/);
  const firstDmUrl = page.url();
  await expect(page.getByRole("heading", { name: displayName, exact: true })).toBeVisible();
  await expect(page.getByRole("link", { name: displayName, exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "チャンネルの管理" })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "メンバー" })).toHaveCount(0);

  // 同じ相手をもう一度選ぶと、既存の DM が開く。
  await page.goto("/");
  await openDmDialog(page);
  await choosePartner(page, displayName);
  await expect(page).toHaveURL(firstDmUrl);

  // 退出すると一覧から消える。
  await page.getByRole("button", { name: "ダイレクトメッセージのメニュー" }).click();
  await page.getByRole("menuitem", { name: "退出", exact: true }).click();
  await expect(page.getByRole("alertdialog")).toBeVisible();
  await page.getByRole("alertdialog").getByRole("button", { name: "退出", exact: true }).click();
  await expect(page).toHaveURL(/\/$/);
  await expect(page.getByRole("link", { name: displayName, exact: true })).toHaveCount(0);

  // 再オープンは同じ DM に戻る。
  await openDmDialog(page);
  await choosePartner(page, displayName);
  await expect(page).toHaveURL(firstDmUrl);
});

test("モバイルで DM を開いて一覧へ戻れる", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  const suffix = Date.now();
  const displayName = `Mobile DM ${suffix}`;
  const userName = `mobile_dm_${suffix}`;
  await createPartner(displayName, userName);

  await page.goto("/");
  await hideDevtools(page);
  await expect(page.getByRole("heading", { name: "チャンネル", exact: true })).toBeVisible();

  await openDmDialog(page);
  await choosePartner(page, displayName);

  await expect(page.getByRole("heading", { name: displayName, exact: true })).toBeVisible();
  await page.getByRole("button", { name: "チャンネル一覧へ戻る" }).click();
  await expect(page.getByRole("link", { name: displayName, exact: true })).toBeVisible();
  await expect(page.getByPlaceholder("メッセージを送信")).toBeHidden();
});

test("グループ DM を作成してメッセージを送れる", async ({ page }) => {
  const suffix = Date.now();
  const first = `Group Alpha ${suffix}`;
  const second = `Group Beta ${suffix}`;
  await createPartner(first, `group_alpha_${suffix}`);
  await createPartner(second, `group_beta_${suffix}`);

  await page.goto("/");
  await hideDevtools(page);
  await openDmDialog(page);
  await selectPartner(page, first);
  await selectPartner(page, second);
  await page.getByRole("button", { name: "会話を始める", exact: true }).click();

  await expect(
    page.getByRole("heading", { name: `${first}, ${second}`, exact: true }),
  ).toBeVisible();
  const message = `hello group ${suffix}`;
  await page.getByPlaceholder("メッセージを送信").fill(message);
  await page.getByRole("button", { name: "送信", exact: true }).click();
  await expect(page.getByText(message)).toBeVisible();
});

test("作成中は Escape でダイアログを閉じられない", async ({ page }) => {
  const suffix = Date.now();
  const displayName = `Slow DM ${suffix}`;
  await createPartner(displayName, `slow_dm_${suffix}`);

  await page.goto("/");
  await hideDevtools(page);
  await page.route("**/api/dms", async (route) => {
    await new Promise((resolve) => setTimeout(resolve, 1500));
    await route.continue();
  });
  await openDmDialog(page);
  await selectPartner(page, displayName);
  await page.getByRole("button", { name: "会話を始める", exact: true }).click();
  await expect(page.getByRole("button", { name: "作成中…", exact: true })).toBeVisible();

  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog")).toBeVisible();
  await expect(page).toHaveURL(/\/$/);
  await expect(page).toHaveURL(/\/channels\/[0-9a-f-]+$/, { timeout: 15_000 });
});
