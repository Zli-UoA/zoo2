import { cn } from "cn";
import { MoreHorizontalIcon, PencilIcon, TrashIcon, SmilePlusIcon } from "lucide-react";
import { Fragment } from "react";

import type { KitchenCatalog } from "../../../lib/emoji/kitchen";
import { kitchenLabel, reactionKey } from "../../../lib/emoji/kitchen";
import type { ReactionGroup } from "../../../lib/emoji/reactions";
import { ReactionEmoji } from "./ReactionEmoji";
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
  reactions: ReactionGroup[];
  kitchenCatalog: KitchenCatalog | null;
  canReact: boolean;
  onReaction: (emoji: string) => void;
  onPickReaction: () => void;
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
  return (
    <div
      className="flex items-center gap-3 py-2"
      role="separator"
      aria-label={formatDateSeparator(value)}
    >
      <span className="h-px flex-1 bg-border" />
      <span className="rounded-full border bg-background/80 px-2.5 py-0.5 text-[11px] font-medium text-muted-foreground">
        {formatDateSeparator(value)}
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
  reactions,
  kitchenCatalog,
  canReact,
  onReaction,
  onPickReaction,
}: MessageRowProps) {
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
              title={fullTimestamp(message.created_at)}
            >
              {formatTime(message.created_at)}
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
                title={fullTimestamp(message.created_at)}
              >
                {formatTime(message.created_at)}
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
                aria-label="メッセージを編集"
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
                  取消
                </Button>
                <Button type="submit" size="sm" disabled={!editingText.trim()}>
                  保存
                </Button>
              </div>
            </form>
          ) : (
            <MessageBody content={message.content} />
          )}
          {!isEditing ? (
            <div
              className="flex flex-wrap items-center gap-1 py-1"
              role="group"
              aria-label="リアクション"
            >
              {reactions.map((reaction) => (
                <button
                  key={reactionKey(reaction.emoji)}
                  type="button"
                  className={cn(
                    "inline-flex min-h-8 items-center gap-1 rounded-full border px-2 text-xs hover:bg-muted disabled:opacity-50",
                    reaction.ownId && "border-primary bg-primary/10",
                  )}
                  aria-pressed={!!reaction.ownId}
                  disabled={!canReact}
                  aria-label={`${kitchenLabel(reaction.emoji)} ${reaction.count}件${reaction.ownId ? "（解除）" : "（追加）"}`}
                  title={kitchenLabel(reaction.emoji)}
                  onClick={() => onReaction(reaction.emoji)}
                >
                  <ReactionEmoji emoji={reaction.emoji} catalog={kitchenCatalog} />
                  <span>{reaction.count}</span>
                </button>
              ))}
              <Button
                variant="ghost"
                size="icon-xs"
                aria-label="リアクションを追加"
                title="リアクションを追加"
                disabled={!canReact}
                onClick={onPickReaction}
              >
                <SmilePlusIcon />
              </Button>
            </div>
          ) : null}
        </div>

        {isOwn && !isEditing ? (
          // 操作群の外枠ごと不可視の間はクリックを通す（子は自前で pointer-events を
          // 戻す）。外枠だけ残して当たり判定を奪わないための指定。
          <div className="pointer-events-none absolute right-1.5 top-0 flex items-center">
            <div className="pointer-events-none hidden items-center gap-0.5 rounded-lg border bg-popover p-0.5 opacity-0 shadow-sm transition-opacity group-hover/row:pointer-events-auto group-hover/row:opacity-100 group-focus-within/row:pointer-events-auto group-focus-within/row:opacity-100 [@media(hover:hover)]:flex">
              <Button
                variant="ghost"
                size="icon-xs"
                aria-label="編集"
                title="編集"
                onClick={onStartEdit}
              >
                <PencilIcon />
              </Button>
              <Button
                variant="ghost"
                size="icon-xs"
                aria-label="削除"
                title="削除"
                className="text-destructive hover:bg-destructive/10 hover:text-destructive"
                onClick={onRequestDelete}
              >
                <TrashIcon />
              </Button>
            </div>
            <DropdownMenu>
              <DropdownMenuTrigger
                aria-label="その他の操作"
                title="その他の操作"
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
                  編集
                </DropdownMenuItem>
                <DropdownMenuItem variant="destructive" onClick={onRequestDelete}>
                  <TrashIcon />
                  削除
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
        ) : null}
      </div>
    </>
  );
}
