import { useMemo, useState, type FormEvent } from "react";

import { useChannelMembers, useUsers } from "../hooks/useRealtime";
import { useWriteRunner } from "../hooks/useWriteRunner";
import { getCollections } from "../lib/electric/collections";
import type { ChannelUserWriteMetadata } from "../lib/electric/writes";
import { Button } from "./ui/button";
import { inputClassName } from "./ui/input";

function nowIso(): string {
  return new Date().toISOString();
}

/**
 * チャンネルのメンバー表示と参加・退出・メンバー追加。
 *
 * `useChannelMembers` で channel_user コレクションを購読するため、この画面を
 * 表示している間は channel_user の書き込みに対する `awaitTxId` の対象が揃う。
 */
export function ChannelMemberPanel({
  channelId,
  isPrivate,
}: {
  channelId: string;
  isPrivate: boolean;
}) {
  const { channelUsers, config } = getCollections();
  const membersQuery = useChannelMembers(channelId);
  const usersQuery = useUsers();
  const { error, setError, run } = useWriteRunner();
  const [memberId, setMemberId] = useState("");

  const members = membersQuery.data ?? [];
  const userNames = useMemo(() => {
    const map = new Map<string, string>();
    for (const user of usersQuery.data ?? []) {
      map.set(user.id, user.display_name || user.user_name);
    }
    return map;
  }, [usersQuery.data]);

  const currentUserId = config.devUserId;
  const isMember = currentUserId
    ? members.some((member) => member.user_id === currentUserId)
    : false;

  async function joinChannel() {
    if (!currentUserId) return;
    await run(() =>
      channelUsers.insert(
        { user_id: currentUserId, channel_id: channelId, joined_at: nowIso() },
        { metadata: { action: "join" } satisfies ChannelUserWriteMetadata },
      ),
    );
  }

  async function leaveChannel() {
    if (!currentUserId) return;
    const userId = currentUserId;
    await run(() =>
      channelUsers.delete(`${userId}:${channelId}`, {
        metadata: { action: "leave" } satisfies ChannelUserWriteMetadata,
      }),
    );
  }

  async function addMember(event: FormEvent) {
    event.preventDefault();
    const user_id = memberId.trim();
    if (!user_id) return;
    if (members.some((member) => member.user_id === user_id)) {
      // 既存メンバーの追加はサーバが既存行を返すだけで Electric に変更が流れず、
      // awaitTxId がタイムアウトする。UI 側で弾く（Phase 3 のガードを踏襲）。
      setError(`既にメンバーです: ${user_id}`);
      return;
    }
    setMemberId("");
    await run(() =>
      channelUsers.insert(
        { user_id, channel_id: channelId, joined_at: nowIso() },
        { metadata: { action: "add", user_id } satisfies ChannelUserWriteMetadata },
      ),
    );
  }

  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-wrap items-center gap-2 text-sm">
        <span className="rounded bg-muted px-2 py-1">
          メンバー: {membersQuery.isLoading ? "…" : `${members.length} 名`}
        </span>
        {currentUserId ? (
          isMember ? (
            <Button variant="outline" size="sm" onClick={leaveChannel}>
              退出
            </Button>
          ) : isPrivate ? (
            <span className="text-xs text-muted-foreground">非公開（招待待ち）</span>
          ) : (
            <Button variant="outline" size="sm" onClick={joinChannel}>
              参加
            </Button>
          )
        ) : null}
        {membersQuery.isError ? <span className="text-destructive">メンバー取得に失敗</span> : null}
      </div>

      <p className="text-xs text-muted-foreground">
        参加者:{" "}
        {members.length > 0
          ? members.map((member) => userNames.get(member.user_id) ?? member.user_id).join(", ")
          : "なし"}
      </p>

      <form className="flex flex-wrap items-center gap-2" onSubmit={addMember}>
        <input
          className={inputClassName()}
          placeholder="追加するユーザー ID"
          value={memberId}
          onChange={(event) => setMemberId(event.target.value)}
        />
        <Button type="submit" variant="outline" size="sm">
          メンバー追加
        </Button>
      </form>

      {error ? <p className="text-xs text-destructive">操作失敗: {error}</p> : null}
    </div>
  );
}
