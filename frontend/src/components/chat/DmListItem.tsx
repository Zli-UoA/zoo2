import { sidebarItemClassName } from "./sidebarStyles";
import { useTranslation } from "react-i18next";
import { Link } from "@tanstack/react-router";
import { LockKeyholeIcon, UsersIcon } from "lucide-react";
import type { DragEvent, KeyboardEventHandler, ReactNode } from "react";
import { cn } from "cn";

import type { ChannelRow } from "../../lib/electric/types";
import { AppAvatar } from "./AppIdentity";
import { isGroupDm } from "./dm";

export function DmListItem({
  channel,
  label,
  avatarUrl,
  active,
  unread = false,
  onOpen,
  trailing,
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
  label: string;
  avatarUrl?: string | null;
  active: boolean;
  unread?: boolean;
  onOpen: () => void;
  trailing?: ReactNode;
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
  const group = isGroupDm(channel.channel_name);
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
      {dropIndicator ? (
        <span
          aria-hidden="true"
          data-drop-indicator={dropIndicator}
          className={cn(
            "pointer-events-none absolute inset-x-0 z-20 h-0.5 rounded-full bg-brand",
            dropIndicator === "before" ? "-top-px" : "-bottom-px",
          )}
        />
      ) : null}
      <Link
        to="/channels/$channelId"
        params={{ channelId: channel.id }}
        onClick={onOpen}
        aria-current={active ? "page" : undefined}
        // 行全体のドラッグを使う。リンク自身のドラッグで並び替えを妨げない。
        draggable={false}
        className={cn(
          "min-w-0 flex-1",
          sidebarItemClassName(active, unread),
          actions && "pr-12 max-md:pr-14",
        )}
      >
        {group ? (
          <span
            className="flex size-6 shrink-0 items-center justify-center rounded-full bg-muted text-muted-foreground"
            aria-hidden="true"
          >
            <UsersIcon className="size-3.5" />
          </span>
        ) : (
          <AppAvatar label={label} src={avatarUrl} size="sm" aria-hidden />
        )}
        <span className="min-w-0 flex-1 truncate">{label}</span>
        {channel.encrypted ? (
          // 同じ相手との平文の DM と暗号化 DM は別の項目になるので、開く前に区別できるようにする。
          <span
            role="img"
            aria-label={t("e2ee.badge")}
            title={t("e2ee.badge")}
            className="shrink-0 text-muted-foreground"
          >
            <LockKeyholeIcon className="size-3.5" aria-hidden="true" />
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
        {trailing}
      </Link>
      {actions ? (
        <div className="absolute inset-y-0 right-0.5 flex items-center opacity-0 transition-opacity group-hover/row:opacity-100 group-focus-within/row:opacity-100 max-md:opacity-100">
          {actions}
        </div>
      ) : null}
    </div>
  );
}
