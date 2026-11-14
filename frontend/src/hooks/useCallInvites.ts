import { useLiveQuery } from "@tanstack/react-db";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { createConfiguredApiClient, type ApiClient, type RequestOptions } from "../lib/api/client";
import {
  callInviteDurationMs,
  callInviteExpiresAt,
  msUntilNextExpiry,
  splitCallInvites,
  type SplitCallInvites,
} from "../lib/call/call-invites";
import type { CallInviteRow } from "../lib/electric/types";
import { useCollections, useOptionalCollections } from "../lib/electric/collections-context";
import { useCurrentUser } from "./useCurrentUser";

/**
 * 自分が呼ばれている招待（`incoming`）と、自分が呼んでいる招待（`outgoing`）。
 * Electric の `call_invites` の shape（呼び出し中だけ）から作る。期限を過ぎたものは、
 * サーバーが `missed` にする前でも、一覧から外す。期限は、端末の時計がサーバーとずれていても
 * 正しく働くよう、行を初めて受け取った時刻 + 招待の長さ（サーバーの `created_at` と `expires_at` の差）で決める。
 */
export function useCallInvites(): SplitCallInvites {
  const collections = useOptionalCollections();
  const { currentUserId } = useCurrentUser();
  const rows =
    useLiveQuery((q) =>
      collections && currentUserId
        ? q.from({ invites: collections.callInvites(currentUserId) })
        : undefined,
    )?.data ?? [];
  const [now, setNow] = useState(() => Date.now());
  const seenAt = useRef(new Map<string, number>());
  const deadlineOf = useCallback((row: CallInviteRow) => {
    let at = seenAt.current.get(row.id);
    if (at === undefined) {
      at = Date.now();
      seenAt.current.set(row.id, at);
    }
    const duration = callInviteDurationMs(row);
    return duration > 0 ? at + duration : callInviteExpiresAt(row);
  }, []);

  useEffect(() => {
    // 終わった招待の記録は捨てる。
    const live = new Set(rows.map((row) => row.id));
    for (const id of seenAt.current.keys()) if (!live.has(id)) seenAt.current.delete(id);
    const wait = msUntilNextExpiry(rows, Date.now(), deadlineOf);
    if (wait === null) return;
    const timer = setTimeout(() => setNow(Date.now()), wait + 50);
    return () => clearTimeout(timer);
  }, [rows, now, deadlineOf]);

  return useMemo(
    () => splitCallInvites(rows, currentUserId, now, deadlineOf),
    [rows, currentUserId, now, deadlineOf],
  );
}

/**
 * 招待の API を呼ぶクライアント（設定ごとに 1 つ）。
 *
 * クライアントは、最初のリクエストのときに作る。`CallBar` は通話していないときも、このフックを呼ぶ。
 * 描画のたびに作ると、開発用のユーザー ID などがない設定（テストなど）で、描画だけが失敗する。
 */
export function useCallInviteApi(): { request: ApiClient["request"] } {
  const { config } = useCollections();
  return useMemo(() => {
    let client: ApiClient | null = null;
    return {
      request: ((path: string, options?: RequestOptions) =>
        (client ??= createConfiguredApiClient(config)).request(
          path,
          options,
        )) as ApiClient["request"],
    };
  }, [config]);
}
