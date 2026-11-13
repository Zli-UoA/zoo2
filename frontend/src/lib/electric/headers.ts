import {
  getToken as defaultGetToken,
  notifyAuthorized,
  notifyUnauthorized,
  type AuthorizedHandler,
  type TokenProvider,
  type UnauthorizedHandler,
} from "../auth";
import { inferElectricShape, reportElectricSyncEvent } from "../telemetry/capture";
import { runWithHttpSpan } from "../telemetry/tracer";

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
  apiBaseUrl?: string;
  telemetry?: boolean;
  /** Shape の 401 を再ログイン導線へ渡す。既定は `auth.notifyUnauthorized`。 */
  onUnauthorized?: UnauthorizedHandler;
  /** Shape の成功通知（再ログイン試行カウンタの消去用）。既定は `auth.notifyAuthorized`。 */
  onAuthorized?: AuthorizedHandler;
  fetch?: typeof fetch;
}

function targetUrlString(input: RequestInfo | URL): string {
  if (typeof input === "string") return input;
  if (input instanceof URL) return input.toString();
  return input.url;
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
  const onAuthorized = source.onAuthorized ?? notifyAuthorized;
  const baseFetch: typeof fetch = source.fetch ?? ((input, init) => fetch(input, init));
  const devUserId = source.devUserId ?? null;
  const apiBaseUrl = source.apiBaseUrl ?? "";
  const telemetry = source.telemetry ?? true;

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
    const url = targetUrlString(input);
    const shape = inferElectricShape(url);
    const response = await runWithHttpSpan(
      {
        enabled: auth && telemetry,
        method: init?.method ?? "GET",
        url,
        apiBaseUrl,
        headers,
      },
      async (traceIds) => {
        try {
          const res = await baseFetch(input, { ...init, headers });
          if (telemetry && res.status >= 500) {
            reportElectricSyncEvent({
              shape,
              state: "error",
              httpStatus: res.status,
              traceIds,
            });
          }
          return res;
        } catch (error) {
          const isAbort = error instanceof DOMException && error.name === "AbortError";
          if (telemetry && !isAbort) {
            reportElectricSyncEvent({
              shape,
              state: "error",
              httpStatus: 0,
              traceIds,
            });
          }
          throw error;
        }
      },
    );
    if (auth) {
      if (response.status === 401) {
        onUnauthorized();
      } else if (response.ok) {
        onAuthorized();
      }
    }
    return response;
  };
}
