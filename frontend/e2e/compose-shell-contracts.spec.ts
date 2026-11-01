import { expect, test, type Page } from "@playwright/test";
import {
  fillControl as fill,
  expectControlValue as value,
  type Implementation,
} from "./compose-control-input";

async function openStory(page: Page, id: string, implementation: Implementation) {
  await page.goto(
    `/iframe.html?id=${id}&viewMode=story&globals=implementation:${implementation};locale:ja;theme:light`,
  );
  if (implementation === "kotlin")
    await expect(page.locator('[data-compose-catalog-state="ready"]')).toBeVisible({
      timeout: 90_000,
    });
  await page.waitForFunction(
    () =>
      (window as typeof window & { __STORYBOOK_PREVIEW__?: { currentRender?: { phase?: string } } })
        .__STORYBOOK_PREVIEW__?.currentRender?.phase === "finished",
    undefined,
    { timeout: 90_000, polling: 50 },
  );
  await expect(page.locator(".sb-errordisplay")).toBeHidden();
  return implementation === "kotlin"
    ? page.frameLocator('iframe[title="Kotlin component preview"]')
    : page;
}
async function closeCalls(page: Page, id: string): Promise<unknown[][]> {
  return page.evaluate(async (storyId) => {
    const preview = (
      window as typeof window & {
        __STORYBOOK_PREVIEW__: {
          storyStore: { loadStory(input: { storyId: string }): Promise<unknown> };
          getStoryContext(story: unknown): {
            args: { onOpenChange: { mock: { calls: unknown[][] } } };
          };
        };
      }
    ).__STORYBOOK_PREVIEW__;
    return preview.getStoryContext(await preview.storyStore.loadStory({ storyId })).args
      .onOpenChange.mock.calls;
  }, id);
}
async function setOpen(page: Page, storyId: string, open: boolean) {
  await page.evaluate(
    ({ storyId, open }) => {
      (
        window as typeof window & {
          __STORYBOOK_ADDONS_CHANNEL__: {
            emit(event: string, payload: unknown): void;
          };
        }
      ).__STORYBOOK_ADDONS_CHANNEL__.emit("updateStoryArgs", { storyId, updatedArgs: { open } });
    },
    { storyId, open },
  );
}
for (const implementation of ["react", "kotlin"] as const) {
  test(`${implementation}: controlled open props hide and reopen without resetting edits`, async ({
    page,
  }) => {
    const id = "features-channels-create--default";
    const root = await openStory(page, id, implementation);
    const dialog = root.getByRole("dialog", { name: "チャンネルを作成", exact: true });
    const name = root.getByRole("textbox", { name: "新しいチャンネル名", exact: true });
    await fill(page, name, "preserved-by-props", implementation);
    await setOpen(page, id, false);
    await expect(dialog).toBeHidden();
    expect(await closeCalls(page, id)).toEqual([]);
    await setOpen(page, id, true);
    await expect(dialog).toBeVisible();
    await value(name, "preserved-by-props", implementation);
    expect(await closeCalls(page, id)).toEqual([]);
  });
  test(`${implementation}: controlled create cancel resets fields and calls onOpenChange(false)`, async ({
    page,
  }) => {
    const id = "features-channels-create--default";
    const root = await openStory(page, id, implementation);
    const dialog = root.getByRole("dialog", { name: "チャンネルを作成", exact: true });
    const name = root.getByRole("textbox", { name: "新しいチャンネル名", exact: true });
    const description = root.getByRole("textbox", { name: "説明（任意）", exact: true });
    await fill(page, name, "contract-channel", implementation);
    await fill(page, description, "contract description", implementation);
    const privacy = root.getByRole("checkbox", { name: "非公開にする", exact: true });
    await privacy.click({ force: implementation === "kotlin" });
    await expect(privacy).toBeChecked();
    await dialog
      .getByRole("button", { name: "キャンセル", exact: true })
      .click({ force: implementation === "kotlin" });
    await expect.poll(() => closeCalls(page, id)).toEqual([[false]]);
    await expect(dialog).toBeVisible();
    await value(name, "", implementation);
    await value(description, "", implementation);
    await expect(privacy).not.toBeChecked();
  });
  test(`${implementation}: controlled edit cancel preserves edits`, async ({ page }) => {
    const id = "features-channels-edit--default";
    const root = await openStory(page, id, implementation);
    const dialog = root.getByRole("dialog", { name: "チャンネルを編集", exact: true });
    const name = root.getByRole("textbox", { name: "新しいチャンネル名", exact: true });
    const description = root.getByRole("textbox", { name: "説明（任意）", exact: true });
    await fill(page, name, "contract-renamed", implementation);
    await fill(page, description, "edited description", implementation);
    await dialog
      .getByRole("button", { name: "キャンセル", exact: true })
      .click({ force: implementation === "kotlin" });
    await expect.poll(() => closeCalls(page, id)).toEqual([[false]]);
    await expect(dialog).toBeVisible();
    await value(name, "contract-renamed", implementation);
    await value(description, "edited description", implementation);
  });
  test(`${implementation}: controlled DM cancel resets selected users`, async ({ page }) => {
    const id = "features-dms-create--default";
    const root = await openStory(page, id, implementation);
    const dialog = root.getByRole("dialog", { name: "新しいダイレクトメッセージ", exact: true });
    const search = root.getByRole("combobox");
    await fill(page, search, "Bob", implementation);
    await root.getByRole("option", { name: /^Bob/ }).click({ force: implementation === "kotlin" });
    const selected = root.getByRole("button", { name: /Bob.*削除|Bob.*外す/ });
    await expect(selected).toBeVisible();
    await dialog
      .getByRole("button", { name: "キャンセル", exact: true })
      .click({ force: implementation === "kotlin" });
    await expect.poll(() => closeCalls(page, id)).toEqual([[false]]);
    await expect(dialog).toBeVisible();
    await expect(selected).toBeHidden();
    await value(search, "", implementation);
    await expect(root.getByText("0 / 9 人を選択", { exact: true })).toBeVisible();
  });
  test(`${implementation}: discovery clear and Escape preserve the controlled dialog`, async ({
    page,
  }) => {
    const id = "features-channels-discovery--default";
    const root = await openStory(page, id, implementation);
    const dialog = root.getByRole("dialog", { name: "チャンネルを探す", exact: true });
    const search = root.getByRole("searchbox");
    await fill(page, search, "zzzz", implementation);
    await root
      .getByRole("button", { name: "検索をクリア", exact: true })
      .click({ force: implementation === "kotlin" });
    await value(search, "", implementation);
    await fill(page, search, "zzzz", implementation);
    await page.keyboard.press("Escape");
    await value(search, "", implementation);
    expect(await closeCalls(page, id)).toEqual([]);
    await expect(dialog).toBeVisible();
    await page.keyboard.press("Escape");
    await expect.poll(() => closeCalls(page, id)).toEqual([[false]]);
    await expect(dialog).toBeVisible();
  });
}
