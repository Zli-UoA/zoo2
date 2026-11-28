import { Link } from "@tanstack/react-router";
import { HashIcon, HeadphonesIcon, LockIcon, LockKeyholeIcon } from "lucide-react";
import type { DragEvent, KeyboardEventHandler, ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "cn";

import { useCall } from "../../lib/call/call-context";
import type { ChannelRow } from "../../lib/electric/types";
import { sidebarItemClassName } from "./sidebarStyles";
import { DropIndicator } from "./DropIndicator";

/**
 * サイドバーの通常チャンネル行。チャンネル一覧とピン留めの両方で使い、
 * NSFW の印・通話中のアイコン・未読の点・行メニューの余白をそろえる。
 */
export function ChannelListItem({
  channel,
  active,
  unread = false,
  onOpen,
  actions,
  draggable = false,
  dragging = false,
  onDragStart,
  onDragEnd,
  onDragOver,
  onDragLeave,
  onDrop,
  dropIndicator = null,
  onKeyDown,
}: {
  channel: ChannelRow;
  active: boolean;
  unread?: boolean;
  onOpen: () => void;
  /** 行の右端に出すメニューなど。リンクの外に置き、入れ子の操作要素を作らない。 */
  actions?: ReactNode;
  draggable?: boolean;
  dragging?: boolean;
  onDragStart?: (event: DragEvent<HTMLDivElement>) => void;
  onDragEnd?: () => void;
  onDragOver?: (event: DragEvent<HTMLDivElement>) => void;
  onDragLeave?: (event: DragEvent<HTMLDivElement>) => void;
  onDrop?: (event: DragEvent<HTMLDivElement>) => void;
  /** ドラッグ中の挿入位置。行の上端・下端に線を出す。 */
  dropIndicator?: "before" | "after" | null;
  onKeyDown?: KeyboardEventHandler<HTMLDivElement>;
}) {
  const { t } = useTranslation();
  const call = useCall();
  const hasActiveCall = (call.activeCalls.get(channel.id) ?? 0) > 0;

  return (
    <div
      className={cn(
        "group/row relative flex min-w-0 items-center",
        draggable && "cursor-grab",
        dragging && "opacity-60",
      )}
      draggable={draggable}
      onDragStart={
        draggable
          ? (event) => {
              event.dataTransfer.effectAllowed = "move";
              event.dataTransfer.setData("text/plain", channel.id);
              onDragStart?.(event);
            }
          : undefined
      }
      onDragEnd={onDragEnd}
      onDragOver={onDragOver}
      onDragLeave={onDragLeave}
      onDrop={
        draggable || onDrop
          ? (event) => {
              event.preventDefault();
              onDrop?.(event);
            }
          : undefined
      }
      onKeyDown={onKeyDown}
    >
      {dropIndicator ? <DropIndicator edge={dropIndicator} /> : null}
      <Link
        to="/channels/$channelId"
        params={{ channelId: channel.id }}
        aria-current={active ? "page" : undefined}
        onClick={onOpen}
        // 行全体のドラッグを使う。リンク自身のドラッグで並び替えを妨げない。
        draggable={false}
        className={cn(
          "min-w-0 flex-1",
          sidebarItemClassName(active, unread),
          actions && "pr-16 max-md:pr-20",
        )}
      >
        {channel.encrypted ? (
          <span role="img" aria-label={t("e2ee.badge")} title={t("e2ee.badge")}>
            <LockKeyholeIcon className="size-4 shrink-0" aria-hidden="true" />
          </span>
        ) : channel.is_private ? (
          <LockIcon className="size-4 shrink-0" aria-hidden="true" />
        ) : (
          <HashIcon className="size-4 shrink-0 opacity-60" aria-hidden="true" />
        )}
        <span className="min-w-0 flex-1 truncate">{channel.channel_name}</span>
        {channel.is_nsfw ? (
          <span className="shrink-0 text-xs text-muted-foreground">NSFW</span>
        ) : null}
        {hasActiveCall ? (
          <span
            role="img"
            title={t("call.inCall")}
            aria-label={t("call.inCall")}
            className="flex items-center text-success"
          >
            <HeadphonesIcon className="size-3 animate-pulse" />
          </span>
        ) : null}
        {unread ? (
          <span
            role="img"
            aria-label={t("channels.unread")}
            title={t("channels.unread")}
            className="size-2 shrink-0 rounded-full bg-brand"
          />
        ) : null}
      </Link>
      {actions ? (
        <div className="absolute inset-y-0 right-0.5 flex items-center opacity-0 transition-opacity group-hover/row:opacity-100 group-focus-within/row:opacity-100 max-md:opacity-100">
          {actions}
        </div>
      ) : null}
    </div>
  );
}
