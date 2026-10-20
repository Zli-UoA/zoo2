import { expect, test } from "@playwright/test";

interface CspViolation {
  directive: string;
  blockedURI: string;
}

/**
 * nginx が返す CSP の下で、SPA シェルのインライン script が実行され、
 * CSP 違反が発生しないことを確認する。
 *
 * - テーマ初期化 script が動けば `<html>` の colorScheme が light / dark になる。
 * - `securitypolicyviolation` を拾い、ハッシュのずれや img-src 等の不足を検出する。
 */
test("本番 CSP 下でインライン script が実行され、違反が出ない", async ({ page }) => {
  await page.addInitScript(() => {
    const state: { violations: CspViolation[] } = { violations: [] };
    Object.assign(window, { __cspState: state });
    document.addEventListener("securitypolicyviolation", (event) => {
      state.violations.push({
        directive: event.violatedDirective,
        blockedURI: event.blockedURI,
      });
    });
  });

  const response = await page.goto("/");
  expect(response?.status()).toBe(200);

  const html = page.locator("html");
  await expect(html).toHaveAttribute("lang", /.+/);

  const state = await page.evaluate(() => {
    const scoped = window as typeof window & { __cspState?: { violations: CspViolation[] } };
    return {
      colorScheme: document.documentElement.style.colorScheme,
      violations: scoped.__cspState?.violations ?? [],
    };
  });

  expect(["light", "dark"]).toContain(state.colorScheme);
  expect(state.violations).toEqual([]);
});
