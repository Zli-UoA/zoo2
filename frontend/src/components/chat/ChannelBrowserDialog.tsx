import { useTranslation } from "react-i18next";
import { useNavigate } from "@tanstack/react-router";
import { cn } from "cn";
import { HashIcon, SearchIcon, XIcon } from "lucide-react";
import { useMemo, useState } from "react";

import { useCurrentUser } from "../../hooks/useCurrentUser";
import { useCollections } from "../../lib/electric/collections-context";
import type { ChannelRow, ChannelUserRow } from "../../lib/electric/types";
import type { ChannelUserWriteMetadata } from "../../lib/electric/writes";
import { localizeApiError } from "../../lib/i18n/apiError";
import { useShell } from "../layout/shell-context";
import { Button } from "../ui/button";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "../ui/dialog";
import { inputClassName } from "../ui/input-styles";
import { ScrollArea } from "../ui/scroll-area";
import { filterDiscoverableChannels, isChannelJoinable } from "./channelSearch";

function nowIso(): string {
  return new Date().toISOString();
}

/**
 * 未参加の公開チャンネルを探して参加するダイアログ。
 *
 * 候補は `channels` コレクションから引き、参加は `channelUsers.insert()` の
 * `join` アクションで行う（バックエンドは既存の `/join` を叩く）。
 * 参加が確定したらそのチャンネルへ移動する。
 */
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
  const shell = useShell();
  const navigate = useNavigate();
  const { channelUsers } = useCollections();
  const { currentUserId, isLoading: currentUserLoading } = useCurrentUser();
  const [query, setQuery] = useState("");
  const [joiningId, setJoiningId] = useState<string | null>(null);
  // 言語を切り替えても文言が追従するよう、翻訳前の原因を保持する。
  const [errorCause, setErrorCause] = useState<{ cause: unknown } | null>(null);

  const discoverableChannels = useMemo(
    () => filterDiscoverableChannels(channels, joinedChannelIds, query),
    [channels, joinedChannelIds, query],
  );

  function handleOpenChange(next: boolean) {
    if (!next) {
      setQuery("");
      setJoiningId(null);
      setErrorCause(null);
    }
    onOpenChange(next);
  }

  async function joinChannel(channel: ChannelRow) {
    if (!currentUserId || currentUserLoading || joiningId !== null || !isChannelJoinable(channel)) {
      return;
    }
    setJoiningId(channel.id);
    setErrorCause(null);
    try {
      const transaction = channelUsers.insert(
        {
          user_id: currentUserId,
          channel_id: channel.id,
          joined_at: nowIso(),
        } satisfies ChannelUserRow,
        { metadata: { action: "join" } satisfies ChannelUserWriteMetadata },
      );
      await transaction.when("settled");
      handleOpenChange(false);
      shell.showChat();
      await navigate({
        to: "/channels/$channelId",
        params: { channelId: channel.id },
      });
    } catch (cause) {
      setErrorCause({ cause });
      setJoiningId(null);
    }
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
          <input
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
                {discoverableChannels.map((channel) => {
                  const joinable = isChannelJoinable(channel);
                  const joining = joiningId === channel.id;
                  return (
                    <li key={channel.id} className="flex items-center gap-2 rounded-lg px-1 py-1.5">
                      <HashIcon className="ml-1 size-4 shrink-0 opacity-60" aria-hidden="true" />
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
                      <Button
                        variant="outline"
                        size="sm"
                        disabled={
                          !joinable || joiningId !== null || currentUserLoading || !currentUserId
                        }
                        onClick={() => void joinChannel(channel)}
                      >
                        {joining
                          ? t("channels.joining")
                          : joinable
                            ? t("channels.join")
                            : t("channels.joinUnavailable")}
                      </Button>
                    </li>
                  );
                })}
              </ul>
            </ScrollArea>
          )}
        </div>

        {errorCause ? (
          <p className="text-xs text-destructive">
            {t("common.writeErrorDetail", {
              error: localizeApiError(errorCause.cause, t),
            })}
          </p>
        ) : null}
      </DialogContent>
    </Dialog>
  );
}
