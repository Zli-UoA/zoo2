/**
 * AuthZ のプロフィール設定画面の URL。issuer（例: https://account.zli.works/api/auth）と
 * 同じ origin の /settings/profile を指す。`return_to` を付けると、AuthZ は保存後にそこへ戻す
 * （登録済みの連携先 origin だけが有効）。
 */
export function accountProfileUrl(issuer: string, returnTo: string | null): string | null {
  try {
    const url = new URL("/settings/profile", new URL(issuer).origin);
    if (returnTo) url.searchParams.set("return_to", returnTo);
    return url.href;
  } catch {
    return null;
  }
}

/**
 * AuthZ のセッション（`/settings/security`）とサービス接続（`/settings/services`）の画面の URL。
 * 失効とリセットで、セッションとトークンを利用者に失効してもらう（docs/e2ee.md の「失効の手順」）。
 */
export function accountRevocationUrls(
  issuer: string,
): { security: string; services: string } | null {
  try {
    const origin = new URL(issuer).origin;
    return {
      security: new URL("/settings/security", origin).href,
      services: new URL("/settings/services", origin).href,
    };
  } catch {
    return null;
  }
}
