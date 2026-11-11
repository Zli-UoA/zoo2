import { useState } from "react";
import { Link } from "@tanstack/react-router";
import { ArchiveIcon, MessageSquareIcon } from "lucide-react";
import { useTranslation } from "react-i18next";
import { cn } from "cn";

import { useThreadIndex, type ThreadSummary } from "../../hooks/useThreadIndex";
import type { ChannelRow } from "../../lib/electric/types";
import { useShell } from "../layout/shell-context";
import { Button } from "../ui/button";
import { CollapsibleSection } from "../ui/collapsible-section";
import { sidebarItemClassName } from "./sidebarStyles";

export function ChannelThreadItems({
  channel,
  threads,
  currentUserId,
}: {
  channel: ChannelRow;
  threads: ThreadSummary[];
  currentUserId: string | null;
}) {
  const { t } = useTranslation();
  const shell = useShell();
  const channelThreads = threads.filter((thread) => thread.channel_id === channel.id);
  const active = channelThreads.filter((thread) => !thread.thread_archive_at);
  const [historyOpen, setHistoryOpen] = useState(false);
  const history = useThreadIndex(currentUserId, {
    channelId: channel.id,
    archived: true,
    enabled: historyOpen,
  });

  const renderThread = (thread: ThreadSummary) => (
    <Link
      key={thread.id}
      to="/channels/$channelId"
      params={{ channelId: channel.id }}
      search={{ thread: thread.id }}
      aria-current={shell.threadRootId === thread.id ? "page" : undefined}
      className={cn(sidebarItemClassName(shell.threadRootId === thread.id, false), "text-xs")}
      title={t("threads.sidebarOpen", {
        title: thread.content || t("threads.untitled"),
        count: thread.reply_count,
      })}
      onClick={() => shell.showChat()}
    >
      <MessageSquareIcon className="size-3 shrink-0" aria-hidden="true" />
      <span className="min-w-0 flex-1 truncate">{thread.content || t("threads.untitled")}</span>
      <span className="shrink-0 tabular-nums">{thread.reply_count}</span>
    </Link>
  );

  return (
    <div className="ml-5 flex min-w-0 flex-col border-l pl-2">
      {active.map(renderThread)}

      <CollapsibleSection
        title={t("threads.archived")}
        ariaLabel={t("threads.archiveForChannel", { channel: channel.channel_name })}
        icon={<ArchiveIcon />}
        defaultOpen={false}
        onOpenChange={setHistoryOpen}
        className="-mx-2"
      >
        <div className="flex min-w-0 flex-col px-2">
          {history.threads.map(renderThread)}
          {historyOpen && !history.isLoading && !history.isError && !history.threads.length ? (
            <p className="p-2 text-xs text-muted-foreground">{t("threads.emptyArchive")}</p>
          ) : null}
          {history.isError ? (
            <p className="p-2 text-xs text-destructive">{t("threads.indexError")}</p>
          ) : null}
          {history.isLoading ? (
            <p className="p-2 text-xs text-muted-foreground">{t("threads.loading")}</p>
          ) : null}
          {history.hasMore || history.isError ? (
            <Button
              variant="ghost"
              size="sm"
              disabled={history.isLoading}
              onClick={history.loadMore}
            >
              {t(history.isError ? "threads.retry" : "threads.more")}
            </Button>
          ) : null}
        </div>
      </CollapsibleSection>
    </div>
  );
}
