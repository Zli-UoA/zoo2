/**
 * AuthZ のプロフィール設定画面の URL。issuer（例: https://account.zli.works/api/auth）と
 * 同じ origin の /settings/profile を指す。`return_to` を付けると、AuthZ は保存後にそこへ戻す
 * （登録済みの連携先 origin だけが有効）。
 */
export function accountProfileUrl(issuer: string, returnTo: string): string | null {
  try {
    const url = new URL("/settings/profile", new URL(issuer).origin);
    url.searchParams.set("return_to", returnTo);
    return url.href;
  } catch {
    return null;
  }
}
