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
  // Asset caching and data persistence finish independently. Verify the durable
  // message snapshot before disconnecting and reloading the page.
  await expect
    .poll(
      () =>
        page.evaluate(async () => {
          const bridge = window.__zooCompose;
          if (!bridge) return null;
          const config = JSON.parse(bridge.config) as Record<string, string>;
          const environment = JSON.parse(config.environment) as { persistenceStatus: string };
          if (environment.persistenceStatus !== "ready") return null;
          const key = `zoo-compose:v1:${config.apiBaseUrl}:${config.shapeProxyPath}:${config.shapeMode}:${config.electricShapeUrl}:${config.userId}:messages:[channel_id=c1]`;
          return bridge.cache.read(key);
        }),
      { timeout: 45_000, message: "The message snapshot must be stored before going offline" },
    )
    .toContain("こんにちは Parity");
  expect(violations).toEqual([]);
  await context.setOffline(true);
  await page.reload({ waitUntil: "domcontentloaded" });
  await expect(page.getByText("こんにちは Parity", { exact: false })).toBeVisible({
    timeout: 45_000,
  });
  await expect(page.getByText("オフラインです", { exact: false })).toBeVisible();
  expect(violations).toEqual([]);
});
