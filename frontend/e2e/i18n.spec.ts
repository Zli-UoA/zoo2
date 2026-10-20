import { expect, test } from "@playwright/test";

const languages = [
  {
    locale: "ja",
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
  await page.goto("/");
  for (const language of languages) {
    await page
      .locator("nav button")
      .filter({ has: page.locator("svg.lucide-languages") })
      .click();
    await page.getByRole("menuitemradio", { name: language.label, exact: true }).click();
    await expect(page.locator("html")).toHaveAttribute("lang", language.locale);
    await expect(page.getByRole("heading", { name: language.heading, exact: true })).toBeVisible();
    expect(await page.evaluate(() => localStorage.getItem("zoo-locale"))).toBe(language.locale);
    await page.reload();
    await expect(page.locator("html")).toHaveAttribute("lang", language.locale);
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
  await page.goto("/");
  await page
    .locator("nav button")
    .filter({ has: page.locator("svg.lucide-languages") })
    .click();
  await page.getByRole("menuitemradio", { name: "廣東話", exact: true }).click();
  await expect(page.locator("html")).toHaveAttribute("lang", "yue");
  await expect(page.getByRole("heading", { name: "歡迎嚟到 Zoo" })).toBeVisible();
});
