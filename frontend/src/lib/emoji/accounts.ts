import { useMemo, useSyncExternalStore } from "react";
import { createConfiguredApiClient } from "../api/client";
import { useOptionalCollections } from "../electric/collections-context";
import { getCollections } from "../electric/collections";
import { visiblePoll } from "../visiblePoll";
import type { AppConfig } from "../config";

export type AuthZAccount = { username: string; display_name: string; avatar_url: string | null };
const EMPTY: readonly AuthZAccount[] = [];
const stores = new WeakMap<AppConfig, ReturnType<typeof directory>>();
function directory(config: AppConfig) {
  const client = createConfiguredApiClient(config);
  let accounts = EMPTY;
  let checked = 0;
  let stop: (() => void) | undefined;
  const listeners = new Set<() => void>();
  return {
    snapshot: () => accounts,
    subscribe: (listener: () => void) => {
      listeners.add(listener);
      if (listeners.size === 1) {
        let active = true;
        const poll = visiblePoll(
          async (signal) => {
            if (checked && Date.now() - checked < 60_000) return;
            let result: readonly AuthZAccount[] = EMPTY;
            try {
              result = (
                await client.get<{ accounts: AuthZAccount[] }>("/api/emoji-accounts", { signal })
              ).accounts;
            } finally {
              if (active && !signal.aborted) {
                checked = Date.now();
                accounts = result;
                listeners.forEach((notify) => notify());
              }
            }
          },
          { interval: 60_000 },
        );
        stop = () => {
          active = false;
          poll.dispose();
        };
      }
      return () => {
        listeners.delete(listener);
        if (!listeners.size) {
          stop?.();
          stop = undefined;
        }
      };
    },
  };
}
const noSubscribe = () => () => {};
const empty = () => EMPTY;

/** 認証設定ごとに結果・進行中の取得・表示中の60秒更新を共有する。 */
export function useAuthZAccounts(): readonly AuthZAccount[] {
  const collections = useOptionalCollections();
  const store = useMemo(() => {
    let config: AppConfig;
    try {
      config = (collections ?? getCollections()).config;
    } catch {
      return undefined;
    }
    let value = stores.get(config);
    if (!value) {
      value = directory(config);
      stores.set(config, value);
    }
    return value;
  }, [collections, collections?.config]);
  return useSyncExternalStore(store?.subscribe ?? noSubscribe, store?.snapshot ?? empty, empty);
}
