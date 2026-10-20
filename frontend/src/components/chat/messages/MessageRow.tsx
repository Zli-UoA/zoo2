import { useTranslation } from "react-i18next";
import { cn } from "cn";
import { MoreHorizontalIcon, PencilIcon, TrashIcon } from "lucide-react";
import { Fragment } from "react";

import { useLocale } from "../../i18n/LocaleProvider";
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

interface MessageRowProps {
  item: MessageItem;
  isOwn: boolean;
  isEditing: boolean;
  editingText: string;
  onEditingTextChange: (value: string) => void;
  onStartEdit: () => void;
  onCancelEdit: () => void;
  onSaveEdit: () => void;
  onRequestDelete: () => void;
}

/**
 * 本文。`http(s)` URL だけを `<a>` にし、残りは素のテキストとして描く。
 * 別タブで開き、`noopener noreferrer` で開いた側から `window.opener` を触らせない。
 * メンション装飾は将来対応（今回はやらない）。
 */
function MessageBody({ content }: { content: string }) {
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

/**
 * 1 行分のメッセージ。連投は先頭だけアバターと投稿者名を出し、以降は本文だけを
 * 詰めて表示する。ホバー / フォーカスの両方で操作群を出す（キーボード到達可能）。
 * 不可視の間は `pointer-events-none` でクリックを奪わない。ホバーが無いタッチ
 * 端末では常時表示の「⋯」メニューを逃げ道にする。
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
}: MessageRowProps) {
  const { t } = useTranslation();
  const { locale } = useLocale();
  const { message } = item;

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
            <MessageBody content={message.content} />
          )}
        </div>

        {isOwn && !isEditing ? (
          // 操作群の外枠ごと不可視の間はクリックを通す（子は自前で pointer-events を
          // 戻す）。外枠だけ残して当たり判定を奪わないための指定。
          <div className="pointer-events-none absolute right-1.5 top-0 flex items-center">
            <div className="pointer-events-none hidden items-center gap-0.5 rounded-lg border bg-popover p-0.5 opacity-0 shadow-sm transition-opacity group-hover/row:pointer-events-auto group-hover/row:opacity-100 group-focus-within/row:pointer-events-auto group-focus-within/row:opacity-100 [@media(hover:hover)]:flex">
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
            </div>
            <DropdownMenu>
              <DropdownMenuTrigger
                aria-label={t("messages.more")}
                title={t("messages.more")}
                className={buttonVariants({
                  variant: "ghost",
                  size: "icon-xs",
                  // タッチ端末では常時表示の逃げ道なので、外枠の pointer-events-none を
                  // ここだけ戻す。
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
          </div>
        ) : null}
      </div>
    </>
  );
}
