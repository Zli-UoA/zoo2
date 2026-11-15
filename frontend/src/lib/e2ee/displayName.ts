/**
 * 端末名（`display_name`）。本人の端末一覧（#362）にだけ出るので、ブラウザーと OS が分かれば足りる。
 * 例：`Chrome（macOS）`（docs/e2ee.md の例と同じ形）。
 */
export function deviceDisplayName(userAgent: string = navigator.userAgent): string {
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
  return os ? `${browser}（${os}）` : browser;
}
