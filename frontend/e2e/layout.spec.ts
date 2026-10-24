import { expect, test, type Page } from "@playwright/test";

async function expectNoOverlap(page: Page) {
  await expect
    .poll(() =>
      page.getByRole("listitem").evaluateAll((rows) => {
        const boxes = rows.map((row) => row.getBoundingClientRect());
        return (
          boxes.length > 1 &&
          boxes.every((box, i) => i === 0 || Math.abs(box.top - boxes[i - 1].bottom) <= 1)
        );
      }),
    )
    .toBe(true);
}

test("editing, saving and cancelling preserve measured row positions", async ({
  page,
  isMobile,
}) => {
  await page.goto("/e2e/fixtures/layout.html");
  const row = page.locator('[data-index="37"]');
  await expect(row).toBeVisible();
  await expectNoOverlap(page);
  const edit = async () => {
    if (isMobile) {
      await row.getByRole("button", { name: "More actions" }).click();
      await page.getByRole("menuitem", { name: "Edit", exact: true }).click();
    } else {
      await row.hover();
      await row.getByRole("button", { name: "Edit", exact: true }).click();
    }
  };
  await edit();
  const input = page.getByRole("textbox", { name: "Edit message" });
  await input.fill("Edited message\n" + "Long line\n".repeat(40));
  await expectNoOverlap(page);
  expect(await input.evaluate((el) => el.scrollHeight > el.clientHeight)).toBe(true);
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(row).toContainText("Edited message");
  await expectNoOverlap(page);
  await edit();
  await input.fill("Short");
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expectNoOverlap(page);
  await edit();
  await input.evaluate((el) =>
    el.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "Escape",
        isComposing: true,
        bubbles: true,
      }),
    ),
  );
  await expect(input).toBeVisible();
  await page.getByRole("button", { name: "Cancel", exact: true }).click();
  await expectNoOverlap(page);
});

test("keyboard viewport resize and pan keep the composer visible and restore on close", async ({
  page,
}) => {
  await page.addInitScript(() => {
    // OSキーボードは自動操作できないため VisualViewport のイベントを再現する。
    const viewport = new EventTarget();
    Object.assign(viewport, { height: 700, offsetTop: 0, scale: 1 });
    Object.defineProperty(window, "visualViewport", { value: viewport });
  });
  await page.goto("/e2e/fixtures/layout.html");
  const shell = page.getByTestId("shell");
  await expect(shell).toHaveCSS("height", "700px");
  await page.getByRole("combobox", { name: "Write a message" }).focus();
  await page.evaluate(() => {
    Object.assign(window.visualViewport!, { height: 380, offsetTop: 35 });
    window.visualViewport!.dispatchEvent(new Event("resize"));
    window.visualViewport!.dispatchEvent(new Event("scroll"));
  });
  await expect(shell).toHaveCSS("height", "380px");
  await expect(shell).toHaveCSS("top", "35px");
  const composer = await page.getByRole("combobox", { name: "Write a message" }).boundingBox();
  expect(composer!.y + composer!.height).toBeLessThanOrEqual(415);
  await expectNoOverlap(page);
  await page.evaluate(() => {
    Object.assign(window.visualViewport!, { height: 700, offsetTop: 0 });
    window.visualViewport!.dispatchEvent(new Event("resize"));
  });
  await expect(shell).toHaveCSS("height", "700px");
  await expect(shell).toHaveCSS("top", "0px");
  await page.evaluate(() => {
    Object.assign(window.visualViewport!, { height: 350, scale: 2 });
    window.visualViewport!.dispatchEvent(new Event("resize"));
  });
  await expect(shell).toHaveCSS("height", "700px");
});
