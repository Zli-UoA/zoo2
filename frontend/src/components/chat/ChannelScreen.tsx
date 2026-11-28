import { useRouter } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";
import { ArchiveIcon, ArrowLeftIcon } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { useCurrentUser } from "../../hooks/useCurrentUser";
import {
  useActiveChannels,
  useAllUsers,
  useChannelAccess,
  useUsers,
} from "../../hooks/useRealtime";
import { useLocale } from "../i18n/LocaleProvider";
import { useShell } from "../layout/shell-context";
import { Button } from "../ui/button";
import { Skeleton } from "../ui/skeleton";
import { ChannelHeader } from "./ChannelHeader";
import { formatDmDisplayName, isGroupDm, resolveDmDisplayName } from "./dm";
import { withoutDeactivatedMembers } from "./memberCandidates";
import {
  channelLinkTargetFromHref,
  sameChannelLinkTarget,
  subscribeChannelLinkReveal,
} from "./messages/messageLink";
import { MessagePane } from "./messages/MessagePane";
import { ChannelNotesScreen } from "./notes/ChannelNotesScreen";
import { PaneHeader } from "../layout/PaneHeader";

const LOADING_ROWS = [0, 1, 2, 3];

function ChannelLoading() {
  return (
    <>
      <PaneHeader>
        <Skeleton className="size-8 rounded-lg" />
        <div className="flex flex-1 flex-col gap-1.5">
          <Skeleton className="h-4 w-40" />
          <Skeleton className="h-3 w-24" />
        </div>
      </PaneHeader>
      <div className="flex flex-1 flex-col gap-3 p-4">
        {LOADING_ROWS.map((row) => (
          <Skeleton key={row} className="h-14 w-full rounded-lg" />
        ))}
      </div>
    </>
  );
}

/** チャンネルルート。ヘッダとメッセージ（非公開は参加者だけ）を出す。 */
export function ChannelScreen({
  channelId,
  targetMessageId,
  targetThreadId,
}: {
  channelId: string;
  targetMessageId?: string;
  targetThreadId?: string;
}) {
  const router = useRouter({ warn: false });
  const { t } = useTranslation();
  const shell = useShell();
  const { locale } = useLocale();
  const { currentUserId, isLoading: currentUserLoading } = useCurrentUser();
  const channelsQuery = useActiveChannels();
  const usersQuery = useUsers();
  const channels = channelsQuery.data ?? [];
  const channel = useMemo(
    () => channels.find((candidate) => candidate.id === channelId) ?? null,
    [channels, channelId],
  );
  // 権限判定（閲覧可否・参加状態・保留中）は `useChannelAccess` に集約する。
  const access = useChannelAccess(channel);
  // 参加中・DM では共有ユーザーで足りる。未参加の公開チャンネルだけ人数判定に全件を使う。
  // 参加状態が確定する前は判定しない（loading 中に全件購読が始まらないようにする）。
  const needsAllUsers =
    !!channel && !access.isSyncing && !channel.is_dm && !channel.is_private && !access.isMember;
  const allUsersQuery = useAllUsers(needsAllUsers);
  const countUsers = needsAllUsers ? (allUsersQuery.data ?? []) : (usersQuery.data ?? []);
  const notesAvailable = !!channel && !channel.is_dm && !channel.encrypted && access.canRead;
  // ノートの書き込みは参加者かつ非アーカイブ。メッセージの posting_policy は適用しない。
  const canWriteNotes = !!channel && access.isMember && !access.isArchived;
  const [tab, setTab] = useState<"messages" | "notes">("messages");
  const [notesDirty, setNotesDirty] = useState(false);
  const [notesBusy, setNotesBusy] = useState(false);
  const previousTarget = useRef({ message: targetMessageId, thread: targetThreadId });
  const showMessages = useCallback(() => {
    if (notesBusy) return;
    if (tab === "notes" && notesDirty && !window.confirm(t("notes.leaveUnsaved"))) {
      return;
    }
    setNotesDirty(false);
    setTab("messages");
  }, [notesBusy, notesDirty, t, tab]);
  // チャンネルを移ったらメッセージへ戻す。ノートの下書きはノート画面の key で破棄する。
  useEffect(() => {
    setTab("messages");
    setNotesDirty(false);
    setNotesBusy(false);
  }, [channelId]);
  useEffect(() => {
    if (tab === "notes" && !notesAvailable) {
      setTab("messages");
      setNotesDirty(false);
      setNotesBusy(false);
    }
  }, [notesAvailable, tab]);
  // URL が変わらない同じメッセージ・スレッドリンクの通常左クリックでは、ルーターも blocker も動かない。
  // ノート表示中だけ、タブ移動と同じ確認・保存中ガードを通してメッセージへ戻す。
  useEffect(() => {
    return subscribeChannelLinkReveal((request) => {
      if (request.channelId !== channelId) return;
      const current = channelLinkTargetFromHref(
        router?.state.location.href,
        window.location.origin,
      );
      if (!current || !sameChannelLinkTarget(current, request)) return;
      if (tab !== "notes") return;
      showMessages();
    });
  }, [channelId, router, showMessages, tab]);
  // 同じチャンネル内で新しいメッセージやスレッドを開くルート移動が受理されたら、隠れた
  // メッセージペインのままにせずメッセージタブへ移る。ルートの承認（useBlocker）より
  // 前に発火する requestMessageJump は見ないので、拒否されたときはノートと下書きを保つ。
  useEffect(() => {
    const previous = previousTarget.current;
    previousTarget.current = { message: targetMessageId, thread: targetThreadId };
    if (!targetMessageId && !targetThreadId) return;
    if (previous.message === targetMessageId && previous.thread === targetThreadId) return;
    setNotesDirty(false);
    setNotesBusy(false);
    setTab("messages");
  }, [targetMessageId, targetThreadId]);
  const dm = useMemo(() => {
    if (!channel?.is_dm) {
      return null;
    }
    const resolved = resolveDmDisplayName({
      channelName: channel.channel_name,
      members: access.members,
      users: usersQuery.data ?? [],
      currentUserId,
      locale,
    });
    return {
      title: formatDmDisplayName(resolved, t),
      group: isGroupDm(channel.channel_name),
    };
  }, [access.members, channel, currentUserId, locale, t, usersQuery.data]);

  if (!channel) {
    if (channelsQuery.isInitialSyncLoading) {
      return (
        <div className="flex h-full min-h-0 flex-col">
          <ChannelLoading />
        </div>
      );
    }
    return (
      <div className="flex h-full items-center justify-center p-6">
        <div className="flex max-w-sm flex-col items-center gap-3 text-center">
          <p
            className={
              channelsQuery.isError ? "text-sm text-destructive" : "text-sm text-muted-foreground"
            }
          >
            {channelsQuery.isError ? t("channels.loadError") : t("channels.notFound")}
          </p>
          <Button variant="outline" size="sm" onClick={shell.showSidebar}>
            <ArrowLeftIcon />
            {t("channels.back")}
          </Button>
        </div>
      </div>
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      <ChannelHeader
        // チャンネルごとに作り直し、開いていた確認の画面（暗号化への切り替えなど）を移動先に持ち越さない。
        key={channel.id}
        channel={channel}
        title={dm?.title}
        subtitle={
          dm
            ? dm.group
              ? access.isLoading
                ? "…"
                : t("dms.memberCount", { count: access.members.length })
              : t("dms.direct")
            : undefined
        }
        memberCount={
          channel.is_dm
            ? undefined
            : access.canRead
              ? access.isLoading
                ? null
                : withoutDeactivatedMembers(access.members, countUsers).length
              : undefined
        }
      />
      {notesAvailable ? (
        <div
          role="tablist"
          aria-label={t("notes.tabs")}
          className="flex shrink-0 items-center gap-1 border-b px-2 py-1"
        >
          <button
            type="button"
            role="tab"
            id={`channel-messages-tab-${channel.id}`}
            aria-selected={tab === "messages"}
            aria-controls={`channel-messages-panel-${channel.id}`}
            className="rounded px-3 py-1.5 text-sm hover:bg-muted aria-selected:bg-muted aria-selected:font-medium"
            onClick={showMessages}
            disabled={notesBusy}
          >
            {t("notes.messagesTab")}
          </button>
          <button
            type="button"
            role="tab"
            id={`channel-notes-tab-${channel.id}`}
            aria-selected={tab === "notes"}
            aria-controls={`channel-notes-panel-${channel.id}`}
            className="rounded px-3 py-1.5 text-sm hover:bg-muted aria-selected:bg-muted aria-selected:font-medium"
            onClick={() => setTab("notes")}
          >
            {t("notes.title")}
          </button>
        </div>
      ) : null}
      <div className="flex min-h-0 flex-1 flex-col">
        <div
          className="flex min-h-0 flex-1 flex-col"
          hidden={tab === "notes"}
          id={`channel-messages-panel-${channel.id}`}
          role="tabpanel"
          aria-labelledby={`channel-messages-tab-${channel.id}`}
        >
          {access.isMembershipPending ? (
            <div className="flex flex-1 items-center justify-center">
              <p className="text-sm text-muted-foreground">{t("common.loading")}</p>
            </div>
          ) : access.canRead ? (
            <>
              {access.isArchived ? (
                <div className="flex items-start gap-2 border-b bg-muted/40 px-4 py-2 text-xs text-muted-foreground">
                  <ArchiveIcon className="mt-0.5 size-3.5 shrink-0" aria-hidden="true" />
                  <span>{t("moderation.archivedReadOnly")}</span>
                </div>
              ) : null}
              <div className="min-h-0 flex-1">
                <MessagePane
                  targetMessageId={targetMessageId}
                  key={channelId}
                  channelId={channelId}
                  isMember={access.isMember}
                  isActive={tab === "messages"}
                  canModerate={!channel.is_dm && access.canManageChannel}
                  readOnly={access.isArchived}
                  // 閲覧はできるまま、書き込みだけを止める（投稿権限は管理者・作成者のみ）。
                  // 現在のユーザー（管理者か）が確定するまでは止めない。
                  postingForbidden={
                    !currentUserLoading &&
                    channel.posting_policy === "managers" &&
                    !access.canManageChannel
                  }
                  encrypted={channel.encrypted === true}
                  membershipEpoch={channel.membership_epoch ?? null}
                  channelName={channel.channel_name}
                />
              </div>
            </>
          ) : (
            <div className="flex flex-1 items-center justify-center p-6">
              <p className="max-w-sm text-center text-sm text-muted-foreground">
                {t("channels.privateMessages")}
              </p>
            </div>
          )}
        </div>
        {tab === "notes" && notesAvailable ? (
          <div
            className="flex min-h-0 flex-1 flex-col"
            id={`channel-notes-panel-${channel.id}`}
            role="tabpanel"
            aria-labelledby={`channel-notes-tab-${channel.id}`}
          >
            <ChannelNotesScreen
              key={`${currentUserId ?? ""}:${channel.id}`}
              channelId={channel.id}
              currentUserId={currentUserId}
              canRead={access.canRead}
              canWrite={canWriteNotes}
              isMember={access.isMember}
              isArchived={access.isArchived}
              onDirtyChange={setNotesDirty}
              onBusyChange={setNotesBusy}
            />
          </div>
        ) : null}
      </div>
    </div>
  );
}
