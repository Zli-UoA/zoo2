import { rawCopyMessage } from "./fixtures/message-content";
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
      const body = row.locator(".mfm-body");
      const box = await body.boundingBox();
      expect(box).not.toBeNull();
      const session = await page.context().newCDPSession(page);
      try {
        await session.send("Input.dispatchTouchEvent", {
          type: "touchStart",
          touchPoints: [{ x: box!.x + 10, y: box!.y + 10 }],
        });
        await expect(page.getByRole("menuitem", { name: "Edit", exact: true })).toBeVisible();
        await session.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
      } finally {
        await session.detach();
      }
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
  await page.getByRole("textbox", { name: "Write a message" }).focus();
  await page.evaluate(() => {
    Object.assign(window.visualViewport!, { height: 380, offsetTop: 35 });
    window.visualViewport!.dispatchEvent(new Event("resize"));
    window.visualViewport!.dispatchEvent(new Event("scroll"));
  });
  await expect(shell).toHaveCSS("height", "380px");
  await expect(shell).toHaveCSS("top", "35px");
  const composer = await page.getByRole("textbox", { name: "Write a message" }).boundingBox();
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

test("keyboard is detected even when innerHeight shrinks with it (iOS Safari)", async ({
  page,
}) => {
  await page.addInitScript(() => {
    const viewport = new EventTarget();
    Object.assign(viewport, { height: window.innerHeight, offsetTop: 0, scale: 1 });
    Object.defineProperty(window, "visualViewport", { value: viewport });
  });
  await page.goto("/e2e/fixtures/layout.html");
  const shell = page.getByTestId("shell");
  await page.getByRole("textbox", { name: "Write a message" }).focus();
  await page.evaluate(() => {
    // iOS Safari ではキーボード表示で innerHeight も visualViewport と同じ高さに縮む。
    Object.defineProperty(window, "innerHeight", { value: 380, configurable: true });
    Object.assign(window.visualViewport!, { height: 380, offsetTop: 35 });
    window.visualViewport!.dispatchEvent(new Event("resize"));
    window.visualViewport!.dispatchEvent(new Event("scroll"));
  });
  await expect(shell).toHaveCSS("height", "380px");
  await expect(shell).toHaveCSS("top", "35px");
  const composer = await page.getByRole("textbox", { name: "Write a message" }).boundingBox();
  expect(composer!.y + composer!.height).toBeLessThanOrEqual(415);
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
  await page.evaluate(() => document.fonts.ready);
  await scrollMessages(page, "top");
  const row = page.getByText(/^Message 1\b/);
  await expect(row).toBeVisible();
  await expectNoOverlap(page);
  for (let batch = 0; batch < 2; batch++) {
    const before = await row.boundingBox();
    expect(before).not.toBeNull();
    await page.getByTestId("prepend-older").click();
    await expect(row.locator("xpath=ancestor::*[@role='listitem']")).toHaveAttribute(
      "data-index",
      String(1 + (batch + 1) * 10),
    );
    await expectNoOverlap(page);
    // Wait for ResizeObserver measurements to commit without a fixed-duration sleep.
    await page.evaluate(
      () =>
        new Promise<void>((resolve) =>
          requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
        ),
    );
    // Virtual total height includes estimates for offscreen rows. Check the actual
    // message the reader sees instead of coupling its position to those estimates.
    await expect
      .poll(async () => {
        const after = await row.boundingBox();
        return after ? Math.abs(after.y - before!.y) : Infinity;
      })
      .toBeLessThanOrEqual(2);
    await expectNoOverlap(page);
  }
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

test("App actions open from the upper-right menu", async ({ page, isMobile }) => {
  await page.goto("/e2e/fixtures/layout.html");
  const row = page.locator('[data-index="37"]');
  await expect(row).toBeVisible();
  await expect(row.getByRole("button", { name: "Message app actions" })).toHaveCount(0);
  await page.evaluate(() => {
    window.addEventListener("zoo:message-actions", (event) => {
      document.documentElement.dataset.appMessageId = (event as CustomEvent<string>).detail;
    });
  });
  const more = row.getByRole("button", { name: "More actions" });
  await expect(row.locator("[data-message-toolbar]")).toHaveCSS("opacity", "0");
  if (isMobile) {
    const box = await row.locator(".mfm-body").boundingBox();
    expect(box).not.toBeNull();
    const session = await page.context().newCDPSession(page);
    try {
      await session.send("Input.dispatchTouchEvent", {
        type: "touchStart",
        touchPoints: [{ x: box!.x + 10, y: box!.y + 10 }],
      });
      await expect(page.getByRole("menuitem", { name: "Message app actions" })).toBeVisible();
      await session.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
    } finally {
      await session.detach();
    }
  } else {
    await row.hover();
    await expect(row.locator("[data-message-toolbar]")).toHaveCSS("opacity", "1");
    await more.click();
  }
  await page.getByRole("menuitem", { name: "Message app actions" }).click();
  await expect(page.locator("html")).toHaveAttribute("data-app-message-id", /.+/);
});

test("hover preserves a short grouped row and centers its timestamp", async ({
  page,
  isMobile,
}) => {
  test.skip(isMobile, "Hover layout applies to pointer devices");
  await page.goto("/e2e/fixtures/layout.html");
  const row = page.locator('[data-index="39"]');
  await expect(row).toBeVisible();
  const before = await row.boundingBox();
  await row.hover();
  await expect(row.locator("time")).toHaveCSS("opacity", "1");
  const after = await row.boundingBox();
  expect(after!.height).toBeCloseTo(before!.height, 1);
  expect(after!.y).toBeCloseTo(before!.y, 1);
  const time = await row.locator("time").boundingBox();
  const body = await row.locator(".mfm-body").boundingBox();
  expect(Math.abs(time!.y + time!.height / 2 - body!.y - body!.height / 2)).toBeLessThan(1);
  const toolbar = row.locator("[data-message-toolbar]");
  await expect(toolbar.getByRole("button", { name: "More actions" })).toBeVisible();
  await expect(toolbar.getByRole("button", { name: "Edit", exact: true })).toBeVisible();
});

test("copies the original text from the upper-right message menu", async ({ page, context }) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await page.goto("/e2e/fixtures/layout.html?raw-copy");
  const row = page.locator('[data-index="37"]');
  await expect(row).toBeVisible();
  await expect(row).not.toContainText("**bold**");
  const more = row.getByRole("button", { name: "More actions" });
  await more.focus();
  await more.click();
  await page.getByRole("menuitem", { name: "Copy original text", exact: true }).click();
  await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe(rawCopyMessage);
});
