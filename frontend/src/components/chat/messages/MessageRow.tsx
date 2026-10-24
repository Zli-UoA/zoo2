import { MessageCard, parseMessageCard } from "./MessageCard";
import { useTranslation } from "react-i18next";
import { cn } from "cn";
import {
  MessageSquarePlusIcon,
  MessageSquareTextIcon,
  MoreHorizontalIcon,
  PencilIcon,
  SmilePlusIcon,
  TrashIcon,
} from "lucide-react";
import { Fragment } from "react";

import { useLocale } from "../../i18n/LocaleProvider";
import type { KitchenCatalog } from "../../../lib/emoji/kitchen";
import { kitchenLabel, reactionKey } from "../../../lib/emoji/kitchen";
import type { ReactionGroup } from "../../../lib/emoji/reactions";
import { Avatar, AvatarFallback } from "../../ui/avatar";
import { Button, buttonVariants } from "../../ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "../../ui/dropdown-menu";
import { Textarea } from "../../ui/textarea";
import { initials } from "../format";
import {
  authorLabel,
  formatDateSeparator,
  formatTime,
  fullTimestamp,
  messageTextSegments,
} from "./messageFormat";
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

/**
 * 本文。`http(s)` URL だけを `<a>` にし、残りは素のテキストとして描く。
 * 別タブで開き、`noopener noreferrer` で開いた側から `window.opener` を触らせない。
 * メンション装飾は将来対応（今回はやらない）。
 */
export function MessageBody({ content, card: data }: { content: string; card?: string | null }) {
  const card = parseMessageCard(data);
  if (card) return <MessageCard card={card} />;
  return (
    <p className="whitespace-pre-wrap break-words text-sm leading-relaxed">
      {messageTextSegments(content).map((segment, index) =>
        segment.kind === "link" ? (
          <a
            key={index}
            href={segment.value}
            target="_blank"
            rel="noopener noreferrer"
            className="rounded-sm text-brand underline underline-offset-2 outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
          >
            {segment.value}
          </a>
        ) : segment.kind === "mention" ? (
          <span
            key={index}
            className="rounded bg-brand-muted px-0.5 font-medium text-brand-muted-foreground"
          >
            {segment.value}
          </span>
        ) : (
          <Fragment key={index}>{segment.value}</Fragment>
        ),
      )}
    </p>
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
 * 編集 / 削除はホバー / フォーカス時（タッチ端末では常時）に出す。
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
        className={cn(
          "group/row relative flex gap-3 rounded-lg px-3 transition-colors hover:bg-muted/60 focus-within:bg-muted/60",
          item.isGroupStart ? "pt-1.5" : "pt-0",
          item.isGroupEnd ? "pb-1.5" : "pb-0",
        )}
      >
        {item.isGroupStart ? (
          <Avatar className="mt-0.5">
            <AvatarFallback>{initials(authorLabel(message))}</AvatarFallback>
          </Avatar>
        ) : (
          <div className="mt-0.5 w-8 shrink-0 text-right">
            <time
              className="hidden text-[10px] text-muted-foreground tabular-nums group-hover/row:inline group-focus-within/row:inline"
              dateTime={message.created_at}
              title={fullTimestamp(message.created_at, locale)}
            >
              {formatTime(message.created_at, locale)}
            </time>
          </div>
        )}

        <div className="min-w-0 flex-1">
          {item.isGroupStart ? (
            <div className="flex items-baseline gap-2">
              <span className="truncate text-sm font-semibold">{authorLabel(message)}</span>
              <time
                className="shrink-0 text-[11px] text-muted-foreground tabular-nums"
                dateTime={message.created_at}
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
                className="min-h-9 resize-none text-sm"
                onChange={(event) => onEditingTextChange(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === "Escape") {
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
            <MessageBody content={message.content} card={message.card} />
          )}

          {!isEditing ? (
            <>
              {replyCount > 0 && onOpenThread ? (
                <ReplyLink count={replyCount} onOpen={onOpenThread} />
              ) : null}
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
            </>
          ) : null}
        </div>

        {!isEditing ? (
          // 外枠は不可視の間クリックを通す（子は自前で pointer-events を戻す）。
          <div className="pointer-events-none absolute right-1.5 top-0 flex items-center">
            <div className="pointer-events-none hidden items-center gap-0.5 rounded-lg border bg-popover p-0.5 opacity-0 shadow-sm transition-opacity group-hover/row:pointer-events-auto group-hover/row:opacity-100 group-focus-within/row:pointer-events-auto group-focus-within/row:opacity-100 [@media(hover:hover)]:flex">
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
            {/* タッチ端末用の常時表示。「⋯」は外枠の pointer-events を打ち消す。 */}
            <div className="pointer-events-auto hidden items-center gap-0.5 rounded-lg border bg-popover p-0.5 shadow-sm [@media(hover:none)]:flex">
              {threadButton}
              {isOwn ? (
                <DropdownMenu>
                  <DropdownMenuTrigger
                    aria-label={t("messages.more")}
                    title={t("messages.more")}
                    className={buttonVariants({
                      variant: "ghost",
                      size: "icon-xs",
                      className:
                        "pointer-events-auto border bg-popover shadow-sm [@media(hover:hover)]:hidden",
                    })}
                  >
                    <MoreHorizontalIcon />
                  </DropdownMenuTrigger>
                  <DropdownMenuContent align="end">
                    <DropdownMenuItem onClick={onStartEdit}>
                      <PencilIcon />
                      {t("messages.edit")}
                    </DropdownMenuItem>
                    <DropdownMenuItem variant="destructive" onClick={onRequestDelete}>
                      <TrashIcon />
                      {t("messages.delete")}
                    </DropdownMenuItem>
                  </DropdownMenuContent>
                </DropdownMenu>
              ) : null}
            </div>
          </div>
        ) : null}
      </div>
    </>
  );
}
