import { ApiError, createConfiguredApiClient, type FetchLike } from "./api/client";
import type { UserResponse } from "./api/types";
import type { AppConfig } from "./config";

/** 現在ユーザーの解決結果。プロフィールの最新値は `users` Shape から別途引く。 */
export interface CurrentUserState {
  currentUserId: string | null;
  isLoading: boolean;
  /** 401 以外で解決に失敗したときの理由（表示・調査用）。 */
  error: string | null;
}

export interface CurrentUserResolution {
  currentUserId: string | null;
  error: string | null;
}

function describeError(cause: unknown): string {
  return cause instanceof Error ? cause.message : String(cause);
}

/**
 * 現在ユーザーを解決する。
 *
 * - OIDC 未設定（dev の X-Dev-User）: `VITE_DEV_USER_ID` をそのまま使う。
 * - OIDC 設定済み: `GET /api/users/me` の `id` を採用する。Bearer は既存の API
 *   クライアントが `getToken()`（#60 の PKCE セッション）経由で載せる。
 *
 * 401 の再ログインは `ApiClient` → `notifyUnauthorized` → `AuthGate` の導線に
 * 委ねる。ここは null を返すだけでよい。
 */
export async function resolveCurrentUser(
  config: AppConfig,
  options: { fetch?: FetchLike } = {},
): Promise<CurrentUserResolution> {
  if (!config.oidc) {
    return { currentUserId: config.devUserId, error: null };
  }
  const client = createConfiguredApiClient(config, options);
  try {
    const user = await client.get<UserResponse>("/api/users/me");
    return { currentUserId: user.id, error: null };
  } catch (cause) {
    if (cause instanceof ApiError && cause.status === 401) {
      return { currentUserId: null, error: null };
    }
    return { currentUserId: null, error: describeError(cause) };
  }
}

export interface CurrentUserStore {
  getState(): CurrentUserState;
  subscribe(listener: () => void): () => void;
  /** 起動時の解決を開始する。2 回目以降は進行中の Promise を使い回す。 */
  ensureLoaded(): Promise<void>;
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
    ? { currentUserId: null, isLoading: true, error: null }
    : { currentUserId: config.devUserId, isLoading: false, error: null };
  let inflight: Promise<void> | null = null;
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
    ensureLoaded: () => {
      if (!config.oidc) {
        return Promise.resolve();
      }
      if (!inflight) {
        setState({ currentUserId: null, isLoading: true, error: null });
        inflight = resolveCurrentUser(config, options).then((resolved) => {
          setState({
            currentUserId: resolved.currentUserId,
            isLoading: false,
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
