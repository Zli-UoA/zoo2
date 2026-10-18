import { useCallback, useEffect, useSyncExternalStore } from "react";

import { getCurrentUserStore, type CurrentUserState } from "../lib/currentUser";
import { getCollections } from "../lib/electric/collections";

/**
 * 現在ユーザーを購読する。OIDC 有効時は `/api/users/me` を起動時に 1 回呼び、
 * dev では `VITE_DEV_USER_ID` をそのまま返す。`config` は `getCollections()` が
 * 解決済みのものを使う（画面はマウント後のブラウザでのみ描画される）。
 */
export function useCurrentUser(): CurrentUserState {
  const store = getCurrentUserStore(getCollections().config);
  const subscribe = useCallback((listener: () => void) => store.subscribe(listener), [store]);
  const getState = useCallback(() => store.getState(), [store]);
  const state = useSyncExternalStore(subscribe, getState, getState);
  useEffect(() => {
    void store.ensureLoaded();
  }, [store]);
  return state;
}
