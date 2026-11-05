import { Input } from "../ui/input";
import { useTranslation } from "react-i18next";
import { cn } from "cn";
import { ArrowLeftIcon, SearchIcon, UserMinusIcon } from "lucide-react";
import { Fragment, useId, useMemo, useState, type ChangeEvent, type KeyboardEvent } from "react";

import { canKickChannelMember } from "../../hooks/channelPermissions";
import { useChannelAdmin } from "../../hooks/useChannelAdmin";
import { useCurrentUser } from "../../hooks/useCurrentUser";
import { useChannelMembers, useUsers, useWorkspaceAdminIds } from "../../hooks/useRealtime";
import { useWriteRunner } from "../../hooks/useWriteRunner";
import { useCollections } from "../../lib/electric/collections-context";
import type { ChannelUserWriteMetadata } from "../../lib/electric/writes";
import type { ChannelRow, UserRow } from "../../lib/electric/types";
import { useLocale } from "../i18n/LocaleProvider";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "../ui/alert-dialog";
import { Avatar, AvatarFallback } from "../ui/avatar";
import { Button } from "../ui/button";
import { inputClassName } from "../ui/input-styles";
import { ScrollArea } from "../ui/scroll-area";
import { Skeleton } from "../ui/skeleton";
import { initials } from "./format";
import { AppAvatar, openApp } from "./AppIdentity";
import { PaneHeader } from "../layout/PaneHeader";
import {
  filterMemberCandidates,
  sortChannelMembers,
  sortUsersByLabel,
  userLabel,
} from "./memberCandidates";

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
 *
 * メンバー追加は生のユーザー id を打ち込むのではなく、`users` から表示名 /
 * ユーザー名で絞り込んだ候補を選ぶ combobox にしている。選択した時点で
 * `channel_user` に追加する（重複ガードは既存どおり残す）。
 */
export function MembersPanel({
  channel,
  className,
  onBack,
}: {
  channel: ChannelRow;
  className?: string;
  onBack?: () => void;
}) {
  const { t } = useTranslation();
  const { locale } = useLocale();
  const { channelUsers } = useCollections();
  const membersQuery = useChannelMembers(channel.id);
  const usersQuery = useUsers();
  const { currentUserId, isLoading: currentUserLoading } = useCurrentUser();
  const { error, setError, run } = useWriteRunner();
  const admin = useChannelAdmin(channel);
  const { adminIds, isLoading: adminIdsLoading } = useWorkspaceAdminIds();
  const isArchived = admin.access.isArchived;
  const isCreator = admin.access.isCreator;
  const canManage = admin.access.canManageChannel;
  const [query, setQuery] = useState("");
  const [isCandidateListOpen, setCandidateListOpen] = useState(false);
  const [activeIndex, setActiveIndex] = useState(-1);
  const [kickTarget, setKickTarget] = useState<{ userId: string; label: string } | null>(null);
  const candidateListId = useId();
  const creatorLeaveHintId = useId();

  const userMap = new Map((usersQuery.data ?? []).map((user) => [user.id, user]));
  const members = useMemo(
    () =>
      sortChannelMembers(membersQuery.data ?? [], usersQuery.data ?? [], channel.created_user_id),
    [membersQuery.data, usersQuery.data, channel.created_user_id],
  );
  const userNames = useMemo(() => {
    const map = new Map<string, string>();
    for (const user of usersQuery.data ?? []) {
      map.set(user.id, userLabel(user));
    }
    return map;
  }, [usersQuery.data]);

  // 並べ替えはユーザー一覧が変わったときだけ行い、キー入力ごとには絞り込みだけを行う。
  const sortedUsers = useMemo(
    () => sortUsersByLabel(usersQuery.data ?? [], locale),
    [usersQuery.data, locale],
  );
  const candidates = useMemo(
    () => filterMemberCandidates({ users: sortedUsers, members, query }),
    [sortedUsers, members, query],
  );

  const isMember = currentUserId
    ? members.some((member) => member.user_id === currentUserId)
    : false;
  const canAddMember = admin.access.canAddMember && !isArchived;
  const canViewMembers = !channel.is_private || isMember;
  const busy = membersQuery.isLoading || admin.pending !== null;

  function candidateOptionId(userId: string): string {
    return `${candidateListId}-option-${userId}`;
  }

  function openCandidateList() {
    setCandidateListOpen(true);
    setActiveIndex(candidates.length > 0 ? 0 : -1);
  }

  function closeCandidateList() {
    setCandidateListOpen(false);
    setActiveIndex(-1);
  }

  function handleQueryChange(event: ChangeEvent<HTMLInputElement>) {
    setQuery(event.target.value);
    setCandidateListOpen(true);
    setActiveIndex(0);
  }

  function handleQueryKeyDown(event: KeyboardEvent<HTMLInputElement>) {
    if (!isCandidateListOpen) {
      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        event.preventDefault();
        openCandidateList();
      }
      return;
    }
    switch (event.key) {
      case "ArrowDown":
        event.preventDefault();
        setActiveIndex((index) => Math.min(index + 1, candidates.length - 1));
        break;
      case "ArrowUp":
        event.preventDefault();
        setActiveIndex((index) => Math.max(index - 1, 0));
        break;
      case "Home":
        event.preventDefault();
        setActiveIndex(candidates.length > 0 ? 0 : -1);
        break;
      case "End":
        event.preventDefault();
        setActiveIndex(candidates.length - 1);
        break;
      case "Enter": {
        const candidate = candidates[activeIndex];
        if (candidate) {
          event.preventDefault();
          void addMember(candidate);
        }
        break;
      }
      case "Escape":
        // ポップアップだけを閉じ、ダイアログ（Base UI が document で Escape を
        // 監視する）まで閉じないよう伝播を止める。閉じた状態で再度 Escape を
        // 押すと、通常どおりダイアログが閉じる。
        event.preventDefault();
        event.stopPropagation();
        closeCandidateList();
        break;
      case "Tab":
        closeCandidateList();
        break;
    }
  }

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
    if (!currentUserId || busy || isCreator) return;
    const userId = currentUserId;
    await run(() =>
      channelUsers.delete(`${userId}:${channel.id}`, {
        metadata: { action: "leave" } satisfies ChannelUserWriteMetadata,
      }),
    );
  }

  async function confirmKick() {
    if (!kickTarget) return;
    await admin.kickMember(kickTarget.userId);
    // 権限判定の結果が古い（管理者一覧の取得失敗など）場合でも、403 のあとに
    // ダイアログを残さない。失敗理由は useChannelAdmin がトーストで通知する。
    setKickTarget(null);
  }

  function handlePanelKeyDown(event: KeyboardEvent<HTMLElement>) {
    if (event.key !== "Escape" || !isCandidateListOpen) {
      return;
    }
    // 入力欄以外にフォーカスがあっても、まず候補ポップアップを閉じる。
    event.preventDefault();
    event.stopPropagation();
    closeCandidateList();
  }

  async function addMember(user: UserRow) {
    if (busy) return;
    if (members.some((member) => member.user_id === user.id)) {
      // 既存メンバーの追加はサーバが既存行を返すだけで Electric に変更が流れず、
      // awaitTxId がタイムアウトする。UI 側で弾く（Phase 3 のガードを踏襲）。
      setError(t("members.already", { userId: userLabel(user) }));
      return;
    }
    setQuery("");
    closeCandidateList();
    await run(() =>
      channelUsers.insert(
        { user_id: user.id, channel_id: channel.id, joined_at: nowIso() },
        { metadata: { action: "add", user_id: user.id } satisfies ChannelUserWriteMetadata },
      ),
    );
  }

  return (
    <section
      role={onBack ? "region" : undefined}
      aria-label={onBack ? t("members.title") : undefined}
      className={cn("min-h-0 flex-col bg-background", className)}
      onKeyDown={handlePanelKeyDown}
    >
      <PaneHeader className="justify-between gap-2">
        {onBack ? (
          <>
            <Button
              autoFocus
              variant="ghost"
              size="icon-lg"
              onClick={onBack}
              aria-label={t("members.back")}
            >
              <ArrowLeftIcon />
            </Button>
            <div className="min-w-0 flex-1">
              <h1 className="text-sm font-semibold">{t("members.title")}</h1>
              <p className="truncate text-xs text-muted-foreground">#{channel.channel_name}</p>
            </div>
          </>
        ) : (
          <h3 className="text-sm font-semibold">{t("members.title")}</h3>
        )}
        <span className="rounded bg-muted px-2 py-0.5 text-xs tabular-nums text-muted-foreground">
          {!canViewMembers
            ? "—"
            : membersQuery.isLoading
              ? "…"
              : members.filter((member) => !userMap.get(member.user_id)?.is_bot).length}
        </span>
      </PaneHeader>

      {canViewMembers ? (
        <>
          {isArchived ? (
            <p className="border-b bg-muted/40 px-4 py-2 text-xs text-muted-foreground">
              {t("moderation.archivedMembersHelp")}
            </p>
          ) : null}
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
                    const bot = userMap.get(member.user_id);
                    const isBot = !!bot?.is_bot;
                    const label = userNames.get(member.user_id) ?? member.user_id;
                    const isCreator = member.user_id === channel.created_user_id;
                    const isMe = member.user_id === currentUserId;
                    const canKick =
                      !isArchived &&
                      !adminIdsLoading &&
                      canKickChannelMember({
                        actorId: currentUserId,
                        actorCanManage: canManage,
                        channelCreatorId: channel.created_user_id,
                        targetUserId: member.user_id,
                        targetIsAdmin: adminIds.has(member.user_id),
                      });
                    return (
                      <Fragment key={member.user_id}>
                        <li className="flex items-center gap-2.5 rounded-lg px-2 py-1.5 hover:bg-accent/60">
                          <AppAvatar label={label} isBot={isBot} src={bot?.avatar_url} size="sm" />
                          <span className="min-w-0 flex-1 truncate text-sm">
                            {isBot && bot?.app_id ? (
                              <button type="button" onClick={() => openApp(bot.app_id!)}>
                                {label} <span className="text-xs text-muted-foreground">APP</span>
                              </button>
                            ) : (
                              label
                            )}
                          </span>
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
                          {canKick ? (
                            <Button
                              variant="ghost"
                              size="icon-xs"
                              className="shrink-0 text-destructive hover:bg-destructive/10 hover:text-destructive"
                              aria-label={t("moderation.kickLabel", { name: label })}
                              title={t("moderation.kick")}
                              disabled={busy}
                              onClick={() => setKickTarget({ userId: member.user_id, label })}
                            >
                              <UserMinusIcon />
                            </Button>
                          ) : null}
                        </li>
                      </Fragment>
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
              <div className="relative">
                <SearchIcon
                  aria-hidden="true"
                  className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground"
                />
                <Input
                  className={cn(inputClassName(), "w-full pl-8")}
                  placeholder={t("members.search")}
                  value={query}
                  onChange={handleQueryChange}
                  onKeyDown={handleQueryKeyDown}
                  // ダイアログを開いた直後の自動フォーカスでは候補を開かない。
                  // 入力（onChange）か明示的なクリック、矢印キーで開く。
                  onClick={openCandidateList}
                  onBlur={closeCandidateList}
                  disabled={busy}
                  role="combobox"
                  aria-label={t("members.addLabel")}
                  aria-expanded={isCandidateListOpen}
                  aria-controls={candidateListId}
                  aria-autocomplete="list"
                  aria-activedescendant={
                    isCandidateListOpen && candidates[activeIndex]
                      ? candidateOptionId(candidates[activeIndex].id)
                      : undefined
                  }
                  autoComplete="off"
                />
                {isCandidateListOpen ? (
                  <ul
                    id={candidateListId}
                    role="listbox"
                    aria-label={t("members.candidates")}
                    className="absolute inset-x-0 bottom-full z-20 mb-1 max-h-64 overflow-y-auto rounded-lg border bg-popover p-1 shadow-md"
                  >
                    {usersQuery.isLoading ? (
                      <li role="presentation" className="px-2 py-1.5 text-xs text-muted-foreground">
                        {t("members.usersLoading")}
                      </li>
                    ) : usersQuery.isError ? (
                      <li role="presentation" className="px-2 py-1.5 text-xs text-destructive">
                        {t("members.usersError")}
                      </li>
                    ) : candidates.length === 0 ? (
                      <li role="presentation" className="px-2 py-1.5 text-xs text-muted-foreground">
                        {t("members.usersEmpty")}
                      </li>
                    ) : (
                      candidates.map((user, index) => {
                        const label = userLabel(user);
                        const active = index === activeIndex;
                        return (
                          <li
                            key={user.id}
                            id={candidateOptionId(user.id)}
                            role="option"
                            aria-selected={active}
                            className={cn(
                              "flex cursor-pointer items-center gap-2.5 rounded-md px-2 py-1.5 text-sm",
                              active ? "bg-accent text-accent-foreground" : "hover:bg-accent/60",
                            )}
                            // クリックで入力欄のフォーカスを失わないようにする
                            // （blur で閉じる前に選択を確定させる）。
                            onMouseDown={(event) => event.preventDefault()}
                            onMouseEnter={() => setActiveIndex(index)}
                            onClick={() => void addMember(user)}
                          >
                            <Avatar size="sm">
                              <AvatarFallback>{initials(label)}</AvatarFallback>
                            </Avatar>
                            <span className="min-w-0 flex-1 truncate">{label}</span>
                            {label !== user.user_name ? (
                              // ユーザー名が長い場合でも表示名を潰さないよう、
                              // 幅を抑えて省略する。
                              <span className="max-w-[40%] shrink-0 truncate text-xs text-muted-foreground">
                                @{user.user_name}
                              </span>
                            ) : null}
                          </li>
                        );
                      })
                    )}
                  </ul>
                ) : null}
              </div>
            ) : null}
            <div className="flex items-center justify-between gap-2">
              <span className="text-xs text-muted-foreground">
                {currentUserLoading
                  ? t("members.checking")
                  : isMember
                    ? isCreator
                      ? t("members.creator")
                      : t("members.joined")
                    : isArchived
                      ? t("moderation.archived")
                      : t("members.notJoined")}
              </span>
              {!currentUserLoading && currentUserId ? (
                isMember ? (
                  <>
                    <span title={isCreator ? t("moderation.forbidden") : undefined}>
                      <Button
                        variant="outline"
                        size="sm"
                        onClick={leaveChannel}
                        disabled={busy || isCreator}
                        aria-describedby={isCreator ? creatorLeaveHintId : undefined}
                      >
                        {t("members.leave")}
                      </Button>
                    </span>
                    {isCreator ? (
                      <span id={creatorLeaveHintId} className="sr-only">
                        {t("moderation.forbidden")}
                      </span>
                    ) : null}
                  </>
                ) : isArchived ? null : (
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

      <AlertDialog
        open={kickTarget !== null}
        onOpenChange={(open) => {
          if (!open) {
            setKickTarget(null);
          }
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("moderation.kickTitle")}</AlertDialogTitle>
            <AlertDialogDescription>
              {kickTarget ? t("moderation.kickDescription", { name: kickTarget.label }) : ""}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancelShort")}</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              disabled={busy}
              onClick={() => void confirmKick()}
            >
              {admin.pending === "kick" ? t("moderation.executing") : t("moderation.kick")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </section>
  );
}
