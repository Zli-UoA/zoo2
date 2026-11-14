import { useRouter } from "@tanstack/react-router";
import { useEffect } from "react";

import { useCurrentUser } from "../../hooks/useCurrentUser";
import { requestAcceptCallInvite } from "../../lib/call/call-invites";
import {
  DESKTOP_CALL_ACCEPT_EVENT,
  desktopListen,
  desktopReportNotifications,
  desktopSetFocus,
  desktopTakeCallAccept,
  isDesktopSession,
  type DesktopChange,
  type DesktopOpenTarget,
} from "../../lib/desktop";
import { useCollections } from "../../lib/electric/collections-context";

/**
 * デスクトップ版（Tauri）の中だけで動く。通常の Web では何もしない。
 *
 * - 通知のコレクションの変化を Rust 側に渡す（OS の通知は Rust 側が判定して出す）。
 * - 表示中のチャンネルと前面かどうかを知らせる（見ているチャンネルの通知を出さないため）。
 * - 通知のクリックで届く `zoo://open` で、そのメッセージへ移る。
 */
export function DesktopBridge({ channelId }: { channelId: string | null }) {
  if (!isDesktopSession()) return null;
  return <DesktopBridgeInner channelId={channelId} />;
}

function DesktopBridgeInner({ channelId }: { channelId: string | null }) {
  const router = useRouter();
  const collections = useCollections();
  const { currentUserId } = useCurrentUser();

  useEffect(() => {
    desktopSetFocus(channelId);
    const update = () => desktopSetFocus(channelId);
    window.addEventListener("focus", update);
    window.addEventListener("blur", update);
    return () => {
      window.removeEventListener("focus", update);
      window.removeEventListener("blur", update);
    };
  }, [channelId]);

  useEffect(() => {
    if (!currentUserId) return;
    const collection = collections.notifications(currentUserId);
    let cancelled = false;
    let unsubscribe: (() => void) | null = null;
    // 初回の読み込みが済んでから、その時点の全行を reset 付きで送る。
    // Rust 側は reset のバッチでは通知を出さないので、起動時に過去の通知は出ない。
    // stateWhenReady は preload (サーバーとの初回同期) を待つ。永続化したキャッシュを
    // 先に表示する initialRender の準備完了とは別なので、キャッシュだけでは解決しない。
    // 行が 0 件でも送る（Rust 側が「初回の同期が済んだ」と分かるように）。
    // 全行の取得と購読の開始の間に await を挟まないので、その間の変化を取りこぼさない。
    void collection.stateWhenReady().then((state) => {
      if (cancelled) return;
      const initial: DesktopChange[] = [...state].map(([key, row]) => ({
        op: "upsert",
        key: String(key),
        row: row as unknown as Record<string, unknown>,
      }));
      desktopReportNotifications({ reset: true, changes: initial, up_to_date: true });
      const subscription = collection.subscribeChanges((changes) => {
        const batch: DesktopChange[] = changes.map((change) =>
          change.type === "delete"
            ? { op: "delete", key: String(change.key) }
            : {
                op: "upsert",
                key: String(change.key),
                row: change.value as unknown as Record<string, unknown>,
              },
        );
        desktopReportNotifications({ reset: false, changes: batch, up_to_date: true });
      });
      unsubscribe = () => subscription.unsubscribe();
    });
    return () => {
      cancelled = true;
      unsubscribe?.();
    };
  }, [collections, currentUserId]);

  useEffect(() => {
    let stop: (() => void) | null = null;
    let cancelled = false;
    void desktopListen<DesktopOpenTarget>("zoo://open", (target) => {
      void router.navigate({
        to: "/channels/$channelId",
        params: { channelId: target.channel_id },
        search: { message: target.message_id ?? undefined },
      });
    }).then((unlisten) => {
      if (cancelled) unlisten();
      else stop = unlisten;
    });
    return () => {
      cancelled = true;
      stop?.();
    };
  }, [router]);

  // 着信のトーストの「応答」。イベントは「印を取りに来て」の合図にすぎず、応答するのは、Rust 側の
  // 印（`zoo_take_call_accept`）を一度だけ取れたときだけ。起動の直後にも、押されたまま残った印を取る。
  useEffect(() => {
    let stop: (() => void) | null = null;
    let cancelled = false;
    const take = () => {
      void desktopTakeCallAccept().then((inviteId) => {
        // 印は取った時点で消えるので、画面が離れていても、取れたら必ず頼む（window へのイベント）。
        if (inviteId) requestAcceptCallInvite(inviteId);
      });
    };
    take();
    void desktopListen<null>(DESKTOP_CALL_ACCEPT_EVENT, take).then((unlisten) => {
      if (cancelled) unlisten();
      else stop = unlisten;
    });
    return () => {
      cancelled = true;
      stop?.();
    };
  }, []);

  return null;
}
