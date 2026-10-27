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

test("hashtags link only to existing channels in the same tab", async ({ page }) => {
  const hashtags = page.locator("#hashtags");
  await expect(hashtags).toHaveText("#開発 #未登録 #削除済み");
  const links = hashtags.getByRole("link");
  await expect(links).toHaveCount(1);
  await expect(links).toHaveText("#開発");
  await expect(links).toHaveAttribute("href", "/channels/dev-id");
  await expect(links).not.toHaveAttribute("target", "_blank");
});

for (const [timezoneId, winter, summer] of [
  ["Asia/Tokyo", "1/1/1970, 9:00:00 AM", "7/1/2020, 9:00:00 AM"],
  ["America/New_York", "12/31/1969, 7:00:00 PM", "6/30/2020, 8:00:00 PM"],
]) {
  test.describe(timezoneId, () => {
    test.use({ timezoneId, locale: "en-US" });
    test("unixtime follows device timezone including daylight saving", async ({ page }) => {
      const time = page.locator("#winter-time time");
      await expect(time).toContainText(winter);
      await expect(page.locator("#summer-time time")).toContainText(summer);
      await expect(time).toHaveAttribute("datetime", "1970-01-01T00:00:00.000Z");
      expect(await time.getAttribute("title")).toBe(await time.textContent());
    });
  });
}

test("MFM suggestions filter and insert at the caret with a keyboard", async ({ page }) => {
  const input = page.getByRole("textbox");
  await input.fill("hello $[sp");
  const list = page.getByRole("listbox", { name: "MFM suggestions" });
  await expect(list.getByRole("option")).toHaveCount(2);
  await input.press("ArrowDown");
  await input.press("Enter");
  await expect(input).toHaveValue("hello $[sparkle ]");
  await expect(page.locator("#sent")).toBeEmpty();
  expect(await input.evaluate((el: HTMLTextAreaElement) => el.selectionStart)).toBe(
    "hello $[sparkle ".length,
  );
  await input.pressSequentially("🎉");
  await expect(input).toHaveValue("hello $[sparkle 🎉]");
  await expect(list).toHaveCount(0);
});

test("MFM suggestions support clicking and tapping and Escape", async ({ page, isMobile }) => {
  const input = page.getByRole("textbox");
  await input.fill("$je");
  const option = page.getByRole("option", { name: "$[jelly] Stretch and squash" });
  if (isMobile) await option.tap();
  else await option.click();
  await expect(input).toHaveValue("$[jelly ]");
  await expect(input).toBeFocused();
  expect(await input.evaluate((el: HTMLTextAreaElement) => el.selectionStart)).toBe(
    "$[jelly ".length,
  );
  await input.fill("$");
  await expect(page.getByRole("listbox")).toBeVisible();
  await input.press("Escape");
  await expect(page.getByRole("listbox")).toHaveCount(0);
  await expect(input).toHaveValue("$");
});

test("MFM and Japanese mentions remain visible while IME owns the keyboard", async ({ page }) => {
  const input = page.getByRole("textbox");
  for (const [token, label, result] of [
    ["$je", "$[jelly]", "$[jelly ]"],
    ["@太", "@taro", "@taro "],
  ]) {
    await input.focus();
    await input.evaluate((el: HTMLTextAreaElement, value) => {
      el.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(el, value);
      el.setSelectionRange(0, value.length);
      el.dispatchEvent(
        new InputEvent("input", {
          bubbles: true,
          data: value,
          inputType: "insertCompositionText",
          isComposing: true,
        }),
      );
    }, token);
    await expect(page.getByRole("listbox")).toContainText(label);
    for (const key of ["ArrowDown", "Tab", "Escape", "Enter"]) {
      const prevented = await input.evaluate((el, key) => {
        const event = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true });
        el.dispatchEvent(event);
        return event.defaultPrevented;
      }, key);
      expect(prevented).toBe(false);
      await expect(input).toHaveValue(token);
      await expect(page.getByRole("listbox")).toBeVisible();
    }
    await expect(page.locator("#sent")).toBeEmpty();
    await input.evaluate((el: HTMLTextAreaElement) => {
      el.setSelectionRange(el.value.length, el.value.length);
      el.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true }));
      el.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", keyCode: 229, bubbles: true }));
    });
    await expect(input).toHaveValue(token);
    await input.press("Enter");
    await expect(input).toHaveValue(result);
  }
});

test("transformed message paint and hit targets stay inside the body", async ({ page }) => {
  const body = page.locator("#transformed-row .mfm-body");
  await expect(body).toBeVisible();
  await page.evaluate(() => document.fonts.ready);
  const section = page.locator("#containment");
  expect(await section.evaluate((el) => el.scrollWidth)).toBe(
    await section.evaluate((el) => el.clientWidth),
  );
  for (const id of ["previous-row", "next-row"]) {
    const neighbor = page.locator(`#${id}`);
    await neighbor.scrollIntoViewIfNeeded();
    await page.mouse.move(0, 0);
    const box = (await neighbor.boundingBox())!;
    // Inset the capture to exclude the fractional pixel shared with the next row.
    const clip = { x: box.x + 2, y: box.y + 2, width: box.width - 4, height: box.height - 4 };
    const painted = await page.screenshot({ clip });
    await body.evaluate((el) => {
      el.style.visibility = "hidden";
    });
    const withoutMessage = await page.screenshot({ clip });
    const samePixels = await page.evaluate(
      async ({ painted, hidden }) => {
        const pixels = async (base64: string) => {
          const bytes = Uint8Array.from(atob(base64), (char) => char.charCodeAt(0));
          const bitmap = await createImageBitmap(new Blob([bytes], { type: "image/png" }));
          const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
          const context = canvas.getContext("2d")!;
          context.drawImage(bitmap, 0, 0);
          bitmap.close();
          return context.getImageData(0, 0, canvas.width, canvas.height).data;
        };
        const [a, b] = await Promise.all([pixels(painted), pixels(hidden)]);
        return a.length === b.length && a.every((value, index) => value === b[index]);
      },
      { painted: painted.toString("base64"), hidden: withoutMessage.toString("base64") },
    );
    expect(samePixels).toBe(true);
    await body.evaluate((el) => {
      el.style.visibility = "";
    });
    await neighbor.click();
    await expect(neighbor).toHaveText(id === "previous-row" ? "clicked previous" : "clicked next");
  }
});
