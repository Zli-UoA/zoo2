import { test, expect } from "@playwright/test";
test("native Compose starts under deployment CSP and reloads cached messages offline", async ({
  page,
  context,
  request,
}) => {
  await request.post("https://127.0.0.1:4189/__reset");
  const violations: string[] = [];
  await page.exposeFunction("recordCspViolation", (directive: string) =>
    violations.push(directive),
  );
  await page.addInitScript(() =>
    document.addEventListener("securitypolicyviolation", (event) => {
      void (
        window as unknown as { recordCspViolation: (directive: string) => Promise<void> }
      ).recordCspViolation(event.violatedDirective);
    }),
  );
  await page.goto("/channels/c1?frontend=compose");
  await expect(page.getByText("こんにちは Parity", { exact: false })).toBeVisible({
    timeout: 90_000,
  });
  await expect
    .poll(
      () =>
        page.evaluate(async () => {
          const keys = await Promise.all(
            (await caches.keys()).map(async (name) =>
              (await (await caches.open(name)).keys()).map((r) => r.url),
            ),
          );
          const assets = keys.flat();
          const loadedScripts = performance
            .getEntriesByType("resource")
            .map((entry) => new URL(entry.name))
            .filter((url) => url.origin === location.origin && url.pathname.startsWith("/assets/"));
          return (
            loadedScripts.every(
              (url) =>
                assets.includes(url.href) ||
                assets.some((asset) => new URL(asset).pathname === url.pathname),
            ) &&
            assets.some((u) => u.endsWith("/zoo-compose.js")) &&
            assets.filter((u) => u.endsWith(".wasm")).length >= 2 &&
            [
              "Geist.ttf",
              "NotoSansJPRegular.otf",
              "NotoSansJPBold.otf",
              "NotoColorEmoji.ttf",
              "LiberationMono.ttf",
              "KaTeXMain.ttf",
              "KaTeXMath.ttf",
            ].every((font) => assets.some((u) => u.endsWith(`/font/${font}`)))
          );
        }),
      { timeout: 45_000 },
    )
    .toBe(true)
    .catch(async (error) => {
      console.log(
        "Compose cache state",
        await page.evaluate(async () => ({
          controller: navigator.serviceWorker.controller?.scriptURL,
          loaded: performance
            .getEntriesByType("resource")
            .map((e) => e.name)
            .filter((u) => u.includes("/assets/")),
          assets: (
            await Promise.all(
              (await caches.keys()).map(async (name) =>
                (await (await caches.open(name)).keys()).map((r) => r.url),
              ),
            )
          )
            .flat()
            .filter((url) => url.includes("/compose/") || url.includes("/assets/")),
        })),
      );
      throw error;
    });
  expect(violations).toEqual([]);
  await context.setOffline(true);
  await page.reload({ waitUntil: "domcontentloaded" });
  await expect(page.getByText("こんにちは Parity", { exact: false })).toBeVisible({
    timeout: 45_000,
  });
  await expect(page.getByText("オフラインです", { exact: false })).toBeVisible();
  expect(violations).toEqual([]);
});
