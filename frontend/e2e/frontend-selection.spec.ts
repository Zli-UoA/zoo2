import { expect, test } from "@playwright/test";

for (const query of ["", "?frontend=compose"]) {
  test(`retired Compose selection opens React ${query || "from storage"}`, async ({ page }) => {
    const composeRequests: string[] = [];
    page.on("request", (request) => {
      if (new URL(request.url()).pathname.startsWith("/compose/"))
        composeRequests.push(request.url());
    });
    await page.addInitScript(() => {
      localStorage.setItem("zoo-frontend", "compose");
      localStorage.setItem("zoo-locale", "ja");
    });
    await page.goto(`/channels/c1${query}`);
    await expect(page.getByText("こんにちは Parity", { exact: false })).toBeVisible({
      timeout: 90_000,
    });
    await expect(page.locator("[data-frontend-switch]")).toHaveCount(0);
    await expect(page.locator("[data-compose-shell]")).toHaveCount(0);
    await page.reload();
    await expect(page.getByText("こんにちは Parity", { exact: false })).toBeVisible({
      timeout: 90_000,
    });
    expect(composeRequests).toEqual([]);
  });
}
