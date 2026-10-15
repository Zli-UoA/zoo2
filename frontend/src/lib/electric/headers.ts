import { getToken as defaultGetToken, type TokenProvider } from "../auth";

export interface ShapeFetchSource {
  getToken?: TokenProvider;
  devUserId?: string | null;
  fetch?: typeof fetch;
}

/**
 * Electric の ShapeStream 用 fetch を生成する。
 *
 * REST クライアント（`api/client.ts` の `buildAuthHeaders()`）と同じ
 * `Authorization: Bearer` / `X-Dev-User` を、値が存在するときだけ付与する。
 *
 * ShapeStream の `headers` は値を常に送ってしまうため、開発時の Electric 直結
 * （`ELECTRIC_INSECURE`）では許可されていない `authorization` ヘッダが CORS の
 * preflight で弾かれる。トークンが無いときは何も付けないよう fetch 側で合成する。
 */
export function createShapeFetch(source: ShapeFetchSource = {}): typeof fetch {
  const getToken = source.getToken ?? defaultGetToken;
  const baseFetch: typeof fetch = source.fetch ?? ((input, init) => fetch(input, init));
  const devUserId = source.devUserId ?? null;

  return async (input, init) => {
    const headers = new Headers(init?.headers);
    if (devUserId) {
      headers.set("X-Dev-User", devUserId);
    }
    const token = await getToken();
    if (token) {
      headers.set("Authorization", `Bearer ${token}`);
    }
    return baseFetch(input, { ...init, headers });
  };
}
