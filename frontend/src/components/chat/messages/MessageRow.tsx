import { ActivityPubBadge } from "../../home/ActivityPubBadge";
import { type MouseEvent, type PointerEvent, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { cn } from "cn";
import {
  BotIcon,
  CopyIcon,
  ForwardIcon,
  LinkIcon,
  LockKeyholeIcon,
  LockKeyholeOpenIcon,
  MessageSquarePlusIcon,
  MessageSquareTextIcon,
  MoreHorizontalIcon,
  PencilIcon,
  SmilePlusIcon,
  TrashIcon,
  UsersRoundIcon,
  ShieldAlertIcon,
} from "lucide-react";
import { decryptedMentionAllowlist, isAllowedMention } from "./mentions";
import { messageLinkUrl } from "./messageLink";
import { EditedIndicator, MessageBody } from "./MessageBody";
import { ForwardedMessageCard } from "./ForwardedMessageCard";
export { MessageBody } from "./MessageBody";

import { useLocale } from "../../i18n/LocaleProvider";
import type { KitchenCatalog } from "../../../lib/emoji/kitchen";
import { kitchenLabel, kitchenParts, reactionKey } from "../../../lib/emoji/kitchen";
import { addRecentEmoji, loadRecentEmojis } from "../../../lib/emoji/recents";
import { RecentReactions } from "./RecentReactions";
import type { AttachmentRow } from "../../../lib/electric/types";
import type { ReactionGroup } from "../../../lib/emoji/reactions";
import { AppAvatar, AuthorIdentity } from "../AppIdentity";
import { MessageAppContent, hasCardBlocks } from "../AppBlocks";
import { hidesContent, parseComponents } from "../components/parse";
import { Button } from "../../ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "../../ui/dropdown-menu";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "../../ui/tooltip";
import { Textarea } from "../../ui/textarea";
import { Dialog, DialogContent, DialogTitle } from "../../ui/dialog";
import { normalizeUtcTimestamp } from "../../../lib/timestamps";
import {
  authorLabel,
  formatDateSeparator,
  formatTime,
  fullTimestamp,
  messageTextSegments,
} from "./messageFormat";
import type { MessageItem } from "./messageGrouping";
import { EncryptedMessageAttachments } from "./EncryptedMessageAttachments";
import { MessageAttachments } from "./MessageAttachments";
import { ReactionEmoji } from "./ReactionEmoji";
import { detectComposerPlatform, shouldSendFromKey } from "./Composer";
import {
  useEncryptedChannel,
  useMentionUserNames,
  useMessageDecryption,
  usePlaintextBeforeEncryption,
  useUnencryptedAfterEncrypted,
  resolveMessageDecryption,
  type MessageDecryption,
  type SenderTrust,
  type UndecryptableReason,
} from "../../../lib/e2ee/messageDecryption";

/**
 * タッチ端末ではアイコンや文字の見た目を変えずにヒットエリアだけを 44×44px へ広げる。
 * `(hover: none)` はマウスを使わない環境を表し、デスクトップのレイアウトには影響しない。
 */
const touchHitArea = "[@media(hover:none)]:min-h-11 [@media(hover:none)]:min-w-11";

/** 枠や背景を持つチップは見た目を変えず、透明な疑似要素で当たり判定だけを広げる。 */
const touchChipHitArea =
  "[@media(hover:none)]:after:absolute [@media(hover:none)]:after:inset-x-0 [@media(hover:none)]:after:-inset-y-3 [@media(hover:none)]:after:content-['']";

const sheetActionClass =
  "flex min-h-11 w-full items-center gap-3 rounded-lg px-3 text-left text-sm outline-none hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring/50";

export interface MessageRowProps {
  item: MessageItem;
  isOwn: boolean;
  /** 自分宛てメンションの強調に使う閲覧者 ID。 */
  mentionUserId?: string | null;
  canDelete?: boolean;
  isEditing: boolean;
  editingText: string;
  onEditingTextChange: (value: string) => void;
  onStartEdit: () => void;
  onCancelEdit: () => void;
  onSaveEdit: () => void;
  onRequestDelete: () => void;
  reactions: ReactionGroup[];
  kitchenCatalog: KitchenCatalog | null;
  canReact: boolean;
  onReaction: (emoji: string) => void;
  onPickReaction: () => void;
  /** このルートに付いている返信数。0 なら返信リンクを出さない。 */
  replyCount?: number;
  /** スレッドペインを開く。返信行では未指定。 */
  onOpenThread?: () => void;
  /** メッセージを別チャンネル・DM へ転送する。平文チャンネルのみ渡される。 */
  onForward?: () => void;
  attachments?: AttachmentRow[];
  /**
   * 暗号化チャンネルのメッセージを復号した結果。暗号文のある行で未指定なら、この端末では読めない
   * ものとして表示する（docs/e2ee.md の「受信時の検証」）。
   */
  decryption?: MessageDecryption;
  /**
   * 暗号化チャンネルで、暗号化したメッセージより後ろにある、暗号文のない行か（#367）。`MessageList` が
   * 並びから決める。`true` なら警告の印を付ける。渡さなければ、`MessagePane` が渡す最初の暗号化した
   * メッセージの作成日時と比べて決める（スレッドのペイン）。
   */
  unencryptedAfterEncrypted?: boolean;
  /**
   * メッセージリンクなどで、この行へ移動したときの強調の再生回数。値が変わるたびに、点滅 → 点灯 → フェードアウトを
   * 最初から再生する（styles.css の `message-target-flash`）。未指定なら出さない。メンションの強調とは別の層で、
   * 終わるとメンションの強調だけが残る。
   */
  highlightRun?: number;
}

/** 復号できない暗号化メッセージの代わりに出す行。理由ごとに文言を変える。 */
export function UndecryptableMessage({ reason }: { reason: UndecryptableReason }) {
  const { t } = useTranslation();
  return (
    <p className="flex items-center gap-1.5 text-sm italic text-muted-foreground">
      <LockKeyholeIcon className="size-3.5 shrink-0" aria-hidden="true" />
      <span>{t(`e2ee.undecryptable.${reason}`)}</span>
    </p>
  );
}

/**
 * 暗号化チャンネルで、暗号化したメッセージより後ろにある、暗号文のない行の警告（#367）。
 *
 * 切り替える前の平文のメッセージ（区切りより上）には付けない。正しいサーバーは、切り替えたあとに暗号文の
 * ない行を作れないので、暗号化したメッセージより後ろにあれば、サーバーが足したか書き換えたものとして示す。
 */
export function UnencryptedMessageBadge() {
  const { t } = useTranslation();
  return (
    <span
      className="inline-flex w-fit items-center gap-1 rounded-full border border-warning/40 bg-warning/10 px-2 py-0.5 text-micro font-medium text-warning-strong"
      title={t("e2ee.legacy.unencryptedBadgeHelp")}
    >
      <LockKeyholeOpenIcon className="size-3" aria-hidden="true" />
      {t("e2ee.legacy.unencryptedBadge")}
    </span>
  );
}

/** 検証済みでない送信者の印。検証済みなら何も出さない。 */
export function SenderTrustBadge({ trust }: { trust: SenderTrust }) {
  const { t } = useTranslation();
  if (trust === "verified") return null;
  return (
    <span className="inline-flex w-fit items-center gap-1 rounded-full border border-warning/40 bg-warning/10 px-2 py-0.5 text-micro font-medium text-warning-strong">
      <ShieldAlertIcon className="size-3" aria-hidden="true" />
      {t(trust === "unverified" ? "e2ee.sender.unverified" : "e2ee.sender.unauthenticated")}
    </span>
  );
}

/** 版の照合の警告（サーバーが古い版や、同じ版で中身の違う行を返した）。 */
export function RevisionWarningBadge({ warning }: { warning: "stale" | "conflict" }) {
  const { t } = useTranslation();
  return (
    <span className="inline-flex w-fit items-center gap-1 rounded-full border border-warning/40 bg-warning/10 px-2 py-0.5 text-micro font-medium text-warning-strong">
      <ShieldAlertIcon className="size-3" aria-hidden="true" />
      {t(warning === "stale" ? "e2ee.revision.stale" : "e2ee.revision.conflict")}
    </span>
  );
}

function DateSeparator({ value }: { value: string }) {
  const { t } = useTranslation();
  const { locale } = useLocale();
  return (
    <div
      className="flex items-center gap-3 py-2"
      role="separator"
      aria-label={formatDateSeparator(value, t, locale)}
    >
      <span className="h-px flex-1 bg-border" />
      <span className="rounded-full border bg-background/80 px-2.5 py-0.5 text-micro font-medium text-muted-foreground">
        {formatDateSeparator(value, t, locale)}
      </span>
      <span className="h-px flex-1 bg-border" />
    </div>
  );
}

function ReplyLink({ count, onOpen }: { count: number; onOpen: () => void }) {
  const { t } = useTranslation();
  return (
    <button
      type="button"
      className={cn(
        "relative mt-1 flex w-fit items-center gap-1 rounded-full border border-brand/30 bg-brand-muted/50 px-2 py-0.5 text-xs font-medium text-brand-muted-foreground transition-colors outline-none hover:bg-brand-muted focus-visible:ring-2 focus-visible:ring-ring/50",
        touchChipHitArea,
      )}
      onClick={onOpen}
      aria-label={t("threads.openWithCount", { count })}
    >
      <MessageSquareTextIcon className="size-3.5" aria-hidden="true" />
      <span className="tabular-nums">{t("threads.replyCount", { count })}</span>
    </button>
  );
}

/**
 * 1 行分のメッセージ。連投は先頭だけアバターと投稿者名を出し、以降は本文だけを
 * 詰めて表示する。リアクションのチップは自分の分をハイライトし、クリックで
 * 追加 / 削除する。返信数はルートにだけ出し、押すとスレッドペインを開く。
 * 編集 / 削除はホバー / フォーカス時（タッチ端末では長押し）に出す。
 */
export function MessageRow({
  item,
  isOwn,
  mentionUserId,
  canDelete = isOwn,
  isEditing,
  editingText,
  onEditingTextChange,
  onStartEdit,
  onCancelEdit,
  onSaveEdit,
  onRequestDelete,
  reactions,
  kitchenCatalog,
  canReact,
  onReaction,
  onPickReaction,
  replyCount = 0,
  onOpenThread,
  onForward,
  attachments = [],
  decryption,
  unencryptedAfterEncrypted,
  highlightRun,
}: MessageRowProps) {
  const { t } = useTranslation();
  const { locale } = useLocale();
  const { message } = item;
  const isEncryptedChannel = useEncryptedChannel();
  const decryptionFromPane = useMessageDecryption(message.id);
  // 暗号化に切り替える前の平文のメッセージ（#367）。本文はそのまま出し、編集させない。
  const beforeEncryption = usePlaintextBeforeEncryption(message);
  const canEdit = isOwn && !beforeEncryption;
  // チャンネルの一覧は並びから決めて渡す。スレッドのペインなど、渡さない場所では作成日時で決める。
  const afterEncryptedFromTime = useUnencryptedAfterEncrypted(message);
  const warnUnencrypted = beforeEncryption && (unencryptedAfterEncrypted ?? afterEncryptedFromTime);
  const resolvedDecryption = resolveMessageDecryption(message, decryption ?? decryptionFromPane);
  const canForward = !!onForward && !isEncryptedChannel && !resolvedDecryption;
  const canSubmitEmptyEdit =
    !resolvedDecryption &&
    (attachments.length > 0 || (message.forwarded_message_ids?.length ?? 0) > 0);
  // 暗号化チャンネルの行の `content` は空なので、復号した本文を使う。
  const bodyText =
    resolvedDecryption?.status === "decrypted" ? resolvedDecryption.body : message.content;
  // 暗号化メッセージのメンションは、本文の `@name` ではなく、復号した `mentions` だけで表示する。
  const mentionUserNames = useMentionUserNames();
  const mentionMe = mentionUserId ? mentionUserNames.get(mentionUserId) : undefined;
  const decryptedMentions =
    resolvedDecryption?.status === "decrypted" ? resolvedDecryption.mentions : undefined;
  const mentionAllowlist = useMemo(
    () =>
      decryptedMentions
        ? decryptedMentionAllowlist(decryptedMentions, mentionUserNames)
        : undefined,
    [decryptedMentions, mentionUserNames],
  );
  const isMentioned =
    !!mentionMe &&
    messageTextSegments(bodyText).some(
      (segment) =>
        segment.kind === "mention" &&
        (segment.value === `@${mentionMe}` ||
          ["@channel", "@here"].includes(segment.value.toLowerCase())) &&
        (!mentionAllowlist || isAllowedMention(segment.value, mentionAllowlist)),
    );

  async function copyRawMessage() {
    // メニューを出さない状態でも、ほかの経路から呼ばれたときに空文字列で上書きしない。
    if (resolvedDecryption?.status === "undecryptable") return;
    try {
      await navigator.clipboard.writeText(bodyText);
      toast.success(t("messages.copied"));
    } catch {
      toast.error(t("messages.copyFailed"));
    }
  }

  // リンクは ID だけで作れるので、復号できない暗号化メッセージでも出す。チャンネルが分からない行だけ出さない。
  const canCopyLink = !!message.channel_id;
  async function copyMessageLink() {
    if (!message.channel_id) return;
    try {
      await navigator.clipboard.writeText(
        messageLinkUrl(window.location.origin, message.channel_id, message.id, message.parent_id),
      );
      toast.success(t("messages.linkCopied"));
    } catch {
      toast.error(t("messages.linkCopyFailed"));
    }
  }

  const [platform] = useState(() => detectComposerPlatform());
  const [menuOpen, setMenuOpen] = useState(false);
  const [reactionUsersDialog, setReactionUsersDialog] = useState<
    { type: "reaction"; key: string } | { type: "all" } | null
  >(null);
  const selectedReaction = reactions.find(
    (reaction) =>
      reactionUsersDialog?.type === "reaction" &&
      reactionKey(reaction.emoji) === reactionUsersDialog.key,
  );
  useEffect(() => {
    if (reactionUsersDialog?.type === "reaction" && !selectedReaction) {
      setReactionUsersDialog(null);
    } else if (reactionUsersDialog?.type === "all" && reactions.length === 0) {
      setReactionUsersDialog(null);
    }
  }, [reactionUsersDialog, selectedReaction, reactions.length]);
  function reactionUserLabels(reaction: ReactionGroup): string[] {
    const ids = reaction.userIds ?? [];
    const labels = ids.flatMap((id) => (reaction.userNames?.[id] ? [reaction.userNames[id]] : []));
    if (ids.some((id) => !reaction.userNames?.[id])) {
      labels.push(
        t(reaction.usersLoading ? "reactions.usersLoading" : "reactions.userUnavailable"),
      );
    }
    return labels;
  }
  const [sheetOpen, setSheetOpen] = useState(false);
  const tapOrigin = useRef<{ x: number; y: number } | null>(null);
  const [recentEmojis, setRecentEmojis] = useState<string[]>([]);
  useEffect(() => {
    if (!sheetOpen) return;
    // チャットのリアクション API は画像付き（apub:）の値を受け付けないので除外する。
    // 件数制限は RecentReactions で有効な候補を絞った後に行う。
    setRecentEmojis(loadRecentEmojis().filter((emoji) => !emoji.startsWith("apub:")));
  }, [sheetOpen]);

  function rememberTouchStart(event: PointerEvent<HTMLDivElement>) {
    // ポータル内のダイアログは React ツリー上この行の子だが、DOM 上は外にある。
    if (!event.currentTarget.contains(event.target as Node)) {
      tapOrigin.current = null;
      return;
    }
    tapOrigin.current =
      event.pointerType === "touch" ? { x: event.clientX, y: event.clientY } : null;
  }

  function openSheetOnTap(event: MouseEvent<HTMLDivElement>) {
    // 画像プレビューなどのポータル内タップは、React のバブリングでここまで届くので除外する。
    if (!event.currentTarget.contains(event.target as Node)) return;
    const origin = tapOrigin.current;
    tapOrigin.current = null;
    // pointerdown がタッチで、移動していないシングルタップだけを対象にする。
    // シートは click で開く（pointerup で開くと直後の click が外側扱いになり閉じるため）。
    if (isEditing || !origin) return;
    if (Math.hypot(event.clientX - origin.x, event.clientY - origin.y) > 10) return;
    // リンク・操作ボタン・メニュー項目のタップは従来どおり動かす。
    if (
      (event.target as HTMLElement).closest(
        "a, button, input, textarea, audio, video, .mfm-blur, [role='button'], [data-slot='dropdown-menu-item']",
      )
    )
      return;
    setSheetOpen(true);
  }

  const threadButton = onOpenThread ? (
    <Button
      variant="ghost"
      size="icon-xs"
      className={touchHitArea}
      aria-label={t("threads.open")}
      title={t("threads.open")}
      onClick={onOpenThread}
    >
      <MessageSquarePlusIcon />
    </Button>
  ) : null;

  return (
    <>
      {item.showDateSeparator ? <DateSeparator value={message.created_at} /> : null}
      <div
        onPointerDown={rememberTouchStart}
        onClick={openSheetOnTap}
        className={cn(
          "group/row relative flex gap-3 rounded-lg px-3 transition-colors",
          isMentioned
            ? "bg-mention/[0.08] before:absolute before:inset-y-0 before:left-0 before:w-[3px] before:bg-mention before:content-[''] hover:bg-mention/[0.12] focus-within:bg-mention/[0.12]"
            : "hover:bg-muted/60 focus-within:bg-muted/60",
          item.isGroupStart ? "pt-1.5" : "pt-0",
          item.isGroupEnd ? "pb-1.5" : "pb-0",
        )}
      >
        {highlightRun !== undefined ? (
          // メンションの強調と同じ領域・同じ形式（背景と左端のアクセント）で、青を重ねる。
          <span
            key={highlightRun}
            aria-hidden="true"
            data-message-target-highlight=""
            className="animate-message-target absolute inset-0 rounded-lg bg-message-target/[0.16] before:absolute before:inset-y-0 before:left-0 before:w-[3px] before:bg-message-target before:content-['']"
          />
        ) : null}
        {item.isGroupStart ? (
          <AppAvatar
            className="mt-0.5"
            label={authorLabel(message)}
            isBot={!!message.is_bot}
            src={message.avatar_url}
          />
        ) : (
          <div className="flex h-lh w-8 shrink-0 items-center justify-end text-sm leading-relaxed">
            <time
              className="text-2xs leading-none text-muted-foreground tabular-nums opacity-0 group-hover/row:opacity-100 group-focus-within/row:opacity-100"
              dateTime={normalizeUtcTimestamp(message.created_at)}
              title={fullTimestamp(message.created_at, locale)}
            >
              {formatTime(message.created_at, locale)}
            </time>
          </div>
        )}

        <div className="min-w-0 flex-1">
          {item.isGroupStart || message.apub ? (
            <div className="flex flex-wrap items-center gap-2">
              {item.isGroupStart ? <AuthorIdentity message={message} /> : null}
              <time
                className="shrink-0 text-micro text-muted-foreground tabular-nums"
                dateTime={normalizeUtcTimestamp(message.created_at)}
                title={fullTimestamp(message.created_at, locale)}
              >
                {formatTime(message.created_at, locale)}
              </time>
              {message.apub ? (
                <ActivityPubBadge visibility={message.apub_visibility ?? "public"} />
              ) : null}
            </div>
          ) : null}

          {isEditing ? (
            <form
              autoComplete="off"
              className="mt-1 flex flex-col gap-2 rounded-lg border bg-background p-2"
              onSubmit={(event) => {
                event.preventDefault();
                onSaveEdit();
              }}
            >
              <Textarea
                value={editingText}
                autoFocus
                rows={1}
                aria-label={t("messages.editLabel")}
                className="max-h-48 min-h-9 resize-none overflow-y-auto text-base md:text-sm"
                onChange={(event) => onEditingTextChange(event.target.value)}
                onKeyDown={(event) => {
                  if (event.nativeEvent.isComposing || event.nativeEvent.keyCode === 229) return;
                  if (event.key === "Escape") {
                    event.preventDefault();
                    onCancelEdit();
                    return;
                  }
                  if (
                    shouldSendFromKey(
                      {
                        key: event.key,
                        metaKey: event.metaKey,
                        ctrlKey: event.ctrlKey,
                        shiftKey: event.shiftKey,
                        isComposing: event.nativeEvent.isComposing,
                      },
                      platform,
                    )
                  ) {
                    event.preventDefault();
                    if (editingText.trim() || canSubmitEmptyEdit) onSaveEdit();
                  }
                }}
              />
              <div className="flex justify-end gap-2">
                <Button type="button" variant="ghost" size="sm" onClick={onCancelEdit}>
                  {t("common.cancelShort")}
                </Button>
                <Button
                  type="submit"
                  size="sm"
                  disabled={!editingText.trim() && !canSubmitEmptyEdit}
                >
                  {t("common.save")}
                </Button>
              </div>
            </form>
          ) : (
            <>
              {resolvedDecryption?.status === "undecryptable" ? (
                <UndecryptableMessage reason={resolvedDecryption.reason} />
              ) : (
                <>
                  {resolvedDecryption?.status === "decrypted" ? (
                    <SenderTrustBadge trust={resolvedDecryption.sender} />
                  ) : null}
                  {warnUnencrypted ? <UnencryptedMessageBadge /> : null}
                  {resolvedDecryption?.status === "decrypted" &&
                  resolvedDecryption.revisionWarning ? (
                    <RevisionWarningBadge warning={resolvedDecryption.revisionWarning} />
                  ) : null}
                  <MessageBody
                    emojis={message.display_data?.emojis}
                    content={bodyText}
                    // 暗号化の行のカードと App の表示は暗号文で確かめられないので、本文の代わりにしない。
                    card={resolvedDecryption ? null : message.card}
                    appBlocks={resolvedDecryption ? null : message.app_blocks}
                    components={resolvedDecryption ? null : message.components}
                    editedAt={message.updated_at}
                    previews={resolvedDecryption ? "manual" : "auto"}
                    mentions={mentionAllowlist}
                  />
                </>
              )}
              {/* 暗号化チャンネルでは、サーバーの添付の行（encrypted.bin）ではなく、復号した本文の
                  attachments を表示する。復号できないメッセージの添付は表示も取得もしない（#364）。 */}
              {resolvedDecryption ? (
                resolvedDecryption.status === "decrypted" ? (
                  <EncryptedMessageAttachments
                    attachments={resolvedDecryption.attachments ?? []}
                    serverAttachments={attachments}
                  />
                ) : null
              ) : (
                <MessageAttachments attachments={attachments} />
              )}
              {resolvedDecryption ? null : <MessageAppContent message={message} />}
              {!isEncryptedChannel &&
              !resolvedDecryption &&
              message.forwarded_message_ids?.length ? (
                <div className="mt-1 flex flex-col gap-1.5">
                  {message.forwarded_message_ids.map((forwardedId) => (
                    <ForwardedMessageCard key={forwardedId} messageId={forwardedId} />
                  ))}
                </div>
              ) : null}
              {!resolvedDecryption &&
              message.updated_at &&
              (hidesContent(parseComponents(message.components)) ||
                (!parseComponents(message.components) && hasCardBlocks(message.app_blocks))) ? (
                <EditedIndicator at={message.updated_at} />
              ) : null}
            </>
          )}

          {!isEditing ? (
            <>
              {replyCount > 0 && onOpenThread ? (
                <ReplyLink count={replyCount} onOpen={onOpenThread} />
              ) : null}
              {reactions.length > 0 ? (
                <div
                  className="flex flex-wrap items-center gap-1 py-1"
                  role="group"
                  aria-label={t("reactions.groupLabel")}
                >
                  {reactions.map((reaction) => (
                    <div key={reactionKey(reaction.emoji)} className="inline-flex items-center">
                      <TooltipProvider>
                        <Tooltip>
                          <TooltipTrigger
                            render={
                              <button
                                type="button"
                                className={cn(
                                  "inline-flex min-h-8 items-center gap-1 rounded-full border px-2 text-xs transition-colors hover:bg-muted aria-disabled:opacity-50",
                                  reaction.ownId &&
                                    "border-brand/40 bg-brand-muted text-brand-muted-foreground",
                                )}
                                aria-pressed={!!reaction.ownId}
                                aria-disabled={!canReact || reaction.emoji.startsWith("apub:")}
                                aria-label={
                                  reaction.ownId
                                    ? t("reactions.chipMine", {
                                        emoji: kitchenLabel(reaction.emoji),
                                        count: reaction.count,
                                      })
                                    : t("reactions.chip", {
                                        emoji: kitchenLabel(reaction.emoji),
                                        count: reaction.count,
                                      })
                                }
                                onClick={() => {
                                  if (canReact && !reaction.emoji.startsWith("apub:"))
                                    onReaction(reaction.emoji);
                                }}
                              />
                            }
                          >
                            <ReactionEmoji
                              emoji={reaction.emoji}
                              catalog={kitchenCatalog}
                              emojis={message.display_data?.emojis}
                            />
                            <span className="tabular-nums">{reaction.count}</span>
                          </TooltipTrigger>
                          <TooltipContent
                            role="tooltip"
                            className="hidden md:flex max-h-64 max-w-xs overflow-y-auto [overflow-wrap:anywhere]"
                          >
                            <span>{reactionUserLabels(reaction).join(", ")}</span>
                          </TooltipContent>
                        </Tooltip>
                      </TooltipProvider>
                      <button
                        type="button"
                        className="hidden size-10 items-center justify-center rounded-full text-muted-foreground hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring md:inline-flex md:sr-only md:focus-visible:not-sr-only md:focus-visible:size-10 md:[@media(hover:none)]:not-sr-only md:[@media(hover:none)]:size-10 md:[@media(any-pointer:coarse)]:not-sr-only md:[@media(any-pointer:coarse)]:size-10"
                        aria-label={t("reactions.showUsers", {
                          emoji: kitchenLabel(reaction.emoji),
                        })}
                        title={t("reactions.showUsers", { emoji: kitchenLabel(reaction.emoji) })}
                        onClick={() =>
                          setReactionUsersDialog({
                            type: "reaction",
                            key: reactionKey(reaction.emoji),
                          })
                        }
                      >
                        <UsersRoundIcon className="size-4" aria-hidden="true" />
                      </button>
                    </div>
                  ))}
                  <Button
                    variant="ghost"
                    size="icon-xs"
                    className={touchHitArea}
                    aria-label={t("reactions.add")}
                    title={t("reactions.add")}
                    disabled={!canReact}
                    onClick={onPickReaction}
                  >
                    <SmilePlusIcon />
                  </Button>
                  <button
                    type="button"
                    className="inline-flex size-10 items-center justify-center rounded-full text-muted-foreground hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring md:hidden"
                    aria-label={t("reactions.showAllUsers")}
                    title={t("reactions.showAllUsers")}
                    onClick={() => setReactionUsersDialog({ type: "all" })}
                  >
                    <UsersRoundIcon className="size-4" aria-hidden="true" />
                  </button>
                </div>
              ) : null}
            </>
          ) : null}
        </div>

        {!isEditing ? (
          // 外枠は不可視の間クリックを通す（子は自前で pointer-events を戻す）。
          <div
            data-message-toolbar
            className={cn(
              "pointer-events-none absolute right-1.5 top-0 flex items-center gap-0.5 rounded-lg border bg-popover p-0.5 opacity-0 shadow-sm transition-opacity focus-within:pointer-events-auto focus-within:opacity-100 [@media(hover:hover)]:group-hover/row:pointer-events-auto [@media(hover:hover)]:group-hover/row:opacity-100",
              menuOpen && "pointer-events-auto opacity-100",
            )}
          >
            <div className="hidden items-center gap-0.5 [@media(hover:hover)]:flex">
              <Button
                variant="ghost"
                size="icon-xs"
                className={touchHitArea}
                aria-label={t("reactions.add")}
                title={t("reactions.add")}
                disabled={!canReact}
                onClick={onPickReaction}
              >
                <SmilePlusIcon />
              </Button>
              {threadButton}
              {canEdit ? (
                <>
                  <Button
                    variant="ghost"
                    size="icon-xs"
                    className={touchHitArea}
                    aria-label={t("messages.editMessage")}
                    title={t("messages.editMessage")}
                    onClick={onStartEdit}
                  >
                    <PencilIcon />
                  </Button>
                </>
              ) : null}
              {canDelete ? (
                <Button
                  variant="destructive"
                  size="icon-xs"
                  aria-label={t("messages.delete")}
                  title={t("messages.delete")}
                  className={touchHitArea}
                  onClick={onRequestDelete}
                >
                  <TrashIcon />
                </Button>
              ) : null}
            </div>
            {/* PC はホバー、タッチは長押し、キーボードはフォーカスで操作する。 */}
            <DropdownMenu open={menuOpen} onOpenChange={setMenuOpen}>
              <DropdownMenuTrigger
                render={<Button variant="ghost" size="icon-xs" className={touchHitArea} />}
                aria-label={t("messages.more")}
                title={t("messages.more")}
              >
                <MoreHorizontalIcon />
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                {/* 並びは、リアクション・スレッド・転送・編集・原文コピー・リンクコピー・App・削除の順。 */}
                <DropdownMenuItem
                  className={touchHitArea}
                  disabled={!canReact}
                  onClick={onPickReaction}
                >
                  <SmilePlusIcon />
                  {t("reactions.add")}
                </DropdownMenuItem>
                {onOpenThread ? (
                  <DropdownMenuItem className={touchHitArea} onClick={onOpenThread}>
                    <MessageSquarePlusIcon />
                    {t("threads.open")}
                  </DropdownMenuItem>
                ) : null}
                {canForward ? (
                  <DropdownMenuItem className={touchHitArea} onClick={onForward}>
                    <ForwardIcon />
                    {t("messages.forward")}
                  </DropdownMenuItem>
                ) : null}
                {canEdit ? (
                  <>
                    <DropdownMenuItem className={touchHitArea} onClick={onStartEdit}>
                      <PencilIcon />
                      {t("messages.editMessage")}
                    </DropdownMenuItem>
                  </>
                ) : null}
                {/* 復号できない暗号化メッセージは本文がないので、空文字列をコピーしないようメニューを出さない。 */}
                {resolvedDecryption?.status === "undecryptable" ? null : (
                  <DropdownMenuItem onClick={() => void copyRawMessage()}>
                    <CopyIcon />
                    {t("messages.copyRaw")}
                  </DropdownMenuItem>
                )}
                {canCopyLink ? (
                  <DropdownMenuItem className={touchHitArea} onClick={() => void copyMessageLink()}>
                    <LinkIcon />
                    {t("messages.copyLink")}
                  </DropdownMenuItem>
                ) : null}
                <DropdownMenuItem
                  onClick={() =>
                    window.dispatchEvent(
                      new CustomEvent("zoo:message-actions", { detail: message.id }),
                    )
                  }
                >
                  <BotIcon />
                  {t("apps.messageActions")}
                </DropdownMenuItem>
                {canDelete ? (
                  <DropdownMenuItem
                    className={touchHitArea}
                    variant="destructive"
                    onClick={onRequestDelete}
                  >
                    <TrashIcon />
                    {t("messages.delete")}
                  </DropdownMenuItem>
                ) : null}
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
        ) : null}
      </div>
      <Dialog
        open={
          reactionUsersDialog?.type === "all"
            ? reactions.length > 0
            : reactionUsersDialog?.type === "reaction" && !!selectedReaction
        }
        onOpenChange={(open) => {
          if (!open) setReactionUsersDialog(null);
        }}
      >
        <DialogContent>
          <DialogTitle>
            {reactionUsersDialog?.type === "all"
              ? t("reactions.allUsersTitle")
              : t("reactions.usersTitle", {
                  emoji: selectedReaction ? kitchenLabel(selectedReaction.emoji) : "",
                })}
          </DialogTitle>
          {reactionUsersDialog?.type === "all" ? (
            <div className="max-h-[min(60vh,32rem)] space-y-4 overflow-y-auto" aria-live="polite">
              {reactions.map((reaction) => (
                <section key={reactionKey(reaction.emoji)}>
                  <h3
                    className="mb-2 flex items-center gap-2 text-sm font-semibold"
                    aria-label={kitchenLabel(reaction.emoji)}
                  >
                    <span aria-hidden="true" className="min-w-0 [overflow-wrap:anywhere]">
                      {reaction.emoji.startsWith(":") || reaction.emoji.startsWith("apub:") ? (
                        kitchenLabel(reaction.emoji)
                      ) : (
                        <ReactionEmoji
                          emoji={reaction.emoji}
                          catalog={kitchenCatalog}
                          emojis={message.display_data?.emojis}
                        />
                      )}
                    </span>
                    {kitchenParts(reaction.emoji) ? (
                      <span aria-hidden="true">{kitchenLabel(reaction.emoji)}</span>
                    ) : null}
                  </h3>
                  <ul className="space-y-2">
                    {reactionUserLabels(reaction).map((label, index) => (
                      <li key={index} className="[overflow-wrap:anywhere]">
                        {label}
                      </li>
                    ))}
                  </ul>
                </section>
              ))}
            </div>
          ) : (
            <ul className="space-y-2" aria-live="polite">
              {(selectedReaction ? reactionUserLabels(selectedReaction) : []).map(
                (label, index) => (
                  <li key={index} className="[overflow-wrap:anywhere]">
                    {label}
                  </li>
                ),
              )}
            </ul>
          )}
        </DialogContent>
      </Dialog>

      <Dialog open={sheetOpen} onOpenChange={setSheetOpen}>
        <DialogContent
          aria-label={t("messages.actions")}
          className="inset-x-0 bottom-0 top-auto max-w-none translate-x-0 translate-y-0 rounded-t-2xl rounded-b-none pb-6 sm:inset-x-auto sm:bottom-auto sm:left-1/2 sm:top-1/2 sm:max-w-sm sm:-translate-x-1/2 sm:-translate-y-1/2 sm:rounded-xl"
        >
          <DialogTitle>{t("messages.actions")}</DialogTitle>
          {canReact ? (
            <RecentReactions
              values={recentEmojis}
              catalog={kitchenCatalog}
              onSelect={(emoji) => {
                addRecentEmoji(emoji);
                onReaction(emoji);
                setSheetOpen(false);
              }}
            />
          ) : null}
          <div className="flex flex-col">
            {canReact ? (
              <button
                type="button"
                className={sheetActionClass}
                onClick={() => {
                  setSheetOpen(false);
                  onPickReaction();
                }}
              >
                <SmilePlusIcon className="size-4 shrink-0" aria-hidden="true" />
                {t("reactions.add")}
              </button>
            ) : null}
            {onOpenThread ? (
              <button
                type="button"
                className={sheetActionClass}
                onClick={() => {
                  setSheetOpen(false);
                  onOpenThread();
                }}
              >
                <MessageSquareTextIcon className="size-4 shrink-0" aria-hidden="true" />
                {t("threads.open")}
              </button>
            ) : null}
            {canForward ? (
              <button
                type="button"
                className={sheetActionClass}
                onClick={() => {
                  setSheetOpen(false);
                  onForward();
                }}
              >
                <ForwardIcon className="size-4 shrink-0" aria-hidden="true" />
                {t("messages.forward")}
              </button>
            ) : null}
            {canEdit ? (
              <button
                type="button"
                className={sheetActionClass}
                onClick={() => {
                  setSheetOpen(false);
                  onStartEdit();
                }}
              >
                <PencilIcon className="size-4 shrink-0" aria-hidden="true" />
                {t("messages.editMessage")}
              </button>
            ) : null}
            {resolvedDecryption?.status !== "undecryptable" ? (
              <button
                type="button"
                className={sheetActionClass}
                onClick={() => {
                  setSheetOpen(false);
                  void copyRawMessage();
                }}
              >
                <CopyIcon className="size-4 shrink-0" aria-hidden="true" />
                {t("messages.copyRaw")}
              </button>
            ) : null}
            {canCopyLink ? (
              <button
                type="button"
                className={sheetActionClass}
                onClick={() => {
                  setSheetOpen(false);
                  void copyMessageLink();
                }}
              >
                <LinkIcon className="size-4 shrink-0" aria-hidden="true" />
                {t("messages.copyLink")}
              </button>
            ) : null}
            <button
              type="button"
              className={sheetActionClass}
              onClick={() => {
                setSheetOpen(false);
                window.dispatchEvent(
                  new CustomEvent("zoo:message-actions", { detail: message.id }),
                );
              }}
            >
              <BotIcon className="size-4 shrink-0" aria-hidden="true" />
              {t("apps.messageActions")}
            </button>
            {canDelete ? (
              <button
                type="button"
                className={cn(sheetActionClass, "text-destructive hover:bg-destructive/10")}
                onClick={() => {
                  setSheetOpen(false);
                  onRequestDelete();
                }}
              >
                <TrashIcon className="size-4 shrink-0" aria-hidden="true" />
                {t("messages.delete")}
              </button>
            ) : null}
          </div>
        </DialogContent>
      </Dialog>
    </>
  );
}
