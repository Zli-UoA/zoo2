import { useTranslation } from "react-i18next";
import { PlusIcon, UsersIcon } from "lucide-react";
import { useMemo } from "react";

import type { ChannelRow, ChannelUserRow, UserRow } from "../../lib/electric/types";
import { useLocale } from "../i18n/LocaleProvider";
import { Button } from "../ui/button";
import { CollapsibleSection } from "../ui/collapsible-section";
import { Skeleton } from "../ui/skeleton";
import { resolveDmDisplayName } from "./dm";
import { DmListItem } from "./DmListItem";

const SKELETON_ROWS = [0, 1, 2];

export function DmSection({
  channels,
  memberships,
  users,
  currentUserId,
  loading,
  error,
  usersError,
  activeChannelId,
  unreadChannelIds,
  onCreate,
  onOpen,
  onReload,
  className,
}: {
  channels: ChannelRow[];
  memberships: ChannelUserRow[];
  users: UserRow[];
  currentUserId: string | null;
  loading: boolean;
  error: boolean;
  usersError: boolean;
  activeChannelId: string | null;
  unreadChannelIds: ReadonlySet<string>;
  onCreate: () => void;
  onOpen: () => void;
  onReload: () => void;
  className?: string;
}) {
  const { t } = useTranslation();
  const { locale } = useLocale();

  const membersByChannel = useMemo(() => {
    const map = new Map<string, ChannelUserRow[]>();
    for (const membership of memberships) {
      const rows = map.get(membership.channel_id);
      if (rows) {
        rows.push(membership);
      } else {
        map.set(membership.channel_id, [membership]);
      }
    }
    return map;
  }, [memberships]);

  const labels = useMemo(() => {
    const map = new Map<string, string>();
    for (const channel of channels) {
      const resolved = resolveDmDisplayName({
        channelName: channel.channel_name,
        members: membersByChannel.get(channel.id) ?? [],
        users,
        currentUserId,
        locale,
      });
      const names = resolved.names.join(", ");
      const base = names || t("dms.untitled");
      map.set(
        channel.id,
        resolved.remaining > 0
          ? `${base} ${t("dms.moreNames", { count: resolved.remaining })}`
          : base,
      );
    }
    return map;
  }, [channels, currentUserId, locale, membersByChannel, t, users]);

  return (
    <CollapsibleSection
      title={t("dms.title")}
      icon={<UsersIcon />}
      count={loading ? "…" : channels.length}
      className={className}
      actions={
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={t("dms.createTitle")}
          title={t("dms.createTitle")}
          onClick={onCreate}
        >
          <PlusIcon />
        </Button>
      }
    >
      <div className="flex flex-col gap-0.5 px-2 pb-2">
        {loading ? (
          SKELETON_ROWS.map((row) => (
            <Skeleton key={row} className="h-9 w-full rounded-md md:h-8" />
          ))
        ) : error ? (
          <p className="px-2 py-1 text-xs text-destructive">{t("dms.loadError")}</p>
        ) : usersError ? (
          <div role="alert" className="flex flex-col items-start gap-2 px-2 py-2">
            <p className="text-xs text-destructive">{t("dms.usersError")}</p>
            <Button variant="outline" size="sm" className="min-h-11" onClick={onReload}>
              {t("dms.reload")}
            </Button>
          </div>
        ) : channels.length === 0 ? (
          <div className="flex flex-col items-start gap-2 px-2 py-2">
            <p className="text-sm text-muted-foreground">{t("dms.empty")}</p>
          </div>
        ) : (
          channels.map((channel) => (
            <DmListItem
              key={channel.id}
              channel={channel}
              label={labels.get(channel.id) ?? t("dms.untitled")}
              active={activeChannelId === channel.id}
              unread={unreadChannelIds.has(channel.id)}
              onOpen={onOpen}
            />
          ))
        )}
      </div>
    </CollapsibleSection>
  );
}
