import {
  getToken as defaultGetToken,
  notifyUnauthorized,
  type TokenProvider,
  type UnauthorizedHandler,
} from "../auth";

export interface ShapeFetchSource {
  getToken?: TokenProvider;
  devUserId?: string | null;
  /**
   * 認証ヘッダ（`Authorization` / `X-Dev-User`）を付与するか。既定は true。
   *
   * direct（Electric 直結）は認可なしの開発専用経路で、Electric の CORS は
   * `authorization` / `X-Dev-User` を許可していないため false にする。
   */
  auth?: boolean;
  /** Shape の 401 を再ログイン導線へ渡す。既定は `auth.notifyUnauthorized`。 */
  onUnauthorized?: UnauthorizedHandler;
  fetch?: typeof fetch;
}

/**
 * Electric の ShapeStream 用 fetch を生成する。
 *
 * REST クライアント（`api/client.ts` の `buildAuthHeaders()`）と同じ
 * `Authorization: Bearer` / `X-Dev-User` を、値が存在するときだけ付与する。
 *
 * ShapeStream の `headers` は値を常に送ってしまうため、許可されていない
 * `authorization` ヘッダを付けると CORS の preflight で弾かれる。トークンが無い
 * ときは何も付けないよう fetch 側で合成する。
 */
export function createShapeFetch(source: ShapeFetchSource = {}): typeof fetch {
  const auth = source.auth ?? true;
  const getToken = source.getToken ?? defaultGetToken;
  const onUnauthorized = source.onUnauthorized ?? notifyUnauthorized;
  const baseFetch: typeof fetch = source.fetch ?? ((input, init) => fetch(input, init));
  const devUserId = source.devUserId ?? null;

  return async (input, init) => {
    const headers = new Headers(init?.headers);
    if (auth) {
      if (devUserId) {
        headers.set("X-Dev-User", devUserId);
      }
      const token = await getToken();
      if (token) {
        headers.set("Authorization", `Bearer ${token}`);
      }
    }
    const response = await baseFetch(input, { ...init, headers });
    if (auth && response.status === 401) {
      onUnauthorized();
    }
    return response;
  };
}
