import { expect, test } from "@playwright/test";

const languages = [
  {
    locale: "ja",
    settings: "Zoo設定",
    languageField: "言語",
    browser: "ja-JP",
    label: "日本語",
    heading: "チャンネル",
    toggle: "言語を切り替える",
    create: "チャンネルを作成",
    name: "新しいチャンネル名",
    close: "閉じる",
  },
  {
    locale: "en",
    settings: "Zoo settings",
    languageField: "Language",
    browser: "en-US",
    label: "English",
    heading: "Channels",
    toggle: "Switch language",
    create: "Create a channel",
    name: "New channel name",
    close: "Close",
  },
  {
    locale: "zh-Hant",
    settings: "Zoo 設定",
    languageField: "語言",
    browser: "zh-TW",
    label: "繁體中文",
    heading: "頻道",
    toggle: "切換語言",
    create: "建立頻道",
    name: "新頻道名稱",
    close: "關閉",
  },
  {
    locale: "zh-Hans",
    settings: "Zoo 设置",
    languageField: "语言",
    browser: "zh-CN",
    label: "简体中文",
    heading: "频道",
    toggle: "切换语言",
    create: "创建频道",
    name: "新频道名称",
    close: "关闭",
  },
  {
    locale: "yue",
    settings: "Zoo 設定",
    languageField: "語言",
    browser: "yue-HK",
    label: "廣東話",
    heading: "頻道",
    toggle: "切換語言",
    create: "開新頻道",
    name: "新頻道名",
    close: "關閉",
  },
];

for (const language of languages) {
  test.describe(language.locale, () => {
    test.use({ locale: language.browser });
    test("detects the browser language and translates dialog labels", async ({ page }) => {
      const errors: string[] = [];
      page.on("pageerror", (error) => errors.push(error.message));
      await page.goto("/");
      await expect(page.locator("html")).toHaveAttribute("lang", language.locale);
      await expect(
        page.getByRole("heading", { name: language.heading, exact: true }),
      ).toBeVisible();
      await page.getByRole("button", { name: language.create, exact: true }).click();
      await expect(page.getByRole("dialog")).toBeVisible();
      await expect(page.getByRole("textbox", { name: language.name })).toBeVisible();
      await page.getByRole("button", { name: language.close, exact: true }).click();
      await expect(page.getByRole("dialog")).toBeHidden();
      expect(errors).toEqual([]);
    });
  });
}

test("switches all five languages, saves the selection and survives reload", async ({ page }) => {
  await page.goto("/settings");
  for (const language of languages) {
    const select = page
      .getByRole("combobox")
      .filter({ has: page.getByRole("option", { name: "日本語", exact: true }) });
    await select.selectOption(language.locale);
    await expect(page.locator("html")).toHaveAttribute("lang", language.locale);
    await expect(page.getByRole("heading", { name: language.settings, exact: true })).toBeVisible();
    await expect(
      page.getByRole("combobox", { name: language.languageField, exact: true }),
    ).toHaveValue(language.locale);
    expect(await page.evaluate(() => localStorage.getItem("zoo-locale"))).toBe(language.locale);
    await page.reload();
    await expect(page.locator("html")).toHaveAttribute("lang", language.locale);
    await expect(
      page.getByRole("combobox", { name: language.languageField, exact: true }),
    ).toHaveValue(language.locale);
  }
});

test("can switch language when storage reads and writes fail", async ({ page }) => {
  await page.addInitScript(() => {
    Storage.prototype.getItem = () => {
      throw new DOMException("Blocked", "SecurityError");
    };
    Storage.prototype.setItem = () => {
      throw new DOMException("Blocked", "QuotaExceededError");
    };
  });
  await page.goto("/settings");
  const select = page
    .getByRole("combobox")
    .filter({ has: page.getByRole("option", { name: "日本語", exact: true }) });
  await select.selectOption("yue");
  await expect(page.locator("html")).toHaveAttribute("lang", "yue");
  await expect(page.getByRole("heading", { name: "Zoo 設定", exact: true })).toBeVisible();
  await expect(page.getByRole("combobox", { name: "語言", exact: true })).toHaveValue("yue");
});

test("sets the saved language before hydration without displaying Japanese content", async ({
  page,
}) => {
  await page.addInitScript(() => localStorage.setItem("zoo-locale", "yue"));
  await page.route(/\/assets\/.*\.js(?:\?.*)?$/, (route) => route.abort());
  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("lang", "yue");
  await expect(page.locator("body")).not.toContainText("チャンネル");
  await expect(page.locator("body")).not.toContainText("ようこそ");
});
