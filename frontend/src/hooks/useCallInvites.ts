import { useLiveQuery } from "@tanstack/react-db";
import { useEffect, useMemo, useState } from "react";

import { createConfiguredApiClient } from "../lib/api/client";
import {
  msUntilNextExpiry,
  splitCallInvites,
  type SplitCallInvites,
} from "../lib/call/call-invites";
import { useCollections, useOptionalCollections } from "../lib/electric/collections-context";
import { useCurrentUser } from "./useCurrentUser";

/**
 * 自分が呼ばれている招待（`incoming`）と、自分が呼んでいる招待（`outgoing`）。
 * Electric の `call_invites` の shape（呼び出し中だけ）から作る。期限を過ぎたものは、
 * サーバーが `missed` にする前でも、期限の時刻に一覧から外す。
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

  useEffect(() => {
    const wait = msUntilNextExpiry(rows, Date.now());
    if (wait === null) return;
    const timer = setTimeout(() => setNow(Date.now()), wait + 50);
    return () => clearTimeout(timer);
  }, [rows, now]);

  return useMemo(() => splitCallInvites(rows, currentUserId, now), [rows, currentUserId, now]);
}

/** 招待の API を呼ぶクライアント（設定ごとに 1 つ）。 */
export function useCallInviteApi() {
  const { config } = useCollections();
  return useMemo(() => createConfiguredApiClient(config), [config]);
}
