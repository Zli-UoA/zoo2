import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { createConfiguredApiClient } from "../lib/api/client";
import { useCollections } from "../lib/electric/collections-context";
import { THREADS_CHANGED } from "../lib/electric/threads";

export interface ThreadSummary {
  id: string;
  channel_id: string;
  content: string;
  thread_archive_at: string | null;
  reply_count: number;
}

export interface ThreadPage {
  items: ThreadSummary[];
  next_cursor: string | null;
}
const EMPTY_PAGE: ThreadPage = { items: [], next_cursor: null };
type Result = ThreadPage & { key: string; isLoading: boolean; isError: boolean };

/** アクティブな一覧は定期取得し、履歴は展開時にページ単位で取得する。 */
export function useThreadIndex(
  userId: string | null,
  {
    channelId,
    archived = false,
    enabled = true,
  }: {
    channelId?: string;
    archived?: boolean;
    enabled?: boolean;
  } = {},
) {
  const { config } = useCollections();
  const client = useMemo(() => createConfiguredApiClient(config), [config, userId]);
  const key = JSON.stringify([userId, channelId, archived]);
  const [result, setResult] = useState<Result | null>(null);
  const requestMore = useRef<() => void>(() => {});
  const loadMore = useCallback(() => requestMore.current(), []);

  useEffect(() => {
    if (!userId || !enabled) return;
    let active = true;
    let inFlight = false;
    let pending = false;
    let generation = 0;
    let cursor: string | null = null;
    let failedRequest: { append: boolean; cursor: string | null } | null = null;
    const refresh = async (append = false, before = cursor) => {
      if (document.visibilityState === "hidden") return;
      if (inFlight) {
        if (!append) pending = true;
        return;
      }
      inFlight = true;
      const started = generation;
      const params = new URLSearchParams();
      if (channelId) params.set("channel_id", channelId);
      if (archived) params.set("archived", "true");
      if (append && before) params.set("before", before);
      const query = params.size ? `?${params}` : "";
      setResult((current) => ({
        ...(current?.key === key ? current : EMPTY_PAGE),
        key,
        isLoading: true,
        isError: false,
      }));
      try {
        const page = await client.get<ThreadPage>(`/api/threads${query}`);
        if (active && started === generation) {
          failedRequest = null;
          cursor = page.next_cursor;
          setResult((current) => ({
            ...page,
            items:
              append && current?.key === key
                ? [
                    ...new Map(
                      [...current.items, ...page.items].map((item) => [item.id, item]),
                    ).values(),
                  ]
                : page.items,
            key,
            isLoading: false,
            isError: false,
          }));
        }
      } catch {
        // 一時的な取得失敗では直前の一覧を残し、再試行できるようにする。
        if (active && started === generation) {
          failedRequest = { append, cursor: before };
          setResult((current) =>
            current?.key === key ? { ...current, isLoading: false, isError: true } : current,
          );
        }
      } finally {
        inFlight = false;
        if (active && pending) {
          pending = false;
          void refresh();
        }
      }
    };
    const invalidate = () => {
      // 更新前に始まった取得結果で、更新後の一覧を上書きしない。
      generation += 1;
      void refresh();
    };
    const refreshFirst = () => void refresh();
    requestMore.current = () => {
      const request = failedRequest;
      void refresh(request?.append ?? cursor !== null, request ? request.cursor : cursor);
    };
    void refresh();
    const interval = archived ? null : window.setInterval(refreshFirst, 60_000);
    window.addEventListener("focus", refreshFirst);
    window.addEventListener("online", refreshFirst);
    window.addEventListener(THREADS_CHANGED, invalidate);
    document.addEventListener("visibilitychange", refreshFirst);
    return () => {
      active = false;
      requestMore.current = () => {};
      if (interval !== null) window.clearInterval(interval);
      window.removeEventListener("focus", refreshFirst);
      window.removeEventListener("online", refreshFirst);
      window.removeEventListener(THREADS_CHANGED, invalidate);
      document.removeEventListener("visibilitychange", refreshFirst);
    };
  }, [client, userId, channelId, archived, enabled, key]);

  const current = result?.key === key ? result : null;
  return {
    threads: current?.items ?? EMPTY_PAGE.items,
    hasMore: !!current?.next_cursor,
    isLoading: current?.isLoading ?? false,
    isError: current?.isError ?? false,
    loadMore,
  };
}
