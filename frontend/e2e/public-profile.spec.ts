import { test, expect } from "@playwright/test";
import { publicProfile, longPublicProfile } from "../.storybook/public-profile-fixtures";

for (const width of [1280, 390]) {
  test(`anonymous public profile at ${width}px`, async ({ page }, info) => {
    const implementation = info.project.name;
    await page.setViewportSize({ width, height: 1000 });
    await page.addInitScript(() => localStorage.setItem("zoo-locale", "ja"));
    const unexpected: string[] = [];
    const authenticated: string[] = [];
    await page.route("**/api/**", async (route) => {
      const request = route.request();
      const url = new URL(request.url());
      if (!url.pathname.startsWith("/api/")) return route.continue();
      if (request.headers().authorization || request.headers()["x-dev-user"])
        authenticated.push(url.pathname);
      if (!url.pathname.endsWith("/api/activitypub/users/alice/profile")) {
        unexpected.push(url.pathname);
        return route.fulfill({ status: 500, body: "Unexpected private request" });
      }
      return route.fulfill({
        json: url.searchParams.has("before")
          ? { ...publicProfile, posts: [publicProfile.posts[1]], next_cursor: null }
          : publicProfile,
      });
    });
    await page.goto(`/public/users/alice?frontend=${implementation}`);
    await expect(page.getByRole("heading", { level: 1, name: "Alice / アリス" })).toBeVisible({
      timeout: 90_000,
    });
    await expect(page.getByText("公開の投稿", { exact: true })).toBeVisible();
    const address = page.getByRole("textbox", { name: "アカウントのアドレス", exact: true });
    await expect(address).toBeVisible();
    if (implementation === "compose") await expect(address).toHaveText("@alice@zoo.example");
    else await expect(address).toHaveValue("@alice@zoo.example");
    await page.screenshot({ path: info.outputPath("public-profile.png"), fullPage: true });
    await expect(
      page.getByRole("link", { name: "Zooでプロフィールを開く", exact: true }),
    ).toHaveAttribute("href", "/home/users/alice");
    await expect(page.getByRole("link", { name: "過去の投稿", exact: true })).toBeVisible();
    if (implementation === "compose") {
      // Canvas content scrolls through native pointer events rather than DOM scrollIntoView.
      const viewport = page.viewportSize()!;
      await page.mouse.move(viewport.width / 2, viewport.height - 100);
      await page.mouse.wheel(0, 1800);
      await expect
        .poll(async () => {
          const bounds = await page
            .getByRole("link", { name: "過去の投稿", exact: true })
            .boundingBox();
          return bounds !== null && bounds.y >= 0 && bounds.y + bounds.height <= viewport.height;
        })
        .toBe(true);
    }

    await page
      .getByRole("link", { name: "過去の投稿", exact: true })
      .click({ force: implementation === "compose" });
    await expect(page.getByRole("link", { name: "最新の投稿", exact: true })).toBeVisible();
    await expect(page.getByRole("link", { name: "過去の投稿", exact: true })).toHaveCount(0);
    expect(unexpected).toEqual([]);
    expect(authenticated).toEqual([]);
    expect(page.url()).toContain("/public/users/alice?before=p1");
  });
}

test("failed anonymous request can be retried", async ({ page }, info) => {
  await page.addInitScript(() => localStorage.setItem("zoo-locale", "ja"));
  let requests = 0;
  let failing = true;
  await page.route("**/api/activitypub/users/alice/profile*", (route) => {
    requests++;
    return failing ? route.fulfill({ status: 500 }) : route.fulfill({ json: publicProfile });
  });
  await page.goto(`/public/users/alice?frontend=${info.project.name}`);
  await expect(page.getByRole("alert")).toBeVisible({ timeout: 90_000 });
  const beforeRetry = requests;
  failing = false;
  await page
    .getByRole("button", { name: "更新", exact: true })
    .click({ force: info.project.name === "compose" });
  await expect(page.getByRole("heading", { level: 1, name: "Alice / アリス" })).toBeVisible();
  expect(requests).toBe(beforeRetry + 1);
});

test("long names and posts fit a narrow screen in both themes", async ({ page }, info) => {
  await page.setViewportSize({ width: 390, height: 950 });
  await page.addInitScript(() => localStorage.setItem("zoo-locale", "ja"));
  await page.route("**/api/activitypub/users/alice/profile*", (route) =>
    route.fulfill({ json: longPublicProfile }),
  );
  for (const colorScheme of ["light", "dark"] as const) {
    await page.emulateMedia({ colorScheme });
    await page.goto(`/public/users/alice?frontend=${info.project.name}`);
    await expect(page.getByRole("heading", { level: 1 })).toBeVisible({ timeout: 90_000 });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
      true,
    );
    await expect(page.getByText("公開の投稿", { exact: true })).toBeVisible();
  }
});
