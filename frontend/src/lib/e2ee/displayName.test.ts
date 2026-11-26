import { describe, expect, it } from "vite-plus/test";

import { createI18n } from "../i18n";
import { deviceDisplayName } from "./displayName";

const SAFARI_MAC =
  "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15";
const SAFARI_IOS =
  "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1";
const CHROME_LINUX =
  "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

describe("deviceDisplayName", () => {
  const ja = createI18n("ja").t;
  const en = createI18n("en").t;

  it("タブはブラウザーと OS", () => {
    expect(deviceDisplayName(ja, { userAgent: SAFARI_MAC, standalone: false })).toBe(
      "Safari（macOS）",
    );
    expect(deviceDisplayName(en, { userAgent: CHROME_LINUX, standalone: false })).toBe(
      "Chrome (Linux)",
    );
  });

  it("ホーム画面や Dock のアプリ（PWA）は、アプリと分かる語を付ける", () => {
    expect(deviceDisplayName(ja, { userAgent: SAFARI_IOS, standalone: true })).toBe(
      "Safari（iOS・アプリ）",
    );
    expect(deviceDisplayName(en, { userAgent: SAFARI_MAC, standalone: true })).toBe(
      "Safari (macOS, app)",
    );
    expect(deviceDisplayName(ja, { userAgent: "Unknown", standalone: true })).toBe(
      "Browser（アプリ）",
    );
  });
});
