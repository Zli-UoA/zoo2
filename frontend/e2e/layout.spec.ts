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
    Object.assign(viewport, { height: window.innerHeight, offsetTop: 0, scale: 1 });
    Object.defineProperty(window, "visualViewport", { value: viewport });
  });
  await page.goto("/e2e/fixtures/layout.html");
  const fullHeight = await page.evaluate(() => window.innerHeight);
  const shell = page.getByTestId("shell");
  await expect(shell).toHaveCSS("height", `${fullHeight}px`);
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
    Object.assign(window.visualViewport!, { height: window.innerHeight, offsetTop: 0 });
    window.visualViewport!.dispatchEvent(new Event("resize"));
  });
  await expect(shell).toHaveCSS("height", `${fullHeight}px`);
  await expect(shell).toHaveCSS("top", "0px");
  await page.evaluate(() => {
    Object.assign(window.visualViewport!, { height: 350, scale: 2 });
    window.visualViewport!.dispatchEvent(new Event("resize"));
  });
  await expect(shell).toHaveCSS("height", `${fullHeight}px`);
});

test("page bounce without keyboard does not move the shell", async ({ page }) => {
  await page.addInitScript(() => {
    // iOS のバウンスでは高さを変えずに offsetTop だけが動く。
    const viewport = new EventTarget();
    Object.assign(viewport, { height: window.innerHeight, offsetTop: 0, scale: 1 });
    Object.defineProperty(window, "visualViewport", { value: viewport });
  });
  await page.goto("/e2e/fixtures/layout.html");
  const shell = page.getByTestId("shell");
  await expect(page.locator("html")).toHaveCSS("overflow-y", "hidden");
  await expect(page.locator("body")).toHaveCSS("overflow-y", "hidden");
  await page.evaluate(() => {
    Object.assign(window.visualViewport!, { offsetTop: 40 });
    window.visualViewport!.dispatchEvent(new Event("scroll"));
  });
  await expect(shell).toHaveCSS("top", "0px");
});

async function scrollMessages(page: Page, to: "top" | "bottom") {
  const scroller = page.getByTestId("message-scroll");
  await scroller.evaluate((element, target) => {
    element.scrollTop = target === "top" ? 0 : element.scrollHeight;
    element.dispatchEvent(new Event("scroll"));
  }, to);
}

test("scrolling to the top loads older messages without moving the reader", async ({ page }) => {
  await page.goto("/e2e/fixtures/layout.html");
  await scrollMessages(page, "bottom");
  await expect(page.getByText(/^Message 39\b/)).toBeVisible();
  await page.waitForTimeout(500);
  await scrollMessages(page, "top");
  await expect(page.getByText(/^Message 1\b/)).toBeVisible();
  const scroller = page.getByTestId("message-scroll");
  const before = await scroller.evaluate((element) => ({
    top: element.scrollTop,
    height: element.scrollHeight,
  }));
  await page.getByTestId("prepend-older").click();
  await expect
    .poll(async () => {
      const after = await scroller.evaluate((element) => ({
        top: element.scrollTop,
        height: element.scrollHeight,
      }));
      return Math.abs(after.top - before.top - (after.height - before.height));
    })
    .toBeLessThanOrEqual(2);
  await expectNoOverlap(page);
});

test("new messages below do not move a scrolled-up reader", async ({ page }) => {
  await page.goto("/e2e/fixtures/layout.html");
  await expect(page.getByText(/^Message 39\b/)).toBeVisible();
  await page.waitForTimeout(500);
  await scrollMessages(page, "top");
  const row = page.getByText(/^Message 1\b/);
  await expect(row).toBeVisible();
  const before = await row.boundingBox();
  await page.getByTestId("append-new").click();
  await expect(page.getByRole("button", { name: "New messages" })).toBeVisible();
  const after = await row.boundingBox();
  expect(after).not.toBeNull();
  expect(before).not.toBeNull();
  expect(Math.abs(after!.y - before!.y)).toBeLessThanOrEqual(2);
});
