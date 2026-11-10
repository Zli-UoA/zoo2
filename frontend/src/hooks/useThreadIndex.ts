import { useEffect, useMemo, useState } from "react";

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

const EMPTY_THREADS: ThreadSummary[] = [];

/** 未表示のチャンネルも、messages の購読を増やさず一覧を取得する。 */
export function useThreadIndex(userId: string | null) {
  const { config } = useCollections();
  const client = useMemo(() => createConfiguredApiClient(config), [config, userId]);
  const [result, setResult] = useState<{ userId: string; threads: ThreadSummary[] } | null>(null);

  useEffect(() => {
    if (!userId) return;
    let active = true;
    let inFlight = false;
    let pending = false;
    let generation = 0;
    const refresh = async () => {
      if (document.visibilityState === "hidden") return;
      if (inFlight) {
        pending = true;
        return;
      }
      inFlight = true;
      const started = generation;
      try {
        const threads = await client.get<ThreadSummary[]>("/api/threads");
        if (active && started === generation) setResult({ userId, threads });
      } catch {
        // 一時的な取得失敗では直前の一覧を残し、次回の更新で再試行する。
      } finally {
        inFlight = false;
        if (active && pending) {
          pending = false;
          void refresh();
        }
      }
    };
    const invalidate = () => {
      // 返信の保存前に始まった取得結果で、保存後の一覧を上書きしない。
      generation += 1;
      void refresh();
    };
    void refresh();
    const interval = window.setInterval(() => void refresh(), 60_000);
    window.addEventListener("focus", refresh);
    window.addEventListener("online", refresh);
    window.addEventListener(THREADS_CHANGED, invalidate);
    document.addEventListener("visibilitychange", refresh);
    return () => {
      active = false;
      window.clearInterval(interval);
      window.removeEventListener("focus", refresh);
      window.removeEventListener("online", refresh);
      window.removeEventListener(THREADS_CHANGED, invalidate);
      document.removeEventListener("visibilitychange", refresh);
    };
  }, [client, userId]);

  return result?.userId === userId ? result.threads : EMPTY_THREADS;
}
