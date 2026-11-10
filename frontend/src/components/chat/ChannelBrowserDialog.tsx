import { Input } from "../ui/input";
import { useTranslation } from "react-i18next";
import { Link } from "@tanstack/react-router";
import { cn } from "cn";
import { HashIcon, SearchIcon, XIcon } from "lucide-react";
import { useMemo, useState } from "react";

import type { ChannelRow } from "../../lib/electric/types";
import { Button } from "../ui/button";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "../ui/dialog";
import { inputClassName } from "../ui/input-styles";
import { ScrollArea } from "../ui/scroll-area";
import { filterDiscoverableChannels } from "./channelSearch";

/** 未参加の公開チャンネルを探す。参加は閲覧後の送信欄で行う。 */
export function ChannelBrowserDialog({
  open,
  onOpenChange,
  channels,
  joinedChannelIds,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  channels: ChannelRow[];
  joinedChannelIds: ReadonlySet<string>;
}) {
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const discoverableChannels = useMemo(
    () => filterDiscoverableChannels(channels, joinedChannelIds, query),
    [channels, joinedChannelIds, query],
  );

  function handleOpenChange(next: boolean) {
    if (!next) {
      setQuery("");
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

        <div className="relative min-w-0">
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

        <div className="h-80 min-h-0 min-w-0">
          {discoverableChannels.length === 0 ? (
            <p className="flex h-full items-center justify-center px-2 text-center text-sm text-muted-foreground">
              {query.trim() ? t("channels.searchEmpty") : t("channels.browseEmpty")}
            </p>
          ) : (
            <ScrollArea className="h-full">
              <ul className="flex flex-col gap-0.5 pr-2">
                {discoverableChannels.map((channel) => (
                  <li key={channel.id}>
                    <Link
                      to="/channels/$channelId"
                      params={{ channelId: channel.id }}
                      onClick={() => handleOpenChange(false)}
                      className="flex min-h-14 items-center gap-3 rounded-lg px-3 py-2 outline-none hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring"
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
                        <span className="shrink-0 rounded bg-muted px-1.5 py-0.5 text-[0.65rem] text-muted-foreground">
                          {t("moderation.archived")}
                        </span>
                      ) : null}
                      <span className="shrink-0 text-xs font-medium text-brand">
                        {t("channels.view")}
                      </span>
                    </Link>
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
