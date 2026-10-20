import { useMemo, useRef, useState } from "react";
import { toast } from "sonner";

import { ApiError, createConfiguredApiClient } from "../lib/api/client";
import { getCollections } from "../lib/electric/collections";
import type { ChannelRow } from "../lib/electric/types";
import { useChannelAccess, type ChannelAccess } from "./useRealtime";

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

/** API のエラーボディ (`{ error: string }`) を優先してユーザー向け文言にする。 */
export function describeApiError(cause: unknown): string {
  if (cause instanceof ApiError) {
    const body = cause.body;
    if (body && typeof body === "object" && "error" in body) {
      const message = (body as { error: unknown }).error;
      if (typeof message === "string" && message.length > 0) {
        return message;
      }
    }
    if (cause.status === 403) {
      return "この操作を行う権限がありません。";
    }
    if (cause.status === 404) {
      return "対象が見つかりませんでした。";
    }
    return cause.message;
  }
  return cause instanceof Error ? cause.message : String(cause);
}

/**
 * チャンネル管理操作（アーカイブ・キック・強制削除）の実行と通知をまとめる。
 *
 * 書き込みは `api/client.ts` を直接叩き、表示の更新は Electric の Shape に任せる
 * （`channels` / `channel_user` / `messages` の変更がそのまま流れる）。失敗は
 * `sonner` で通知し、呼び出し側は戻り値で確認ダイアログを閉じるか判断する。
 */
export function useChannelAdmin(channel: ChannelRow | null): ChannelAdmin {
  const access = useChannelAccess(channel);
  const client = useMemo(() => createConfiguredApiClient(getCollections().config), []);
  const [pending, setPending] = useState<ChannelAdminAction | null>(null);
  // 同じイベント内の連打は state 反映前なので、ref で同期的に弾く。
  const pendingRef = useRef<ChannelAdminAction | null>(null);

  async function run(
    action: ChannelAdminAction,
    request: () => Promise<unknown>,
    successMessage: string,
  ): Promise<boolean> {
    if (pendingRef.current) {
      return false;
    }
    pendingRef.current = action;
    setPending(action);
    try {
      await request();
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
        () =>
          client.post<unknown>(`/api/channels/${channelId}/${archived ? "archive" : "unarchive"}`),
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
        () => client.delete<unknown>(`/api/channels/${channelId}/users/${userId}`),
        "メンバーをキックしました",
      );
    },
    deleteMessage: async (messageId) =>
      run(
        "delete-message",
        () => client.delete<unknown>(`/api/messages/${messageId}`),
        "メッセージを削除しました",
      ),
  };
}
