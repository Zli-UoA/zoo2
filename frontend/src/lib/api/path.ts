const HEX = "[0-9a-f]";
const HYPHENATED = `${HEX}{8}-${HEX}{4}-${HEX}{4}-${HEX}{4}-${HEX}{12}`;
// バックエンドの `Uuid::parse_str` が受け付ける 4 つの形（ハイフン付き、ハイフンなし、
// `{...}`、`urn:uuid:...`）に合わせる。
const UUID_PATTERN = new RegExp(
  `^(?:${HYPHENATED}|${HEX}{32}|\\{${HYPHENATED}\\}|urn:uuid:${HYPHENATED})$`,
  "i",
);

/** ルートのパラメーターなどが UUID の形かどうかを返す。 */
export function isUuid(value: unknown): value is string {
  return typeof value === "string" && UUID_PATTERN.test(value);
}

/**
 * API のパスを組み立てるタグ付きテンプレート。埋め込んだ値はパスの 1 区間として
 * `encodeURIComponent` でエンコードするので、`/`、`?`、`#`、`..` を含んでも
 * 他のパスやクエリには解釈されない。
 *
 * 例: `apiPath\`/api/channels/${channelId}/view\``
 */
export function apiPath(strings: TemplateStringsArray, ...values: unknown[]): string {
  let path = strings[0] ?? "";
  values.forEach((value, index) => {
    path += encodeURIComponent(String(value)) + (strings[index + 1] ?? "");
  });
  return path;
}

/** `createApiClient` が拒否するパスに当たったときに投げる。 */
export class UnsafeApiPathError extends Error {
  constructor(path: string) {
    super(`Refusing to send a request to an unsafe API path: ${path}`);
    this.name = "UnsafeApiPathError";
  }
}

/**
 * パスのうちクエリより前の部分に、正規化すると別のパスになる文字列や区切り文字が
 * 含まれていないかを確認する。`/` 始まりでない、`\`、`#`、`//`、エンコードされた
 * `\`、`.` / `..` の区間のいずれかがあれば拒否する。
 */
export function assertSafeApiPath(path: string): void {
  const queryStart = path.indexOf("?");
  const pathname = queryStart === -1 ? path : path.slice(0, queryStart);
  const unsafe =
    !pathname.startsWith("/") ||
    pathname.startsWith("//") ||
    pathname.includes("//") ||
    pathname.includes("\\") ||
    /[\t\n\r]/.test(path) ||
    pathname.includes("#") ||
    path.includes("#") ||
    /%5c/i.test(pathname) ||
    pathname.split("/").some((segment) => {
      const decoded = segment.replace(/%2e/gi, ".");
      return decoded === "." || decoded === "..";
    });
  if (unsafe) {
    throw new UnsafeApiPathError(path);
  }
}
