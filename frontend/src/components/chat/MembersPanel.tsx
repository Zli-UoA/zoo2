import { useTranslation } from "react-i18next";
import { cn } from "cn";
import { UserPlusIcon } from "lucide-react";
import { useMemo, useState, type FormEvent } from "react";

import { useCurrentUser } from "../../hooks/useCurrentUser";
import { useChannelMembers, useUsers } from "../../hooks/useRealtime";
import { useWriteRunner } from "../../hooks/useWriteRunner";
import { getCollections } from "../../lib/electric/collections";
import type { ChannelUserWriteMetadata } from "../../lib/electric/writes";
import type { ChannelRow } from "../../lib/electric/types";
import { Avatar, AvatarFallback } from "../ui/avatar";
import { Button } from "../ui/button";
import { inputClassName } from "../ui/input-styles";
import { ScrollArea } from "../ui/scroll-area";
import { Skeleton } from "../ui/skeleton";
import { initials } from "./format";

const SKELETON_ROWS = [0, 1, 2];

function nowIso(): string {
  return new Date().toISOString();
}

/**
 * チャンネルのメンバー一覧と参加・退出・追加。
 *
 * `channel_user` の Shape は `getCollections()` が 1 本だけ作るため、パネルが複数
 * あっても購読は増えない。権限はバックエンドの `add_channel_user` / `leave_channel`
 * と同じ条件を UI 側でも守る（非公開は参加者のみ閲覧、追加は作成者か公開の既存
 * メンバー、退出は本人のみ）。
 */
export function MembersPanel({ channel, className }: { channel: ChannelRow; className?: string }) {
  const { t } = useTranslation();
  const { channelUsers } = getCollections();
  const membersQuery = useChannelMembers(channel.id);
  const usersQuery = useUsers();
  const { currentUserId, isLoading: currentUserLoading } = useCurrentUser();
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

  const isMember = currentUserId
    ? members.some((member) => member.user_id === currentUserId)
    : false;
  const canAddMember =
    currentUserId === channel.created_user_id || (!channel.is_private && isMember);
  const canViewMembers = !channel.is_private || isMember;
  const busy = membersQuery.isLoading;

  async function joinChannel() {
    if (!currentUserId || busy) return;
    await run(() =>
      channelUsers.insert(
        { user_id: currentUserId, channel_id: channel.id, joined_at: nowIso() },
        { metadata: { action: "join" } satisfies ChannelUserWriteMetadata },
      ),
    );
  }

  async function leaveChannel() {
    if (!currentUserId || busy) return;
    const userId = currentUserId;
    await run(() =>
      channelUsers.delete(`${userId}:${channel.id}`, {
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
      setError(t("members.already", { userId: user_id }));
      return;
    }
    setMemberId("");
    await run(() =>
      channelUsers.insert(
        { user_id, channel_id: channel.id, joined_at: nowIso() },
        { metadata: { action: "add", user_id } satisfies ChannelUserWriteMetadata },
      ),
    );
  }

  return (
    <div className={cn("min-h-0 flex-col bg-background", className)}>
      <header className="flex items-center justify-between gap-2 border-b px-4 py-3">
        <h3 className="text-sm font-semibold">{t("members.title")}</h3>
        <span className="rounded bg-muted px-2 py-0.5 text-xs tabular-nums text-muted-foreground">
          {!canViewMembers ? "—" : membersQuery.isLoading ? "…" : members.length}
        </span>
      </header>

      {canViewMembers ? (
        <>
          <ScrollArea className="min-h-0 flex-1">
            <ul className="flex flex-col gap-0.5 p-2">
              {membersQuery.isLoading
                ? SKELETON_ROWS.map((row) => (
                    <li key={row} className="flex items-center gap-2.5 px-2 py-1.5">
                      <Skeleton className="size-6 rounded-full" />
                      <Skeleton className="h-4 flex-1" />
                    </li>
                  ))
                : members.map((member) => {
                    const label = userNames.get(member.user_id) ?? member.user_id;
                    const isCreator = member.user_id === channel.created_user_id;
                    const isMe = member.user_id === currentUserId;
                    return (
                      <li
                        key={member.user_id}
                        className="flex items-center gap-2.5 rounded-lg px-2 py-1.5 hover:bg-accent/60"
                      >
                        <Avatar size="sm">
                          <AvatarFallback>{initials(label)}</AvatarFallback>
                        </Avatar>
                        <span className="min-w-0 flex-1 truncate text-sm">{label}</span>
                        {isCreator ? (
                          <span className="shrink-0 rounded bg-muted px-1.5 py-0.5 text-[0.65rem] text-muted-foreground">
                            {t("members.creator")}
                          </span>
                        ) : null}
                        {isMe ? (
                          <span className="shrink-0 rounded bg-brand-muted px-1.5 py-0.5 text-[0.65rem] text-brand-muted-foreground">
                            {t("members.you")}
                          </span>
                        ) : null}
                      </li>
                    );
                  })}
            </ul>
            {!membersQuery.isLoading && members.length === 0 ? (
              <p className="px-4 py-3 text-xs text-muted-foreground">{t("members.empty")}</p>
            ) : null}
            {membersQuery.isError ? (
              <p className="px-4 py-3 text-xs text-destructive">{t("members.loadError")}</p>
            ) : null}
          </ScrollArea>

          <div className="flex flex-col gap-2 border-t p-3">
            {canAddMember ? (
              <form className="flex items-center gap-2" onSubmit={addMember}>
                <input
                  className={cn(inputClassName(), "min-w-0 flex-1")}
                  placeholder={t("members.userId")}
                  value={memberId}
                  onChange={(event) => setMemberId(event.target.value)}
                  disabled={busy}
                  aria-label={t("members.userId")}
                />
                <Button type="submit" variant="outline" size="sm" disabled={busy}>
                  <UserPlusIcon />
                  {t("members.add")}
                </Button>
              </form>
            ) : null}
            <div className="flex items-center justify-between gap-2">
              <span className="text-xs text-muted-foreground">
                {currentUserLoading
                  ? t("members.checking")
                  : isMember
                    ? t("members.joined")
                    : t("members.notJoined")}
              </span>
              {!currentUserLoading && currentUserId ? (
                isMember ? (
                  <Button variant="outline" size="sm" onClick={leaveChannel} disabled={busy}>
                    {t("members.leave")}
                  </Button>
                ) : (
                  <Button size="sm" onClick={joinChannel} disabled={busy}>
                    {t("members.join")}
                  </Button>
                )
              ) : null}
            </div>
            {error ? (
              <p className="text-xs text-destructive">{t("common.writeErrorDetail", { error })}</p>
            ) : null}
          </div>
        </>
      ) : (
        <div className="flex flex-1 items-center justify-center p-6">
          <p className="text-center text-xs text-muted-foreground">{t("members.private")}</p>
        </div>
      )}
    </div>
  );
}
