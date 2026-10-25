import { expect, test } from "@playwright/test";

test.beforeEach(async ({ page }) => {
  await page.route("**/mfm-emojis.json", (route) =>
    route.fulfill({ json: { party: "/party.svg" } }),
  );
  await page.route("**/party.svg", (route) =>
    route.fulfill({
      contentType: "image/svg+xml",
      body: '<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24"><circle cx="12" cy="12" r="10" fill="red"/></svg>',
    }),
  );
  await page.goto("/e2e/fixtures/mfm.html");
});

test("animation parameters, emoji registry, math and formatting render in the browser", async ({
  page,
}) => {
  const spin = page.locator(".mfm-spin");
  await expect(spin).toHaveCSS("animation-name", "mfm-spin-x");
  await expect(spin).toHaveCSS("animation-direction", "reverse");
  await expect(spin).toHaveCSS("animation-delay", "-1s");
  await expect(spin).toHaveCSS("animation-duration", "2s");
  const image = page.getByRole("img", { name: ":party:" });
  await expect(image).toBeVisible();
  expect(await image.evaluate((el: HTMLImageElement) => el.complete && el.naturalWidth > 0)).toBe(
    true,
  );
  await expect(page.locator("#emoji")).toContainText("😄");
  await expect(page.locator(".katex")).toBeVisible();
  await page.evaluate(() => document.fonts.ready);
  expect(await page.evaluate(() => document.fonts.check("16px KaTeX_Main"))).toBe(true);
  await expect(page.locator("ruby rt")).toHaveText("かんじ");
  await expect(page.locator('#format [style*="border-color"]')).toHaveCSS("border-style", "dashed");
});

test("reduced motion stops all animations, including sparkle decorations", async ({ page }) => {
  await expect(page.locator(".mfm-spin")).toBeVisible();
  await page.emulateMedia({ reducedMotion: "reduce" });
  for (const el of await page.locator(".mfm-effect, .mfm-sparkles").all())
    await expect(el).toHaveCSS("animation-name", "none");
});

test("blur and clickable functions work with the keyboard", async ({ page }) => {
  const blur = page.locator(".mfm-blur");
  await expect(blur).toHaveCSS("filter", "blur(6px)");
  await blur.focus();
  await expect(blur).toHaveCSS("filter", "none");
  await page.getByRole("button", { name: "click me" }).focus();
  await page.keyboard.press("Enter");
  await expect(page.locator("#result")).toHaveText("test");
});
