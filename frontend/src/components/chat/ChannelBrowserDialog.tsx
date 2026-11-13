import { Input } from "../ui/input";
import { useTranslation } from "react-i18next";
import { Link } from "@tanstack/react-router";
import { cn } from "cn";
import { ArchiveIcon, ArrowUpDownIcon, HashIcon, SearchIcon, XIcon } from "lucide-react";
import { useMemo, useState } from "react";

import type { ChannelRow } from "../../lib/electric/types";
import { Button } from "../ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "../ui/dropdown-menu";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "../ui/dialog";
import { inputClassName } from "../ui/input-styles";
import { ScrollArea } from "../ui/scroll-area";
import {
  CHANNEL_SORTS,
  DEFAULT_CHANNEL_SORT,
  filterDiscoverableChannels,
  isChannelJoinable,
  isChannelSort,
  sortChannels,
  type ChannelSort,
} from "./channelSearch";

/**
 * 未参加の公開チャンネルとアーカイブ済みチャンネルを探す。行を押すとチャンネルを開き、
 * 「参加」ボタンは開かずにこの画面で参加する（アーカイブ済みは参加不可）。`onJoinChannel` は
 * 失敗時に reject し、ここでエラーを表示する。
 */
export function ChannelBrowserDialog({
  open,
  onOpenChange,
  channels,
  joinedChannelIds,
  onJoinChannel,
  memberCounts,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  channels: ChannelRow[];
  joinedChannelIds: ReadonlySet<string>;
  onJoinChannel?: (channelId: string) => Promise<void>;
  /** チャンネル ID ごとのメンバー数。メンバー数での並び替えに使う。 */
  memberCounts?: ReadonlyMap<string, number>;
}) {
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const [joinError, setJoinError] = useState<string | null>(null);
  const [joiningIds, setJoiningIds] = useState<ReadonlySet<string>>(new Set());
  const [sort, setSort] = useState<ChannelSort>(DEFAULT_CHANNEL_SORT);
  const discoverableChannels = useMemo(
    () =>
      sortChannels(
        filterDiscoverableChannels(channels, joinedChannelIds, query),
        sort,
        memberCounts,
      ),
    [channels, joinedChannelIds, query, sort, memberCounts],
  );

  async function handleJoin(channelId: string) {
    if (!onJoinChannel || joiningIds.has(channelId)) {
      return;
    }
    setJoinError(null);
    setJoiningIds((prev) => new Set(prev).add(channelId));
    try {
      await onJoinChannel(channelId);
    } catch (cause) {
      setJoinError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setJoiningIds((prev) => {
        const next = new Set(prev);
        next.delete(channelId);
        return next;
      });
    }
  }

  function handleOpenChange(next: boolean) {
    if (!next) {
      setQuery("");
      setJoinError(null);
    }
    onOpenChange(next);
  }

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>{t("channels.browseTitle")}</DialogTitle>
          <DialogDescription>{t("channels.browseDescription")}</DialogDescription>
        </DialogHeader>

        <div className="flex items-center gap-2">
          <div className="relative min-w-0 flex-1">
            <SearchIcon
              aria-hidden="true"
              className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground"
            />
            <Input
              type="search"
              className={cn(
                inputClassName(),
                "w-full pr-8 pl-8 [&::-webkit-search-cancel-button]:appearance-none",
              )}
              placeholder={t("channels.browseSearch")}
              aria-label={t("channels.browseSearch")}
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Escape" && query) {
                  event.preventDefault();
                  event.stopPropagation();
                  setQuery("");
                }
              }}
              autoComplete="off"
              spellCheck={false}
              autoFocus
            />
            {query ? (
              <Button
                type="button"
                variant="ghost"
                size="icon-xs"
                className="absolute top-1/2 right-1 -translate-y-1/2"
                aria-label={t("channels.searchClear")}
                title={t("channels.searchClear")}
                onClick={() => setQuery("")}
              >
                <XIcon />
              </Button>
            ) : null}
          </div>
          <DropdownMenu>
            <DropdownMenuTrigger
              render={
                <Button
                  type="button"
                  variant="outline"
                  size="icon"
                  aria-label={`${t("channels.sortLabel")}: ${t(`channels.sort.${sort}`)}`}
                  title={`${t("channels.sortLabel")}: ${t(`channels.sort.${sort}`)}`}
                />
              }
            >
              <ArrowUpDownIcon aria-hidden="true" />
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              <DropdownMenuRadioGroup
                value={sort}
                onValueChange={(value) => {
                  if (isChannelSort(value)) setSort(value);
                }}
              >
                {CHANNEL_SORTS.map((value) => (
                  <DropdownMenuRadioItem key={value} value={value} closeOnClick>
                    {t(`channels.sort.${value}`)}
                  </DropdownMenuRadioItem>
                ))}
              </DropdownMenuRadioGroup>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>

        {joinError ? (
          <p role="alert" className="text-xs text-destructive">
            {t("common.writeError")}
            <span className="sr-only">: {joinError}</span>
          </p>
        ) : null}

        <div className="h-80 min-h-0 min-w-0">
          {discoverableChannels.length === 0 ? (
            <p className="flex h-full items-center justify-center px-2 text-center text-sm text-muted-foreground">
              {query.trim() ? t("channels.searchEmpty") : t("channels.browseEmpty")}
            </p>
          ) : (
            <ScrollArea className="h-full">
              <ul className="flex flex-col gap-0.5 pr-2">
                {discoverableChannels.map((channel) => (
                  <li
                    key={channel.id}
                    className="flex min-h-14 items-center gap-1 rounded-lg pr-3 focus-within:ring-2 focus-within:ring-ring hover:bg-muted"
                  >
                    <Link
                      to="/channels/$channelId"
                      params={{ channelId: channel.id }}
                      onClick={() => handleOpenChange(false)}
                      className="flex min-h-14 min-w-0 flex-1 items-center gap-3 rounded-lg py-2 pl-3 outline-none"
                    >
                      <HashIcon className="size-4 shrink-0 opacity-60" aria-hidden="true" />
                      <div className="min-w-0 flex-1">
                        <p className="truncate text-sm text-foreground">{channel.channel_name}</p>
                        {channel.description?.trim() ? (
                          <p className="truncate text-xs text-muted-foreground">
                            {channel.description.trim()}
                          </p>
                        ) : null}
                      </div>
                      {channel.archive_at ? (
                        <span
                          role="img"
                          title={t("moderation.archived")}
                          aria-label={t("moderation.archived")}
                          className="shrink-0 text-muted-foreground/70"
                        >
                          <ArchiveIcon className="size-4" aria-hidden="true" />
                        </span>
                      ) : null}
                    </Link>
                    {onJoinChannel && isChannelJoinable(channel) ? (
                      <Button
                        type="button"
                        size="sm"
                        variant="outline"
                        disabled={joiningIds.has(channel.id)}
                        aria-busy={joiningIds.has(channel.id)}
                        onClick={() => void handleJoin(channel.id)}
                      >
                        {joiningIds.has(channel.id) ? t("channels.joining") : t("channels.join")}
                      </Button>
                    ) : null}
                  </li>
                ))}
              </ul>
            </ScrollArea>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}
