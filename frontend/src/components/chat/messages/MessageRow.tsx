import { useEffect, useRef, useState } from "react";
import { MessageCard, parseMessageCards } from "./MessageCard";
import { useTranslation } from "react-i18next";
import { cn } from "cn";
import {
  BotIcon,
  MessageSquarePlusIcon,
  MessageSquareTextIcon,
  MoreHorizontalIcon,
  PencilIcon,
  SmilePlusIcon,
  TrashIcon,
} from "lucide-react";
import { MfmContent } from "./MfmContent";
import { LinkPreviews } from "./LinkPreviews.tsx";

import { useLocale } from "../../i18n/LocaleProvider";
import type { KitchenCatalog } from "../../../lib/emoji/kitchen";
import { kitchenLabel, reactionKey } from "../../../lib/emoji/kitchen";
import type { ReactionGroup } from "../../../lib/emoji/reactions";
import { AppAvatar, AuthorIdentity } from "../AppIdentity";
import { MessageAppContent } from "../AppBlocks";
import { Button, buttonVariants } from "../../ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "../../ui/dropdown-menu";
import { Textarea } from "../../ui/textarea";
import { normalizeUtcTimestamp } from "../../../lib/timestamps";
import { authorLabel, formatDateSeparator, formatTime, fullTimestamp } from "./messageFormat";
import type { MessageItem } from "./messageGrouping";
import { ReactionEmoji } from "./ReactionEmoji";

export interface MessageRowProps {
  item: MessageItem;
  isOwn: boolean;
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
}

/** 通常投稿は MFM、連携カードは専用の描画を使う。 */
export function MessageBody({ content, card: data }: { content: string; card?: string | null }) {
  const cards = parseMessageCards(data);
  if (cards)
    return (
      <>
        {cards.map((card, index) => (
          <MessageCard key={index} card={card} />
        ))}
      </>
    );
  return (
    <>
      <div className="mfm-body whitespace-pre-wrap break-words text-sm leading-relaxed">
        <MfmContent content={content} />
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
}: MessageRowProps) {
  const { t } = useTranslation();
  const { locale } = useLocale();
  const { message } = item;

  const [menuOpen, setMenuOpen] = useState(false);
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
          <div className="mt-0.5 w-8 shrink-0 text-right">
            <time
              className="hidden text-[10px] text-muted-foreground tabular-nums group-hover/row:inline group-focus-within/row:inline"
              dateTime={normalizeUtcTimestamp(message.created_at)}
              title={fullTimestamp(message.created_at, locale)}
            >
              {formatTime(message.created_at, locale)}
            </time>
          </div>
        )}

        <div className="min-w-0 flex-1">
          {item.isGroupStart ? (
            <div className="flex items-baseline gap-2">
              <AuthorIdentity message={message} />
              <time
                className="shrink-0 text-[11px] text-muted-foreground tabular-nums"
                dateTime={normalizeUtcTimestamp(message.created_at)}
                title={fullTimestamp(message.created_at, locale)}
              >
                {formatTime(message.created_at, locale)}
              </time>
            </div>
          ) : null}

          {isEditing ? (
            <form
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
              <MessageBody content={message.content} card={message.card} />
              <MessageAppContent message={message} />
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
                    <button
                      key={reactionKey(reaction.emoji)}
                      type="button"
                      className={cn(
                        "inline-flex min-h-8 items-center gap-1 rounded-full border px-2 text-xs transition-colors hover:bg-muted disabled:opacity-50",
                        reaction.ownId &&
                          "border-brand/40 bg-brand-muted text-brand-muted-foreground",
                      )}
                      aria-pressed={!!reaction.ownId}
                      disabled={!canReact}
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
                      title={kitchenLabel(reaction.emoji)}
                      onClick={() => onReaction(reaction.emoji)}
                    >
                      <ReactionEmoji emoji={reaction.emoji} catalog={kitchenCatalog} />
                      <span className="tabular-nums">{reaction.count}</span>
                    </button>
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
          <div className="pointer-events-none absolute right-1.5 top-0 flex items-center">
            <div className="pointer-events-none hidden items-center gap-0.5 rounded-lg border bg-popover p-0.5 opacity-0 shadow-sm transition-opacity group-hover/row:pointer-events-auto group-hover/row:opacity-100 group-focus-within/row:pointer-events-auto group-focus-within/row:opacity-100 [@media(hover:hover)]:flex">
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
                </>
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
                  className:
                    "pointer-events-none opacity-0 focus-visible:pointer-events-auto focus-visible:opacity-100 [@media(hover:hover)]:group-hover/row:pointer-events-auto [@media(hover:hover)]:group-hover/row:opacity-100 data-popup-open:pointer-events-auto data-popup-open:opacity-100",
                })}
              >
                <MoreHorizontalIcon />
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
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
                    <DropdownMenuItem variant="destructive" onClick={onRequestDelete}>
                      <TrashIcon />
                      {t("messages.delete")}
                    </DropdownMenuItem>
                  </>
                ) : null}
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
        ) : null}
      </div>
    </>
  );
}
