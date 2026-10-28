import { useTranslation } from "react-i18next";
import { cn } from "cn";
import { PlusIcon } from "lucide-react";
import { useId, useMemo } from "react";

import type { ChannelRow, ChannelUserRow, UserRow } from "../../lib/electric/types";
import { useLocale } from "../i18n/LocaleProvider";
import { Button } from "../ui/button";
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
  activeChannelId,
  unreadChannelIds,
  onCreate,
  onOpen,
  className,
}: {
  channels: ChannelRow[];
  memberships: ChannelUserRow[];
  users: UserRow[];
  currentUserId: string | null;
  loading: boolean;
  error: boolean;
  activeChannelId: string | null;
  unreadChannelIds: ReadonlySet<string>;
  onCreate: () => void;
  onOpen: () => void;
  className?: string;
}) {
  const { t } = useTranslation();
  const { locale } = useLocale();
  const headingId = useId();

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
    <section className={cn("flex flex-col", className)} aria-labelledby={headingId}>
      <div className="flex items-center justify-between gap-2 px-3 py-2">
        <div className="flex items-baseline gap-2">
          <h2
            id={headingId}
            className="font-heading text-xs font-semibold tracking-wide text-muted-foreground"
          >
            {t("dms.title")}
          </h2>
          <span className="text-xs tabular-nums text-muted-foreground">
            {loading ? "…" : channels.length}
          </span>
        </div>
        <Button
          variant="ghost"
          size="icon"
          className="size-11"
          aria-label={t("dms.createTitle")}
          title={t("dms.createTitle")}
          onClick={onCreate}
        >
          <PlusIcon />
        </Button>
      </div>

      <div className="flex flex-col gap-0.5 px-2 pb-2">
        {loading ? (
          SKELETON_ROWS.map((row) => <Skeleton key={row} className="h-11 w-full rounded-lg" />)
        ) : error ? (
          <p className="px-2 py-1 text-xs text-destructive">{t("dms.loadError")}</p>
        ) : channels.length === 0 ? (
          <div className="flex flex-col items-start gap-2 px-2 py-3">
            <p className="text-sm text-muted-foreground">{t("dms.empty")}</p>
            <Button variant="outline" size="sm" className="min-h-11" onClick={onCreate}>
              <PlusIcon />
              {t("dms.newAction")}
            </Button>
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
    </section>
  );
}
