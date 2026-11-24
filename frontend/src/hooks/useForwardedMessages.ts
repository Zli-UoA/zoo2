import {
  createContext,
  createElement,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";

import { useCurrentUser } from "./useCurrentUser";
import { DEFAULT_CONFIG, type AppConfig } from "../lib/config";
import { createConfiguredApiClient } from "../lib/api/client";
import {
  cachedForwardEntry,
  forgetForwardIds,
  resolveForwardMessages,
  type ForwardCacheScope,
  type ResolvedForwardEntry,
} from "../lib/api/forwards";
import { useCollections } from "../lib/electric/collections-context";

export type ForwardEntryState = ResolvedForwardEntry | { status: "loading" };

export interface ForwardedMessagesContextValue {
  entries: ReadonlyMap<string, ForwardEntryState>;
  retry: (id: string) => void;
  currentUserId: string | null;
}

const ForwardedMessagesContext = createContext<ForwardedMessagesContextValue>({
  entries: new Map(),
  retry: () => {},
  currentUserId: null,
});

/**
 * 表示中の転送元メッセージ ID 群を一括解決・キャッシュして状態を返す。
 * 暗号化チャンネル等では `enabled = false` を渡してサーバー問い合わせを抑止する。
 */
export function useForwardedMessages(
  ids: readonly string[],
  enabled = true,
): ForwardedMessagesContextValue {
  const collections = useCollections() as { config?: AppConfig } | undefined;
  const config = collections?.config ?? DEFAULT_CONFIG;
  const { currentUserId } = useCurrentUser();
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  const scope: ForwardCacheScope = useMemo(
    () => ({ apiBaseUrl: config.apiBaseUrl, currentUserId }),
    [config.apiBaseUrl, currentUserId],
  );
  const uniqueIds = useMemo(() => [...new Set(ids.filter((id) => id.length > 0))].sort(), [ids]);
  const idsKey = uniqueIds.join(",");

  const [resolvedMap, setResolvedMap] = useState<Map<string, ResolvedForwardEntry>>(
    () => new Map(),
  );
  const [retryTick, setRetryTick] = useState(0);

  useEffect(() => {
    if (!enabled || uniqueIds.length === 0) return;
    const missing: string[] = [];
    const hits = new Map<string, ResolvedForwardEntry>();
    for (const id of uniqueIds) {
      const cached = cachedForwardEntry(scope, id);
      if (cached) {
        hits.set(id, cached);
      } else {
        missing.push(id);
      }
    }
    if (hits.size > 0) {
      setResolvedMap((prev) => {
        const next = new Map(prev);
        for (const [k, v] of hits) next.set(k, v);
        return next;
      });
    }
    if (missing.length === 0) return;

    let cancelled = false;
    void resolveForwardMessages(client, scope, missing).then((fetched) => {
      if (cancelled) return;
      setResolvedMap((prev) => {
        const next = new Map(prev);
        for (const [k, v] of fetched) next.set(k, v);
        return next;
      });
    });
    return () => {
      cancelled = true;
    };
  }, [client, enabled, idsKey, retryTick, scope, uniqueIds]);

  const retry = useCallback(
    (id: string) => {
      forgetForwardIds(scope, [id]);
      setResolvedMap((prev) => {
        if (!prev.has(id)) return prev;
        const next = new Map(prev);
        next.delete(id);
        return next;
      });
      setRetryTick((n) => n + 1);
    },
    [scope],
  );

  const entries = useMemo(() => {
    const map = new Map<string, ForwardEntryState>();
    if (!enabled) return map;
    for (const id of uniqueIds) {
      const cached = cachedForwardEntry(scope, id) ?? resolvedMap.get(id);
      map.set(id, cached ?? { status: "loading" });
    }
    return map;
  }, [enabled, resolvedMap, scope, uniqueIds]);

  return useMemo(() => ({ entries, retry, currentUserId }), [entries, retry, currentUserId]);
}

export function ForwardedMessagesProvider({
  value,
  children,
}: {
  value: ForwardedMessagesContextValue;
  children: ReactNode;
}) {
  return createElement(ForwardedMessagesContext.Provider, { value }, children);
}

export function useForwardedMessagesContext(): ForwardedMessagesContextValue {
  return useContext(ForwardedMessagesContext);
}
