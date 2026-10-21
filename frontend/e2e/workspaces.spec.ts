import { expect, test } from "@playwright/test";

test.use({ locale: "ja-JP" });

test("ワークスペース切替でチャンネル・メッセージ・下書きを分離する", async ({ page }) => {
  // The development-only floating launcher overlaps the composer send button.
  await page.addInitScript(() => {
    document.addEventListener("DOMContentLoaded", () => {
      const style = document.createElement("style");
      style.textContent = 'button[aria-label="Open TanStack Devtools"]{display:none !important}';
      document.head.append(style);
    });
  });
  const suffix = Date.now();
  const channelName = `shared-name-${suffix}`;
  const workspaceA = `Workspace A ${suffix}`;
  const workspaceB = `Workspace B ${suffix}`;
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "チャンネル", exact: true })).toBeVisible();

  for (const name of [workspaceA, workspaceB]) {
    await page.getByRole("button", { name: "新規作成", exact: true }).click();
    await page.getByRole("textbox", { name: "ワークスペース名", exact: true }).fill(name);
    await page.getByRole("button", { name: "作成", exact: true }).click();
    await expect(
      page.getByRole("combobox", { name: "ワークスペース", exact: true }).locator("option:checked"),
    ).toHaveText(name);
    await expect(page.getByRole("heading", { name: "チャンネル", exact: true })).toBeVisible();
    await expect(page.getByRole("link", { name: new RegExp(channelName) })).toHaveCount(0);
    await page.getByRole("button", { name: "チャンネルを作成", exact: true }).click();
    await page.getByPlaceholder("新しいチャンネル名").fill(channelName);
    await page.getByRole("button", { name: "チャンネル作成", exact: true }).click();
    await page.getByRole("link", { name: new RegExp(channelName) }).click();
    await page
      .getByRole("textbox", { name: "メッセージを送信", exact: true })
      .fill(`Message in ${name}`);
    await page.getByRole("button", { name: "送信", exact: true }).click();
    await expect(page.getByText(`Message in ${name}`, { exact: true })).toBeVisible();
    if (name === workspaceA)
      await page.getByRole("textbox", { name: "メッセージを送信", exact: true }).fill("Draft in A");
    else await expect(page.getByText(`Message in ${workspaceA}`, { exact: true })).toHaveCount(0);
  }

  const selector = page.getByRole("combobox", { name: "ワークスペース", exact: true });
  await selector.selectOption({ label: workspaceA });
  await expect(page).toHaveURL("/");
  await page.getByRole("link", { name: new RegExp(channelName) }).click();
  await expect(page.getByText(`Message in ${workspaceA}`, { exact: true })).toBeVisible();
  await expect(page.getByText(`Message in ${workspaceB}`, { exact: true })).toHaveCount(0);
  await expect(page.getByRole("textbox", { name: "メッセージを送信", exact: true })).toHaveValue(
    "",
  );
});
