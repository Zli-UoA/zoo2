import { useTranslation } from "react-i18next";
import { ArrowLeftIcon, UsersIcon, XIcon } from "lucide-react";
import { type KeyboardEvent, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";

import type { CommandSuggestion } from "./composerSuggestions";
import type { UploadedAttachment } from "../../../lib/api/attachments";
import type { KitchenCatalog } from "../../../lib/emoji/kitchen";
import type { AttachmentRow } from "../../../lib/electric/types";
import { groupReactions, type ReactionGroup } from "../../../lib/emoji/reactions";
import { Button } from "../../ui/button";
import type { ThreadParticipant } from "../../../hooks/useThreadMembership";
import { PaneHeader } from "../../layout/PaneHeader";
import { useMediaQuery } from "../../layout/useMediaQuery";

import type { TypingUser } from "../../../hooks/useTyping";
import { TypingIndicator } from "./TypingIndicator";
import { Composer } from "./Composer";
import { JoinPrompt } from "./JoinPrompt";
import {
  ThreadParticipantsDialog,
  type ThreadParticipantsAction,
  type ThreadParticipantsStatus,
} from "./ThreadParticipantsDialog";
import { MessageRow } from "./MessageRow";
import type { FeedMessage } from "./messageFormat";
import { buildMessageItems } from "./messageGrouping";
import { isNearBottom } from "./messageScroll";

const FOCUSABLE_SELECTOR =
  'a[href], button:not([disabled]), input:not([disabled]), textarea:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex="-1"])';

export interface ThreadPaneProps {
  commands?: readonly CommandSuggestion[];
  onRequestCommands?: () => void;
  targetMessageId?: string;
  mentionUsers?: import("../../../lib/electric/types").UserRow[];
  mentionMemberIds?: ReadonlySet<string>;
  isMember?: boolean;
  /** オフラインか。返信がないときの案内と composer を切り替える。 */
  isOffline?: boolean;
  /** 返信を送れない理由（暗号化チャンネルで暗号化の送信がまだ使えない、など）。指定すると入力欄を止める。 */
  composeDisabledReason?: string;
  /** 投稿権限がない。閲覧と参加はでき、返信・再Openだけを止める（理由は `composeDisabledReason`）。 */
  postingForbidden?: boolean;
  typingUsers?: TypingUser[];
  onTyping?: (active: boolean) => void;
  root: FeedMessage;
  replies: FeedMessage[];
  membership?: "none" | "joined" | "left";
  membershipPending?: boolean;
  membershipError?: boolean;
  /** 参加状態の初回取得中。確定するまで入力を止める（未参加で送れてしまうのを防ぐ）。 */
  membershipLoading?: boolean;
  /** 参加中の人数と一覧（先頭から最大50人）。 */
  participantCount?: number;
  participants?: ThreadParticipant[];
  onMembershipRetry?: () => void;
  onJoinThread?: () => void;
  onLeaveThread?: () => void;
  onReopenThread?: () => void;
  /** アーカイブ済みを再Openして参加する（参加ボタンの代わりに出す）。 */
  onReopenAndJoinThread?: () => void;
  isLoading: boolean;
  isError: boolean;
  /**
   * 編集・削除・リアクションなどの操作に使う自分の ID。アーカイブ済み・読み取り専用のときは、
   * 操作を止めるために呼び出し元が null にする。
   */
  currentUserId: string | null;
  /**
   * 本人の判定（メンバーのダイアログの「あなた」の印）に使う閲覧者の ID。操作可否では null にしない。
   * 省略時は `currentUserId` を使う。
   */
  viewerId?: string | null;
  canModerate?: boolean;
  isArchived?: boolean;
  readOnly?: boolean;
  /** 親が削除されている（検索・リンクから返信を開いたとき）。親の代わりに知らせを出し、読むだけにする。 */
  rootDeleted?: boolean;
  reactions: Map<string, ReactionGroup[]>;
  attachments?: Map<string, AttachmentRow[]>;
  kitchenCatalog: KitchenCatalog | null;
  canReact: boolean;
  onReaction: (messageId: string, emoji: string) => void;
  onPickReaction: (messageId: string) => void;
  editingId: string | null;
  editingText: string;
  onEditingTextChange: (value: string) => void;
  onStartEdit: (id: string, content: string) => void;
  onCancelEdit: () => void;
  onSaveEdit: () => void;
  onRequestDelete: (id: string) => void;
  onUploadAttachment?: (file: File) => Promise<UploadedAttachment>;
  onSendReply: (
    content: string,
    commandAppId?: string,
    attachmentIds?: string[],
    sensitiveAttachmentIds?: string[],
  ) => Promise<boolean>;
  /** 上端付近で古い返信を読み足す。 */
  onLoadOlder?: () => void;
  /** 端末より古い返信が、まだあるか。 */
  hasOlder?: boolean;
  /** 古いページを取っている最中か。 */
  isLoadingOlder?: boolean;
  /** 取得失敗時の再試行。 */
  onRetry?: () => void;
  /** Read existing nested threads until their staged migration completes. */
  onOpenThread?: (id: string) => void;
  onClose: () => void;
}

/**
 * ルートメッセージと返信一覧を出すスレッドペイン。
 *
 * 返信は専用 Shape ではなく、メインと同じ messages コレクションを
 * `parent_id = <root>` で絞った live query の結果を受け取る。送信も通常の
 * `messages.insert`（`parent_id` 付き）で、txid の確定は MessagePane が待つ。
 */
export function ThreadPane({
  commands,
  onRequestCommands,
  onOpenThread,
  targetMessageId,
  mentionUsers,
  mentionMemberIds,
  isMember = true,
  isOffline = false,
  composeDisabledReason,
  postingForbidden = false,
  typingUsers = [],
  onTyping,
  root,
  replies,
  membership,
  membershipPending = false,
  membershipError = false,
  membershipLoading = false,
  participantCount,
  participants,
  onMembershipRetry,
  onJoinThread,
  onLeaveThread,
  onReopenThread,
  onReopenAndJoinThread,
  isLoading,
  isError,
  currentUserId,
  viewerId,
  canModerate = false,
  isArchived = false,
  readOnly = false,
  rootDeleted = false,
  reactions,
  attachments,
  kitchenCatalog,
  canReact,
  onReaction,
  onPickReaction,
  editingId,
  editingText,
  onEditingTextChange,
  onStartEdit,
  onCancelEdit,
  onSaveEdit,
  onRequestDelete,
  onUploadAttachment,
  onSendReply,
  onLoadOlder,
  hasOlder = true,
  isLoadingOlder = false,
  onRetry,
  onClose,
}: ThreadPaneProps) {
  const { t } = useTranslation();
  // 狭幅（sm 未満）は absolute のオーバーレイ。ダイアログとして扱うのはこのときだけ。
  // 広幅の sm:static なサイドペインは従来どおりインラインのままにする。
  const isWidePane = useMediaQuery("(min-width: 640px)");
  const items = useMemo(() => buildMessageItems(replies), [replies]);
  const paneRef = useRef<HTMLDivElement>(null);
  const restoreFocusRef = useRef<HTMLElement | null>(null);
  // チャンネルと同様、参加してから返信する。返信がまだないメッセージも同じで、最初の返信の前に参加する
  // （サーバーは、参加行がないスレッドの最初の返信を、旧版のクライアントのために、参加なしで受け付ける）。
  // 参加状態の取得中（`membership` が未定）は、入力欄を止めるだけで、参加の案内は出さない。
  const needsJoin = isMember && (membership === "none" || membership === "left");
  // 参加ボタンを押したあと、入力欄が使えるようになったら、入力欄へフォーカスを移す。ボタンは消えるので、
  // 何もしないとフォーカスが失われ、キーボード・スクリーンリーダーの利用者が入力欄を探し直すことになる。
  const composerRef = useRef<HTMLDivElement>(null);
  const focusComposerAfterJoin = useRef(false);
  useEffect(() => {
    if (!needsJoin && focusComposerAfterJoin.current) {
      focusComposerAfterJoin.current = false;
      composerRef.current?.querySelector("textarea")?.focus();
    } else if (membershipError) {
      // 参加に失敗したら、あとで別の理由で入力欄が使えるようになっても、フォーカスを奪わない。
      focusComposerAfterJoin.current = false;
    }
  }, [needsJoin, membershipError]);
  const joinThread = isArchived ? (onReopenAndJoinThread ?? onJoinThread) : onJoinThread;
  // 参加者は、ヘッダーの人数ボタンから開く、中央のダイアログに出す。参加・退出の操作もそこに置く。
  const [participantsOpen, setParticipantsOpen] = useState(false);
  const participantsStatus: ThreadParticipantsStatus =
    readOnly || !isMember
      ? null
      : membership === "joined"
        ? "joined"
        : membership === "none" || membership === "left"
          ? "notJoined"
          : "checking";
  // 参加ボタンの出し方は、入力欄の代わりの案内（`ThreadJoinPrompt`）と同じ。アーカイブ済みの再Openは
  // 投稿権限が要るので、権限がないときは出さない。
  // 人数をまだ読み込めていなくても、自分の参加状態が確定していれば、参加・退出の操作のために出す。
  // どちらも未確定（読み込み中）のあいだは、押しても何も出せないので出さない。
  const showParticipants =
    participantCount !== undefined ||
    participantsStatus === "joined" ||
    participantsStatus === "notJoined";
  // ダイアログを描画しない間は、開いた状態も残さない。残すと、狭幅のペインが Escape・Tab を扱わないままになり、
  // あとで人数や参加状態が戻ったときに、操作なしでダイアログが開いてしまう。
  useEffect(() => {
    if (!showParticipants) setParticipantsOpen(false);
  }, [showParticipants]);
  const participantsAction: ThreadParticipantsAction | null =
    participantsStatus === "joined"
      ? onLeaveThread
        ? { kind: "leave", onClick: onLeaveThread, disabled: membershipPending || isOffline }
        : null
      : participantsStatus === "notJoined" && joinThread && !(isArchived && postingForbidden)
        ? {
            kind: isArchived ? "reopenAndJoin" : "join",
            onClick: joinThread,
            disabled: membershipPending || isOffline,
          }
        : null;
  const scrollRef = useRef<HTMLDivElement>(null);
  const hasTargetInReplies = Boolean(
    targetMessageId && replies.some((reply) => reply.id === targetMessageId),
  );
  const atBottomRef = useRef(!hasTargetInReplies);
  const scrolledTarget = useRef<string | undefined>(undefined);
  const prevRootIdRef = useRef(root.id);
  // 件数が上限のままでも、新着で末尾の返信が変わったら、最下部にいるときは追従する。
  const lastReplyId = replies[replies.length - 1]?.id;

  useLayoutEffect(() => {
    if (prevRootIdRef.current !== root.id) {
      prevRootIdRef.current = root.id;
      scrolledTarget.current = undefined;
      atBottomRef.current = !hasTargetInReplies;
    }
  }, [root.id, hasTargetInReplies]);

  useLayoutEffect(() => {
    const element = scrollRef.current;
    if (
      !element ||
      !atBottomRef.current ||
      (hasTargetInReplies && scrolledTarget.current !== targetMessageId)
    ) {
      return;
    }
    element.scrollTop = element.scrollHeight;
  }, [items.length, lastReplyId, root.id, hasTargetInReplies, targetMessageId]);

  // 古い返信のページが先頭に足されたときは、読んでいた位置が動かないよう、増えた高さの分だけ補正する。
  const scrollSnapshot = useRef({ firstId: "", lastId: "", height: 0 });
  useLayoutEffect(() => {
    const element = scrollRef.current;
    const previous = scrollSnapshot.current;
    const firstId = replies[0]?.id ?? "";
    const lastId = replies[replies.length - 1]?.id ?? "";
    if (element) {
      const prepended =
        previous.firstId !== "" &&
        firstId !== previous.firstId &&
        lastId === previous.lastId &&
        !atBottomRef.current;
      if (prepended) element.scrollTop += element.scrollHeight - previous.height;
      scrollSnapshot.current = { firstId, lastId, height: element.scrollHeight };
    }
  }, [replies]);

  useLayoutEffect(() => {
    if (!targetMessageId) {
      scrolledTarget.current = undefined;
      return;
    }
    if (scrolledTarget.current === targetMessageId) return;
    if (!replies.some((reply) => reply.id === targetMessageId)) return;
    const align = () => {
      const target = Array.from(
        scrollRef.current?.querySelectorAll<HTMLElement>("[data-message-id]") ?? [],
      ).find((element) => element.dataset.messageId === targetMessageId);
      if (target && scrollRef.current) {
        atBottomRef.current = false;
        scrolledTarget.current = targetMessageId;
        scrollRef.current.scrollTop = target.offsetTop - scrollRef.current.offsetTop;
      }
    };
    align();
    const frame = requestAnimationFrame(align);
    return () => cancelAnimationFrame(frame);
  }, [targetMessageId, replies]);

  // 幅が変わっても同じ要素ツリーを保つため、ダイアログの属性とフォーカス制御は
  // この要素にだけ付け外しする（Composer の下書きやスクロール位置を失わないため）。
  useEffect(() => {
    restoreFocusRef.current =
      document.activeElement instanceof HTMLElement ? document.activeElement : null;
    return () => restoreFocusRef.current?.focus();
  }, []);

  useEffect(() => {
    if (isWidePane) return;
    // 広幅から狭幅へ戻ったとき、フォーカスが背後に残っていたらペイン内へ移す。
    // すでにペイン内にある場合はユーザーのフォーカス位置を維持する。
    const pane = paneRef.current;
    if (pane && !pane.contains(document.activeElement)) pane.focus();
  }, [isWidePane]);

  function handlePaneKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    if (isWidePane) return;
    // メンバーのダイアログが開いているあいだは、キー操作はそちらのもの。開いた直後で、まだ
    // フォーカスがダイアログへ移っていないときの Escape で、ペインごと閉じないようにする。
    if (participantsOpen) return;
    // 削除確認やリアクションピッカーなど、子のダイアログが Escape を処理する場合は干渉しない。
    if ((event.target as HTMLElement).closest('[role="dialog"]') !== event.currentTarget) return;
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      onClose();
      return;
    }
    if (event.key !== "Tab") return;
    const focusables = Array.from(
      event.currentTarget.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR),
    ).filter(
      (element) =>
        element.getClientRects().length > 0 &&
        !element.hasAttribute("disabled") &&
        element.getAttribute("aria-disabled") !== "true",
    );
    if (focusables.length === 0) {
      event.preventDefault();
      event.currentTarget.focus();
      return;
    }
    const first = focusables[0];
    const last = focusables[focusables.length - 1];
    if (document.activeElement === event.currentTarget) {
      event.preventDefault();
      (event.shiftKey ? last : first).focus();
    } else if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  }

  const paneClassName =
    "absolute inset-0 z-20 flex min-h-0 min-w-0 flex-col border-l bg-background sm:static sm:z-auto sm:w-80 sm:shrink-0 xl:w-96";

  const paneContent = (
    <>
      <PaneHeader className="gap-2" role="none">
        <Button
          variant="ghost"
          size="icon-sm"
          className="size-11 sm:hidden"
          aria-label={t("threads.close")}
          onClick={onClose}
        >
          <ArrowLeftIcon />
        </Button>
        <div className="min-w-0 flex-1">
          <h2 className="text-sm font-semibold">{t("threads.title")}</h2>
          <p className="text-xs tabular-nums text-muted-foreground">
            {isLoading
              ? t("threads.loading")
              : t("threads.replyCount", {
                  count: root.display_data?.reply_count ?? replies.length,
                })}
          </p>
        </div>
        {isArchived ? (
          <span className="text-xs text-muted-foreground">{t("moderation.archived")}</span>
        ) : null}
        {showParticipants ? (
          <Button
            variant="ghost"
            size="sm"
            aria-haspopup="dialog"
            aria-label={
              participantCount === undefined
                ? t("members.title")
                : t("threads.participants", { count: participantCount })
            }
            onClick={() => setParticipantsOpen(true)}
          >
            <UsersIcon aria-hidden="true" />
            <span className="tabular-nums">{participantCount ?? "…"}</span>
          </Button>
        ) : null}
        <Button
          variant="ghost"
          size="icon-sm"
          className="hidden sm:inline-flex"
          aria-label={t("threads.close")}
          onClick={onClose}
        >
          <XIcon />
        </Button>
      </PaneHeader>

      {showParticipants ? (
        <ThreadParticipantsDialog
          open={participantsOpen}
          onOpenChange={setParticipantsOpen}
          count={participantCount}
          participants={participants ?? []}
          creatorId={root.user_id}
          // アーカイブ済み・読み取り専用でも、自分の行に「あなた」の印を付ける。
          currentUserId={viewerId === undefined ? currentUserId : viewerId}
          status={participantsStatus}
          action={participantsAction}
          // ペイン側の表示と同じ条件（メンバーでない人・読み取り専用には、参加・退出の操作がないので出さない）。
          membershipError={!readOnly && isMember && membershipError}
          onMembershipRetry={onMembershipRetry}
        />
      ) : null}

      {!readOnly &&
      isMember &&
      ((isArchived && onReopenThread && !postingForbidden) || membershipError) ? (
        <div className="flex flex-wrap items-center gap-2 border-b px-3 py-2">
          {isArchived && onReopenThread && !postingForbidden ? (
            <Button
              variant="outline"
              size="sm"
              disabled={membershipPending || isOffline}
              onClick={onReopenThread}
            >
              {t("threads.reopen")}
            </Button>
          ) : null}
          {membershipError ? (
            <p role="alert" className="text-xs text-destructive">
              {t("threads.membershipError")}{" "}
              <Button variant="ghost" size="sm" onClick={onMembershipRetry}>
                {t("offline.retry")}
              </Button>
            </p>
          ) : null}
        </div>
      ) : null}
      <div className="max-h-60 shrink-0 overflow-y-auto border-b bg-muted/30 px-2 py-2">
        {rootDeleted ? (
          <p className="px-2 py-1 text-xs text-muted-foreground">{t("threads.rootDeleted")}</p>
        ) : (
          <MessageRow
            item={{ message: root, isGroupStart: true, isGroupEnd: true, showDateSeparator: false }}
            mentionUserId={viewerId ?? currentUserId}
            isOwn={!!currentUserId && root.user_id === currentUserId}
            canDelete={canModerate || (!!currentUserId && root.user_id === currentUserId)}
            isEditing={editingId === root.id}
            editingText={editingText}
            onEditingTextChange={onEditingTextChange}
            onStartEdit={() => onStartEdit(root.id, root.content)}
            onCancelEdit={onCancelEdit}
            onSaveEdit={onSaveEdit}
            onRequestDelete={() => onRequestDelete(root.id)}
            reactions={
              root.display_data
                ? (groupReactions(root.display_data.reactions, currentUserId).get(root.id) ?? [])
                : (reactions.get(root.id) ?? [])
            }
            attachments={root.display_data?.attachments ?? attachments?.get(root.id) ?? []}
            kitchenCatalog={kitchenCatalog}
            canReact={canReact}
            onReaction={(emoji) => onReaction(root.id, emoji)}
            onPickReaction={() => onPickReaction(root.id)}
          />
        )}
      </div>

      <div
        ref={scrollRef}
        data-scroll-restoration-id={`thread-scroll:${root.id}`}
        className="min-h-0 flex-1 overflow-y-auto overscroll-contain px-2 py-2"
        onScroll={(event) => {
          const element = event.currentTarget;
          scrollSnapshot.current.height = element.scrollHeight;
          atBottomRef.current = isNearBottom(
            element.scrollTop,
            element.scrollHeight,
            element.clientHeight,
          );
          if (element.scrollTop <= 48 && hasOlder && !isLoadingOlder) {
            onLoadOlder?.();
          }
        }}
      >
        {isLoading && replies.length === 0 ? (
          <p className="p-3 text-xs text-muted-foreground">{t("threads.repliesLoading")}</p>
        ) : isOffline && replies.length === 0 ? (
          <div className="flex flex-col items-start gap-2 p-3">
            <p className="text-xs text-muted-foreground">{t("offline.emptyTitle")}</p>
            {onRetry ? (
              <Button variant="outline" size="sm" onClick={onRetry}>
                {t("offline.retry")}
              </Button>
            ) : null}
          </div>
        ) : isError && replies.length === 0 ? (
          <div className="flex flex-col items-start gap-2 p-3">
            <p className="text-xs text-destructive">{t("threads.loadError")}</p>
            {onRetry ? (
              <Button variant="outline" size="sm" onClick={onRetry}>
                {t("offline.retry")}
              </Button>
            ) : null}
          </div>
        ) : replies.length === 0 ? (
          <p className="p-3 text-xs text-muted-foreground">{t("threads.empty")}</p>
        ) : (
          <div role="list">
            {items.map((item) => (
              <div
                key={item.message.id}
                data-message-id={item.message.id}
                role="listitem"
                className={
                  item.message.id === targetMessageId
                    ? "rounded-lg bg-brand-muted/40 ring-1 ring-brand"
                    : undefined
                }
              >
                <MessageRow
                  item={item}
                  mentionUserId={viewerId ?? currentUserId}
                  isOwn={!!currentUserId && item.message.user_id === currentUserId}
                  canDelete={
                    canModerate || (!!currentUserId && item.message.user_id === currentUserId)
                  }
                  isEditing={editingId === item.message.id}
                  editingText={editingText}
                  onEditingTextChange={onEditingTextChange}
                  onStartEdit={() => onStartEdit(item.message.id, item.message.content)}
                  onCancelEdit={onCancelEdit}
                  onSaveEdit={onSaveEdit}
                  onRequestDelete={() => onRequestDelete(item.message.id)}
                  replyCount={item.message.display_data?.reply_count ?? 0}
                  onOpenThread={
                    (item.message.display_data?.reply_count ?? 0) > 0 && onOpenThread
                      ? () => onOpenThread(item.message.id)
                      : undefined
                  }
                  reactions={
                    item.message.display_data
                      ? (groupReactions(item.message.display_data.reactions, currentUserId).get(
                          item.message.id,
                        ) ?? [])
                      : (reactions.get(item.message.id) ?? [])
                  }
                  attachments={
                    item.message.display_data?.attachments ??
                    attachments?.get(item.message.id) ??
                    []
                  }
                  kitchenCatalog={kitchenCatalog}
                  canReact={canReact}
                  onReaction={(emoji) => onReaction(item.message.id, emoji)}
                  onPickReaction={() => onPickReaction(item.message.id)}
                />
              </div>
            ))}
          </div>
        )}
      </div>

      {!readOnly ? (
        <>
          {isArchived && !needsJoin && !postingForbidden ? (
            <p className="px-3 py-2 text-xs text-muted-foreground">{t("threads.reopenOnSend")}</p>
          ) : null}
          <TypingIndicator users={typingUsers} />
          {needsJoin ? (
            <ThreadJoinPrompt
              archived={!!isArchived}
              reason={postingForbidden ? composeDisabledReason : undefined}
              pending={membershipPending}
              disabled={membership === undefined || isOffline}
              onJoin={
                joinThread
                  ? () => {
                      focusComposerAfterJoin.current = true;
                      joinThread();
                    }
                  : undefined
              }
            />
          ) : null}
          {/* 参加が必要になっても Composer はアンマウントせず隠す。入力中の本文・添付の下書きを保つ。 */}
          <div ref={composerRef} hidden={needsJoin}>
            <Composer
              commands={commands}
              onRequestCommands={onRequestCommands}
              mentionUsers={mentionUsers}
              mentionMemberIds={mentionMemberIds}
              disabled={
                needsJoin ||
                !isMember ||
                isOffline ||
                membershipLoading ||
                composeDisabledReason !== undefined
              }
              onTyping={onTyping}
              onSend={onSendReply}
              onUploadAttachment={onUploadAttachment}
              placeholder={
                composeDisabledReason ??
                (isOffline ? t("offline.composeDisabled") : t("threads.compose"))
              }
              ariaLabel={t("threads.compose")}
            />
          </div>
        </>
      ) : null}
    </>
  );

  return (
    <div
      ref={paneRef}
      role={isWidePane ? "complementary" : "dialog"}
      aria-modal={isWidePane ? undefined : true}
      aria-label={t("threads.title")}
      tabIndex={-1}
      className={paneClassName}
      onKeyDown={handlePaneKeyDown}
    >
      {paneContent}
    </div>
  );
}

function ThreadJoinPrompt({
  archived,
  reason,
  pending,
  disabled,
  onJoin,
}: {
  archived: boolean;
  /** 投稿権限がないときの理由。出すと、返信を促す文の代わりになる。 */
  reason?: string;
  pending: boolean;
  disabled: boolean;
  onJoin?: () => void;
}) {
  const { t } = useTranslation();
  return (
    <JoinPrompt
      description={reason ?? t("threads.joinToReply")}
      // アーカイブ済みの再Openは投稿権限が要るので、権限がないときはボタンを出さない。
      actionLabel={
        reason && archived ? undefined : archived ? t("threads.reopenAndJoin") : t("threads.join")
      }
      onAction={onJoin}
      pending={pending}
      disabled={disabled}
    />
  );
}
