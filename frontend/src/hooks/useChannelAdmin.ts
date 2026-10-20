import { useMemo, useRef, useState } from "react";
import { toast } from "sonner";

import { ApiError, createConfiguredApiClient } from "../lib/api/client";
import { getCollections } from "../lib/electric/collections";
import { changeMessageMatcher, syncTxid } from "../lib/electric/txid";
import type { ChannelRow, ChannelUserRow, MessageRow } from "../lib/electric/types";
import { useChannelAccess, type ChannelAccess } from "./useRealtime";

/** Shape 反映待ちの上限。届かなくても操作自体は成功しているので致命的扱いにしない。 */
const SHAPE_SYNC_TIMEOUT_MS = 5000;

export type ChannelAdminAction = "archive" | "unarchive" | "kick" | "delete-message";

export interface ChannelAdmin {
  access: ChannelAccess;
  /** 実行中の操作。二重送信を防ぐため、いずれか 1 つだけ立つ。 */
  pending: ChannelAdminAction | null;
  /** アーカイブ / アンアーカイブ。成功時は true。 */
  setArchived: (archived: boolean) => Promise<boolean>;
  /** メンバーをチャンネルからキックする。 */
  kickMember: (userId: string) => Promise<boolean>;
  /** 管理者によるメッセージの強制削除。 */
  deleteMessage: (messageId: string) => Promise<boolean>;
}

/**
 * API のエラーをユーザー向けの日本語文言にする。
 *
 * バックエンドの `AppError` は英語の説明的メッセージを返すため、権限・存在・
 * 状態のエラーは既知の日本語を優先する。それ以外はレスポンスの `error` を
 * そのまま使う（詳細なバリデーション理由など）。
 */
export function describeApiError(cause: unknown): string {
  if (cause instanceof ApiError) {
    if (cause.status === 403) {
      return "この操作を行う権限がありません。";
    }
    if (cause.status === 404) {
      return "対象が見つかりませんでした。";
    }
    if (cause.status === 400) {
      return "この操作は現在の状態では実行できません。";
    }
    const body = cause.body;
    if (body && typeof body === "object" && "error" in body) {
      const message = (body as { error: unknown }).error;
      if (typeof message === "string" && message.length > 0) {
        return message;
      }
    }
    return cause.message;
  }
  return cause instanceof Error ? cause.message : String(cause);
}

/**
 * チャンネル管理操作（アーカイブ・キック・強制削除）の実行と通知をまとめる。
 *
 * 書き込みは `api/client.ts` を直接叩き、表示の更新は Electric の Shape に任せる
 * （`channels` / `channel_user` / `messages` の変更がそのまま流れる）。レスポンスの
 * `txid` を Shape と突き合わせてから成功を返すため、ダイアログを閉じた直後に
 * 古い状態のメニューを再操作しにくい。失敗は `sonner` で通知し、呼び出し側は
 * 戻り値で確認ダイアログを閉じるか判断する。
 */
export function useChannelAdmin(channel: ChannelRow | null): ChannelAdmin {
  const access = useChannelAccess(channel);
  const collections = useMemo(() => getCollections(), []);
  const client = useMemo(() => createConfiguredApiClient(collections.config), [collections]);
  const [pending, setPending] = useState<ChannelAdminAction | null>(null);
  // 同じイベント内の連打は state 反映前なので、ref で同期的に弾く。
  const pendingRef = useRef<ChannelAdminAction | null>(null);

  /**
   * Shape の反映待ち。`txid` の待機がタイムアウトしても API 自体は成功して
   * いるため、操作失敗にはせずログだけ残す（表示は次の Shape 更新で追いつく）。
   */
  async function waitForShape(wait: () => Promise<void>): Promise<void> {
    try {
      await wait();
    } catch (cause) {
      console.warn("Shape 反映の待機に失敗しました", cause);
    }
  }

  async function run(
    action: ChannelAdminAction,
    task: () => Promise<void>,
    successMessage: string,
  ): Promise<boolean> {
    if (pendingRef.current) {
      return false;
    }
    pendingRef.current = action;
    setPending(action);
    try {
      await task();
      toast.success(successMessage);
      return true;
    } catch (cause) {
      toast.error("操作に失敗しました", {
        description: describeApiError(cause),
        id: `channel-admin-${action}`,
      });
      return false;
    } finally {
      pendingRef.current = null;
      setPending(null);
    }
  }

  return {
    access,
    pending,
    setArchived: async (archived) => {
      const channelId = channel?.id;
      if (!channelId) {
        return false;
      }
      return run(
        archived ? "archive" : "unarchive",
        async () => {
          const response = await client.post<ChannelRow>(
            `/api/channels/${channelId}/${archived ? "archive" : "unarchive"}`,
          );
          await waitForShape(() =>
            syncTxid({
              mode: collections.config.txidMode,
              txid: response.txid,
              target: collections.channels.utils,
              match: changeMessageMatcher<ChannelRow>(
                "update",
                (row) =>
                  row.id === channelId &&
                  (archived ? row.archive_at !== null : row.archive_at === null),
              ),
              timeout: SHAPE_SYNC_TIMEOUT_MS,
            }),
          );
        },
        archived ? "チャンネルをアーカイブしました" : "アーカイブを解除しました",
      );
    },
    kickMember: async (userId) => {
      const channelId = channel?.id;
      if (!channelId) {
        return false;
      }
      return run(
        "kick",
        async () => {
          const response = await client.delete<ChannelUserRow>(
            `/api/channels/${channelId}/users/${userId}`,
          );
          await waitForShape(() =>
            syncTxid({
              mode: collections.config.txidMode,
              txid: response.txid,
              target: collections.channelUsers.utils,
              match: changeMessageMatcher<ChannelUserRow>(
                "delete",
                (row) => row.user_id === userId && row.channel_id === channelId,
              ),
              timeout: SHAPE_SYNC_TIMEOUT_MS,
            }),
          );
        },
        "メンバーをキックしました",
      );
    },
    deleteMessage: async (messageId) => {
      const channelId = channel?.id;
      if (!channelId) {
        return false;
      }
      const messagesCollection = collections.messages(channelId);
      return run(
        "delete-message",
        async () => {
          const response = await client.delete<MessageRow>(`/api/messages/${messageId}`);
          await waitForShape(() =>
            syncTxid({
              mode: collections.config.txidMode,
              txid: response.txid,
              target: messagesCollection.utils,
              match: changeMessageMatcher<MessageRow>(
                "update",
                (row) => row.id === messageId && row.deleted_at !== null,
              ),
              timeout: SHAPE_SYNC_TIMEOUT_MS,
            }),
          );
        },
        "メッセージを削除しました",
      );
    },
  };
}
