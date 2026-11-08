import { sidebarItemClassName } from "./sidebarStyles";
import { useTranslation } from "react-i18next";
import { Link } from "@tanstack/react-router";
import { UsersIcon } from "lucide-react";
import type { ReactNode } from "react";

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
}: {
  channel: ChannelRow;
  label: string;
  avatarUrl?: string | null;
  active: boolean;
  unread?: boolean;
  onOpen: () => void;
  trailing?: ReactNode;
}) {
  const { t } = useTranslation();
  const group = isGroupDm(channel.channel_name);
  return (
    <Link
      to="/channels/$channelId"
      params={{ channelId: channel.id }}
      onClick={onOpen}
      aria-current={active ? "page" : undefined}
      className={sidebarItemClassName(active, unread)}
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
  );
}
