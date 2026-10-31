import { ApiError, createConfiguredApiClient, type FetchLike } from "./api/client";
import type { UserResponse } from "./api/types";
import type { AppConfig } from "./config";

/**
 * バックエンドの開発用固定管理者。`backend/src/auth.rs` の
 * `DEFAULT_DEV_USER_ID`（環境変数 `DEV_DEFAULT_USER_ID` の既定値）と同じ。
 * `/api/users/me` が失敗したときのフォールバックにだけ使う。
 */
const DEFAULT_DEV_ADMIN_ID = "00000000-0000-0000-0000-000000000001";

/**
 * `GET /api/users/me` のレスポンス。既存の `UserResponse` に `is_admin` が
 * 加わった形（古いバックエンドでは欠落しうるので optional）。
 */
interface MeResponse extends UserResponse {
  is_admin?: boolean;
}

/** 現在ユーザーの解決結果。プロフィールの最新値は `users` Shape から別途引く。 */
export interface CurrentUserState {
  currentUserId: string | null;
  isLoading: boolean;
  /** ワークスペース管理者かどうか（RBAC UI の出し分け用）。 */
  isAdmin: boolean;
  /** 401 以外で解決に失敗したときの理由（表示・調査用）。 */
  error: string | null;
}

export interface CurrentUserResolution {
  currentUserId: string | null;
  isAdmin: boolean;
  error: string | null;
}

function describeError(cause: unknown): string {
  return cause instanceof Error ? cause.message : String(cause);
}

/**
 * 現在ユーザーを解決する。
 *
 * `GET /api/users/me` から `id` と `is_admin` を取る。Bearer は既存の API
 * クライアントが `getToken()`（#60 の PKCE セッション）経由で載せ、dev では
 * `VITE_DEV_USER_ID` を `X-Dev-User` として送る。
 *
 * dev で `/me` が失敗した場合は従来どおり `VITE_DEV_USER_ID` にフォールバック
 * する（バックエンドの開発用固定管理者だけ `is_admin` を true にする）。
 * OIDC の 401 再ログインは `ApiClient` → `notifyUnauthorized` → `AuthGate` の
 * 導線に委ねる。ここは null を返すだけでよい。
 */
export async function resolveCurrentUser(
  config: AppConfig,
  options: { fetch?: FetchLike } = {},
): Promise<CurrentUserResolution> {
  // direct モードの dev は現在ユーザーを特定しない（従来どおり null）。
  if (!config.oidc && !config.devUserId) {
    return { currentUserId: null, isAdmin: false, error: null };
  }
  const client = createConfiguredApiClient(config, options);
  try {
    const me = await client.get<MeResponse>("/api/users/me", {
      signal: AbortSignal.timeout(8_000),
    });
    return { currentUserId: me.id, isAdmin: me.is_admin === true, error: null };
  } catch (cause) {
    if (!config.oidc) {
      return {
        currentUserId: config.devUserId,
        isAdmin: config.devUserId === DEFAULT_DEV_ADMIN_ID,
        error: null,
      };
    }
    if (cause instanceof ApiError && cause.status === 401) {
      return { currentUserId: null, isAdmin: false, error: null };
    }
    return { currentUserId: null, isAdmin: false, error: describeError(cause) };
  }
}

export interface CurrentUserStore {
  getState(): CurrentUserState;
  subscribe(listener: () => void): () => void;
  /** 起動時の解決を開始する。2 回目以降は進行中の Promise を使い回す。 */
  ensureLoaded(): Promise<void>;
  refresh(): Promise<void>;
}

/**
 * 解決処理をストア化する。`/api/users/me` を 1 回に集約し、
 * `useCurrentUser`（`useSyncExternalStore`）と書き込み層（退出の本人確認）の
 * 双方から同じ値を見られるようにする。
 */
export function createCurrentUserStore(
  config: AppConfig,
  options: { fetch?: FetchLike } = {},
): CurrentUserStore {
  let state: CurrentUserState = config.oidc
    ? { currentUserId: null, isLoading: true, isAdmin: false, error: null }
    : {
        currentUserId: config.devUserId,
        isLoading: false,
        isAdmin: config.devUserId === DEFAULT_DEV_ADMIN_ID,
        error: null,
      };
  let inflight: Promise<void> | null = null;
  let refreshing: Promise<void> | null = null;
  const listeners = new Set<() => void>();

  function setState(next: CurrentUserState): void {
    state = next;
    for (const listener of listeners) {
      listener();
    }
  }

  return {
    getState: () => state,
    subscribe: (listener) => {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    refresh: () => {
      refreshing ??= resolveCurrentUser(config, options)
        .then((resolved) => {
          setState({
            ...resolved,
            // A transient role refresh failure does not invalidate the signed-in identity.
            // resolveCurrentUser returns error=null for 401, so that path still clears it.
            currentUserId: resolved.error !== null ? state.currentUserId : resolved.currentUserId,
            isLoading: false,
          });
        })
        .finally(() => {
          refreshing = null;
        });
      return refreshing;
    },
    ensureLoaded: () => {
      if (!inflight) {
        if (config.oidc) {
          setState({ currentUserId: null, isLoading: true, isAdmin: false, error: null });
        }
        inflight = resolveCurrentUser(config, options).then((resolved) => {
          setState({
            currentUserId: resolved.currentUserId,
            isLoading: false,
            isAdmin: resolved.isAdmin,
            error: resolved.error,
          });
          if (resolved.error !== null) {
            // 一時的な失敗（ネットワーク・5xx）は次の ensureLoaded（マウント・
            // 画面遷移）で再試行できるようにする。401 は AuthGate が再ログインする。
            inflight = null;
          }
        });
      }
      return inflight;
    },
  };
}

const stores = new WeakMap<AppConfig, CurrentUserStore>();

/**
 * config ごとの共有ストア。`getCollections()` と `useCurrentUser()` が同じ
 * config を渡すため、解決は 1 回に集約される。
 */
export function getCurrentUserStore(config: AppConfig): CurrentUserStore {
  let store = stores.get(config);
  if (!store) {
    store = createCurrentUserStore(config);
    stores.set(config, store);
  }
  return store;
}
