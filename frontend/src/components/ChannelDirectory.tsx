import { Link } from "@tanstack/react-router";
import { useMemo, useState, type FormEvent } from "react";

import { useActiveChannels, useAllChannelMembers, useUsers } from "../hooks/useRealtime";
import { useWriteRunner } from "../hooks/useWriteRunner";
import { getCollections } from "../lib/electric/collections";
import type { ChannelUserRow } from "../lib/electric/types";
import { ChannelMemberPanel } from "./ChannelMemberPanel";
import { SyncStatus } from "./SyncStatus";
import { Button } from "./ui/button";
import { inputClassName } from "./ui/input-styles";

function nowIso(): string {
  return new Date().toISOString();
}

/** トップ画面: チャンネルの一覧・作成と、各チャンネルのメンバー管理。 */
export function ChannelDirectory() {
  const { channels: channelsCollection, config } = getCollections();
  const channelsQuery = useActiveChannels();
  const membersQuery = useAllChannelMembers();
  const usersQuery = useUsers();
  const channels = channelsQuery.data ?? [];
  const currentUserId = config.devUserId;
  const { error, run } = useWriteRunner();
  const [newChannelName, setNewChannelName] = useState("");
  const [newChannelPrivate, setNewChannelPrivate] = useState(false);

  const userNames = useMemo(() => {
    const map = new Map<string, string>();
    for (const user of usersQuery.data ?? []) {
      map.set(user.id, user.display_name || user.user_name);
    }
    return map;
  }, [usersQuery.data]);

  // channel_user を 1 回だけ購読し、チャンネル単位にまとめて各パネルへ渡す。
  const membersByChannel = useMemo(() => {
    const map = new Map<string, ChannelUserRow[]>();
    for (const member of membersQuery.data ?? []) {
      const list = map.get(member.channel_id);
      if (list) {
        list.push(member);
      } else {
        map.set(member.channel_id, [member]);
      }
    }
    return map;
  }, [membersQuery.data]);

  async function createChannel(event: FormEvent) {
    event.preventDefault();
    const channel_name = newChannelName.trim();
    if (!channel_name) return;
    setNewChannelName("");
    await run(() =>
      channelsCollection.insert({
        id: crypto.randomUUID(),
        channel_name,
        description: null,
        created_user_id: currentUserId ?? "",
        is_private: newChannelPrivate,
        created_at: nowIso(),
        updated_at: null,
        archive_at: null,
        deleted_at: null,
      }),
    );
  }

  return (
    <section className="flex flex-col gap-6">
      <header className="flex flex-col gap-2">
        <h1 className="text-2xl font-bold">チャンネル</h1>
        <SyncStatus />
      </header>

      <form className="flex flex-wrap items-center gap-2" onSubmit={createChannel}>
        <input
          className={inputClassName()}
          placeholder="新しいチャンネル名"
          value={newChannelName}
          onChange={(event) => setNewChannelName(event.target.value)}
        />
        <label className="flex items-center gap-1 text-sm">
          <input
            type="checkbox"
            checked={newChannelPrivate}
            onChange={(event) => setNewChannelPrivate(event.target.checked)}
          />
          非公開
        </label>
        <Button type="submit" size="sm">
          チャンネル作成
        </Button>
      </form>

      {error ? <p className="text-sm text-destructive">操作失敗: {error}</p> : null}
      {channelsQuery.isError ? (
        <p className="text-sm text-destructive">チャンネル取得に失敗: {channelsQuery.status}</p>
      ) : null}
      {channels.length === 0 && !channelsQuery.isLoading ? (
        <p className="text-sm text-muted-foreground">チャンネルがありません。</p>
      ) : null}

      <ul className="flex flex-col gap-4">
        {channels.map((channel) => {
          const members = membersByChannel.get(channel.id) ?? [];
          const isMember = currentUserId
            ? members.some((member) => member.user_id === currentUserId)
            : false;
          // 非公開チャンネルの説明・メンバーは参加者にだけ表示する。
          const canViewMembers = !channel.is_private || isMember;
          return (
            <li key={channel.id} className="flex flex-col gap-3 rounded-lg border p-4">
              <div className="flex flex-wrap items-baseline justify-between gap-2">
                <Link
                  to="/channels/$channelId"
                  params={{ channelId: channel.id }}
                  className="text-lg font-medium underline-offset-4 hover:underline"
                >
                  {channel.is_private ? "🔒 " : "# "}
                  {channel.channel_name}
                </Link>
                {canViewMembers ? (
                  <Link
                    to="/channels/$channelId"
                    params={{ channelId: channel.id }}
                    className="text-xs text-muted-foreground underline-offset-4 hover:underline"
                  >
                    メッセージを開く
                  </Link>
                ) : null}
              </div>
              {channel.description && canViewMembers ? (
                <p className="text-sm text-muted-foreground">{channel.description}</p>
              ) : null}
              <ChannelMemberPanel
                channelId={channel.id}
                isPrivate={channel.is_private}
                createdUserId={channel.created_user_id}
                members={members}
                isMember={isMember}
                userNames={userNames}
                isLoading={membersQuery.isLoading}
              />
            </li>
          );
        })}
      </ul>
    </section>
  );
}
