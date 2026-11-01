import { ActivityPubBadge } from "../../home/ActivityPubBadge";
import { useEffect, useId, useRef, useState } from "react";
import { MessageCard, parseMessageCards } from "./MessageCard";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { cn } from "cn";
import {
  BotIcon,
  CopyIcon,
  MessageSquarePlusIcon,
  MessageSquareTextIcon,
  MoreHorizontalIcon,
  PencilIcon,
  SmilePlusIcon,
  TrashIcon,
  UsersRoundIcon,
} from "lucide-react";
import { MfmContent } from "./MfmContent";
import { LinkPreviews } from "./LinkPreviews.tsx";

import { useLocale } from "../../i18n/LocaleProvider";
import type { KitchenCatalog } from "../../../lib/emoji/kitchen";
import { kitchenLabel, reactionKey } from "../../../lib/emoji/kitchen";
import type { AttachmentRow } from "../../../lib/electric/types";
import type { ReactionGroup } from "../../../lib/emoji/reactions";
import { AppAvatar, AuthorIdentity } from "../AppIdentity";
import { MessageAppContent, hasCardBlocks } from "../AppBlocks";
import { Button, buttonVariants } from "../../ui/button";
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
import { authorLabel, formatDateSeparator, formatTime, fullTimestamp } from "./messageFormat";
import type { MessageItem } from "./messageGrouping";
import { MessageAttachments } from "./MessageAttachments";
import { ReactionEmoji } from "./ReactionEmoji";

export interface MessageRowProps {
  item: MessageItem;
  isOwn: boolean;
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
  attachments?: AttachmentRow[];
}

function EditedIndicator({ at }: { at: string }) {
  const tooltipId = useId();
  const [tooltipOpen, setTooltipOpen] = useState(false);
  const { t } = useTranslation();
  const { locale } = useLocale();
  return (
    <TooltipProvider>
      <Tooltip onOpenChange={setTooltipOpen}>
        <TooltipTrigger
          render={<span role="img" tabIndex={0} />}
          aria-label={t("messages.edited")}
          aria-describedby={tooltipOpen ? tooltipId : undefined}
          className="ml-1 inline-flex rounded-sm align-middle text-muted-foreground/80 outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
        >
          <PencilIcon className="size-3" strokeWidth={1.5} aria-hidden="true" />
        </TooltipTrigger>
        <TooltipContent id={tooltipId} role="tooltip">
          {t("messages.edited")} ·{" "}
          <time dateTime={normalizeUtcTimestamp(at)}>{fullTimestamp(at, locale)}</time>
        </TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
}

/** 通常投稿は MFM、連携カードは専用の描画を使う。 */
export function MessageBody({
  content,
  card: data,
  appBlocks,
  editedAt,
}: {
  editedAt?: string | null;
  content: string;
  card?: string | null;
  appBlocks?: string | null;
}) {
  if (hasCardBlocks(appBlocks)) return null;
  const cards = parseMessageCards(data);
  if (cards)
    return (
      <>
        {cards.map((card, index) => (
          <MessageCard key={index} card={card} />
        ))}
        {editedAt ? <EditedIndicator at={editedAt} /> : null}
      </>
    );
  return (
    <>
      <div className="mfm-body whitespace-pre-wrap break-words text-sm leading-relaxed">
        <MfmContent content={content} />
        {editedAt ? <EditedIndicator at={editedAt} /> : null}
      </div>
      <LinkPreviews content={content} />
    </>
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
      <span className="rounded-full border bg-background/80 px-2.5 py-0.5 text-[11px] font-medium text-muted-foreground">
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
      className="mt-1 flex w-fit items-center gap-1 rounded-full border border-brand/30 bg-brand-muted/50 px-2 py-0.5 text-xs font-medium text-brand-muted-foreground transition-colors outline-none hover:bg-brand-muted focus-visible:ring-2 focus-visible:ring-ring/50"
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
  attachments = [],
}: MessageRowProps) {
  const { t } = useTranslation();
  const { locale } = useLocale();
  const { message } = item;

  async function copyRawMessage() {
    try {
      await navigator.clipboard.writeText(message.content);
      toast.success(t("messages.copied"));
    } catch {
      toast.error(t("messages.copyFailed"));
    }
  }

  const [menuOpen, setMenuOpen] = useState(false);
  const [reactionUsersKey, setReactionUsersKey] = useState<string | null>(null);
  const selectedReaction = reactions.find(
    (reaction) => reactionKey(reaction.emoji) === reactionUsersKey,
  );
  useEffect(() => {
    if (reactionUsersKey !== null && !selectedReaction) setReactionUsersKey(null);
  }, [reactionUsersKey, selectedReaction]);
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
  const hold = useRef<ReturnType<typeof setTimeout> | null>(null);
  const longPressed = useRef(false);
  const origin = useRef({ x: 0, y: 0 });
  function cancelHold() {
    if (hold.current !== null) clearTimeout(hold.current);
    hold.current = null;
  }
  useEffect(() => cancelHold, []);

  const threadButton = onOpenThread ? (
    <Button
      variant="ghost"
      size="icon-xs"
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
        onPointerDown={(event) => {
          cancelHold();
          longPressed.current = false;
          if (
            isEditing ||
            event.pointerType !== "touch" ||
            !event.isPrimary ||
            (event.target as HTMLElement).closest("button, a, input, textarea")
          )
            return;
          origin.current = { x: event.clientX, y: event.clientY };
          hold.current = setTimeout(() => {
            longPressed.current = true;
            setMenuOpen(true);
            hold.current = null;
          }, 500);
        }}
        onPointerMove={(event) => {
          if (Math.hypot(event.clientX - origin.current.x, event.clientY - origin.current.y) > 10)
            cancelHold();
        }}
        onPointerUp={(event) => {
          cancelHold();
          if (longPressed.current) event.stopPropagation();
        }}
        onTouchEnd={(event) => {
          if (longPressed.current) {
            event.preventDefault();
            event.stopPropagation();
            longPressed.current = false;
          }
        }}
        onPointerCancel={() => {
          cancelHold();
          longPressed.current = false;
        }}
        onTouchCancel={() => {
          cancelHold();
          longPressed.current = false;
        }}
        onContextMenu={(event) => {
          // リンクなどの標準メニューは維持し、本文の長押しは操作メニューに使う。
          if ((event.target as HTMLElement).closest("button, a, input, textarea")) return;
          if (longPressed.current || menuOpen || hold.current !== null) event.preventDefault();
        }}
        className={cn(
          "group/row relative flex gap-3 rounded-lg px-3 transition-colors hover:bg-muted/60 focus-within:bg-muted/60",
          item.isGroupStart ? "pt-1.5" : "pt-0",
          item.isGroupEnd ? "pb-1.5" : "pb-0",
        )}
      >
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
              className="text-[10px] leading-none text-muted-foreground tabular-nums opacity-0 group-hover/row:opacity-100 group-focus-within/row:opacity-100"
              dateTime={normalizeUtcTimestamp(message.created_at)}
              title={fullTimestamp(message.created_at, locale)}
            >
              {formatTime(message.created_at, locale)}
            </time>
          </div>
        )}

        <div className="min-w-0 flex-1">
          {item.isGroupStart || message.apub ? (
            <div className="flex flex-wrap items-baseline gap-2">
              {item.isGroupStart ? <AuthorIdentity message={message} /> : null}
              <time
                className="shrink-0 text-[11px] text-muted-foreground tabular-nums"
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
                  if (event.key === "Escape" && !event.nativeEvent.isComposing) {
                    event.preventDefault();
                    onCancelEdit();
                  }
                }}
              />
              <div className="flex justify-end gap-2">
                <Button type="button" variant="ghost" size="sm" onClick={onCancelEdit}>
                  {t("common.cancelShort")}
                </Button>
                <Button type="submit" size="sm" disabled={!editingText.trim()}>
                  {t("common.save")}
                </Button>
              </div>
            </form>
          ) : (
            <>
              <MessageBody
                content={message.content}
                card={message.card}
                appBlocks={message.app_blocks}
                editedAt={message.updated_at}
              />
              <MessageAttachments attachments={attachments} />
              <MessageAppContent message={message} />
              {message.updated_at && hasCardBlocks(message.app_blocks) ? (
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
                                aria-disabled={!canReact}
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
                                  if (canReact) onReaction(reaction.emoji);
                                }}
                              />
                            }
                          >
                            <ReactionEmoji emoji={reaction.emoji} catalog={kitchenCatalog} />
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
                        className="inline-flex size-10 items-center justify-center rounded-full text-muted-foreground hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring md:sr-only md:focus-visible:not-sr-only md:focus-visible:size-10 md:[@media(hover:none)]:not-sr-only md:[@media(hover:none)]:size-10 md:[@media(any-pointer:coarse)]:not-sr-only md:[@media(any-pointer:coarse)]:size-10"
                        aria-label={t("reactions.showUsers", {
                          emoji: kitchenLabel(reaction.emoji),
                        })}
                        title={t("reactions.showUsers", { emoji: kitchenLabel(reaction.emoji) })}
                        onClick={() => setReactionUsersKey(reactionKey(reaction.emoji))}
                      >
                        <UsersRoundIcon className="size-4" aria-hidden="true" />
                      </button>
                    </div>
                  ))}
                  <Button
                    variant="ghost"
                    size="icon-xs"
                    aria-label={t("reactions.add")}
                    title={t("reactions.add")}
                    disabled={!canReact}
                    onClick={onPickReaction}
                  >
                    <SmilePlusIcon />
                  </Button>
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
                aria-label={t("reactions.add")}
                title={t("reactions.add")}
                disabled={!canReact}
                onClick={onPickReaction}
              >
                <SmilePlusIcon />
              </Button>
              {threadButton}
              {isOwn ? (
                <>
                  <Button
                    variant="ghost"
                    size="icon-xs"
                    aria-label={t("messages.edit")}
                    title={t("messages.edit")}
                    onClick={onStartEdit}
                  >
                    <PencilIcon />
                  </Button>
                </>
              ) : null}
              {canDelete ? (
                <Button
                  variant="ghost"
                  size="icon-xs"
                  aria-label={t("messages.delete")}
                  title={t("messages.delete")}
                  className="text-destructive hover:bg-destructive/10 hover:text-destructive"
                  onClick={onRequestDelete}
                >
                  <TrashIcon />
                </Button>
              ) : null}
            </div>
            {/* PC はホバー、タッチは長押し、キーボードはフォーカスで操作する。 */}
            <DropdownMenu open={menuOpen} onOpenChange={setMenuOpen}>
              <DropdownMenuTrigger
                aria-label={t("messages.more")}
                title={t("messages.more")}
                className={buttonVariants({
                  variant: "ghost",
                  size: "icon-xs",
                })}
              >
                <MoreHorizontalIcon />
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                <DropdownMenuItem onClick={() => void copyRawMessage()}>
                  <CopyIcon />
                  {t("messages.copyRaw")}
                </DropdownMenuItem>
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
                <DropdownMenuItem disabled={!canReact} onClick={onPickReaction}>
                  <SmilePlusIcon />
                  {t("reactions.add")}
                </DropdownMenuItem>
                {onOpenThread ? (
                  <DropdownMenuItem onClick={onOpenThread}>
                    <MessageSquarePlusIcon />
                    {t("threads.open")}
                  </DropdownMenuItem>
                ) : null}
                {isOwn ? (
                  <>
                    <DropdownMenuItem onClick={onStartEdit}>
                      <PencilIcon />
                      {t("messages.edit")}
                    </DropdownMenuItem>
                  </>
                ) : null}
                {canDelete ? (
                  <DropdownMenuItem variant="destructive" onClick={onRequestDelete}>
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
        open={reactionUsersKey !== null && !!selectedReaction}
        onOpenChange={(open) => {
          if (!open) setReactionUsersKey(null);
        }}
      >
        <DialogContent>
          <DialogTitle>
            {t("reactions.usersTitle", {
              emoji: selectedReaction ? kitchenLabel(selectedReaction.emoji) : "",
            })}
          </DialogTitle>
          <ul className="space-y-2" aria-live="polite">
            {(selectedReaction ? reactionUserLabels(selectedReaction) : []).map((label, index) => (
              <li key={index} className="[overflow-wrap:anywhere]">
                {label}
              </li>
            ))}
          </ul>
        </DialogContent>
      </Dialog>
    </>
  );
}
