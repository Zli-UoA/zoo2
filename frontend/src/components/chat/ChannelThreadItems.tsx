import { Link } from "@tanstack/react-router";
import { ArchiveIcon, MessageSquareIcon } from "lucide-react";
import { useTranslation } from "react-i18next";
import { cn } from "cn";

import type { ThreadSummary } from "../../hooks/useThreadIndex";
import type { ChannelRow } from "../../lib/electric/types";
import { useShell } from "../layout/shell-context";
import { CollapsibleSection } from "../ui/collapsible-section";
import { sidebarItemClassName } from "./sidebarStyles";

export function ChannelThreadItems({
  channel,
  threads,
}: {
  channel: ChannelRow;
  threads: ThreadSummary[];
}) {
  const { t } = useTranslation();
  const shell = useShell();
  const channelThreads = threads.filter((thread) => thread.channel_id === channel.id);
  const active = channelThreads.filter((thread) => !thread.thread_archive_at);
  const archived = channelThreads.filter((thread) => !!thread.thread_archive_at);

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

  if (!channelThreads.length) return null;

  return (
    <div className="ml-5 flex min-w-0 flex-col border-l pl-2">
      {active.map(renderThread)}
      {archived.length ? (
        <CollapsibleSection
          title={t("threads.archivedCount", { count: archived.length })}
          icon={<ArchiveIcon />}
          defaultOpen={false}
          className="-mx-2"
        >
          <div className="flex min-w-0 flex-col px-2">{archived.map(renderThread)}</div>
        </CollapsibleSection>
      ) : null}
    </div>
  );
}
