import { useState, type FormEvent } from "react";

import { useWriteRunner } from "../hooks/useWriteRunner";
import { getCollections } from "../lib/electric/collections";
import type { ChannelUserRow } from "../lib/electric/types";
import type { ChannelUserWriteMetadata } from "../lib/electric/writes";
import { Button } from "./ui/button";
import { inputClassName } from "./ui/input-styles";

function nowIso(): string {
  return new Date().toISOString();
}

/**
 * チャンネルのメンバー表示と参加・退出・メンバー追加。
 *
 * メンバー（channel_user）と投稿者名（users）は親画面が購読して渡す。パネル自身は
 * 描画と書き込みだけを行う（チャンネル数分の live query を増やさないため）。
 */
export function ChannelMemberPanel({
  channelId,
  isPrivate,
  createdUserId,
  members,
  isMember,
  userNames,
  isLoading,
}: {
  channelId: string;
  isPrivate: boolean;
  createdUserId: string;
  members: ChannelUserRow[];
  isMember: boolean;
  userNames: Map<string, string>;
  isLoading: boolean;
}) {
  const { channelUsers, config } = getCollections();
  const { error, setError, run } = useWriteRunner();
  const [memberId, setMemberId] = useState("");

  const currentUserId = config.devUserId;
  // バックエンドの add_channel_user と同じ条件。作成者は常に追加可、非公開は
  // 作成者のみ、公開は作成者または既存メンバーが追加可。権限がないときは
  // 常に 403 になるフォームを出さない。
  const canAddMember = currentUserId === createdUserId || (!isPrivate && isMember);
  // 非公開チャンネルのメンバー一覧・参加状態は参加者にだけ表示する。
  const canViewMembers = !isPrivate || isMember;
  // メンバー読み込み中はガード（重複判定）が効かないため操作を止める。
  const busy = isLoading;

  async function joinChannel() {
    if (!currentUserId || busy) return;
    await run(() =>
      channelUsers.insert(
        { user_id: currentUserId, channel_id: channelId, joined_at: nowIso() },
        { metadata: { action: "join" } satisfies ChannelUserWriteMetadata },
      ),
    );
  }

  async function leaveChannel() {
    if (!currentUserId || busy) return;
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
    if (!user_id || busy) return;
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
      {canViewMembers ? (
        <>
          <div className="flex flex-wrap items-center gap-2 text-sm">
            <span className="rounded bg-muted px-2 py-1">
              メンバー: {isLoading ? "…" : `${members.length} 名`}
            </span>
            {currentUserId ? (
              isMember ? (
                <Button variant="outline" size="sm" onClick={leaveChannel} disabled={busy}>
                  退出
                </Button>
              ) : (
                <Button variant="outline" size="sm" onClick={joinChannel} disabled={busy}>
                  参加
                </Button>
              )
            ) : null}
          </div>

          <p className="text-xs text-muted-foreground">
            参加者:{" "}
            {members.length > 0
              ? members.map((member) => userNames.get(member.user_id) ?? member.user_id).join(", ")
              : "なし"}
          </p>

          {canAddMember ? (
            <form className="flex flex-wrap items-center gap-2" onSubmit={addMember}>
              <input
                className={inputClassName()}
                placeholder="追加するユーザー ID"
                value={memberId}
                onChange={(event) => setMemberId(event.target.value)}
                disabled={busy}
              />
              <Button type="submit" variant="outline" size="sm" disabled={busy}>
                メンバー追加
              </Button>
            </form>
          ) : null}
        </>
      ) : (
        <p className="text-xs text-muted-foreground">
          非公開チャンネルです。参加には作成者からの招待が必要です。
        </p>
      )}

      {error ? <p className="text-xs text-destructive">操作失敗: {error}</p> : null}
    </div>
  );
}
