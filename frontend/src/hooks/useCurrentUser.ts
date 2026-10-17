import { useCallback, useEffect, useSyncExternalStore } from "react";

import { ApiError, createConfiguredApiClient, type FetchLike } from "../lib/api/client";
import type { UserResponse } from "../lib/api/types";
import type { AppConfig } from "../lib/config";
import { getCollections } from "../lib/electric/collections";

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
 * 解決処理をストア化する。コンポーネントをまたいで `/api/users/me` を
 * 1 回に集約し、`useSyncExternalStore` から購読できるようにする。
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
      inflight ??= resolveCurrentUser(config, options).then((resolved) => {
        setState({
          currentUserId: resolved.currentUserId,
          isLoading: false,
          error: resolved.error,
        });
      });
      return inflight;
    },
  };
}

const stores = new WeakMap<AppConfig, CurrentUserStore>();

function storeFor(config: AppConfig): CurrentUserStore {
  let store = stores.get(config);
  if (!store) {
    store = createCurrentUserStore(config);
    stores.set(config, store);
  }
  return store;
}

/**
 * 現在ユーザーを購読する。OIDC 有効時は `/api/users/me` を起動時に 1 回呼び、
 * dev では `VITE_DEV_USER_ID` をそのまま返す。`config` は `getCollections()` が
 * 解決済みのものを使う（画面はマウント後のブラウザでのみ描画される）。
 */
export function useCurrentUser(): CurrentUserState {
  const store = storeFor(getCollections().config);
  const subscribe = useCallback((listener: () => void) => store.subscribe(listener), [store]);
  const getState = useCallback(() => store.getState(), [store]);
  const state = useSyncExternalStore(subscribe, getState, getState);
  useEffect(() => {
    void store.ensureLoaded();
  }, [store]);
  return state;
}
