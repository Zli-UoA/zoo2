import type { TFunction } from "i18next";

import { isStandaloneDisplay } from "../pwa/standalone";

/**
 * 端末名（`display_name`）。本人の端末一覧（#362）にだけ出るので、ブラウザーと OS と、アプリとして
 * 開いているかが分かれば足りる。登録したときの言語で作って送る。
 * 例：`Chrome（macOS）`、`Safari（iOS・アプリ）`（docs/e2ee.md の例と同じ形）。
 */
export function deviceDisplayName(
  t: TFunction,
  {
    userAgent = globalThis.navigator?.userAgent ?? "",
    // Safari では、タブとアプリ（PWA）で保存先が分かれて別の端末になるので、端末名で見分ける。
    standalone = isStandaloneDisplay(),
  }: { userAgent?: string; standalone?: boolean } = {},
): string {
  const browser = /Edg\//.test(userAgent)
    ? "Edge"
    : /OPR\//.test(userAgent)
      ? "Opera"
      : /Firefox\//.test(userAgent)
        ? "Firefox"
        : /Chrome\//.test(userAgent)
          ? "Chrome"
          : /Safari\//.test(userAgent)
            ? "Safari"
            : "Browser";
  const os = /Android/.test(userAgent)
    ? "Android"
    : /iPhone|iPad|iPod/.test(userAgent)
      ? "iOS"
      : /CrOS/.test(userAgent)
        ? "ChromeOS"
        : /Windows/.test(userAgent)
          ? "Windows"
          : /Mac OS X|Macintosh/.test(userAgent)
            ? "macOS"
            : /Linux/.test(userAgent)
              ? "Linux"
              : null;
  if (standalone) {
    return os
      ? t("e2ee.deviceName.appWithOs", { browser, os })
      : t("e2ee.deviceName.app", { browser });
  }
  return os ? t("e2ee.deviceName.withOs", { browser, os }) : browser;
}
