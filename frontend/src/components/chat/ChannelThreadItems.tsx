import { Link, useNavigate } from "@tanstack/react-router";
import { ArchiveIcon, MessageSquareIcon, RotateCcwIcon } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { cn } from "cn";

import type { ThreadSummary } from "../../hooks/useThreadIndex";
import type { ChannelRow } from "../../lib/electric/types";
import { useOfflineStatus } from "../../lib/offline/offline-context";
import { useShell } from "../layout/shell-context";
import { Button } from "../ui/button";
import { CollapsibleSection } from "../ui/collapsible-section";
import { sidebarItemClassName } from "./sidebarStyles";

export function ChannelThreadItems({
  channel,
  threads,
  onUnarchive,
}: {
  channel: ChannelRow;
  threads: ThreadSummary[];
  onUnarchive: (id: string) => Promise<void>;
}) {
  const { t } = useTranslation();
  const shell = useShell();
  const navigate = useNavigate();
  const { isOnline } = useOfflineStatus();
  const [busyId, setBusyId] = useState<string | null>(null);
  const channelThreads = threads.filter((thread) => thread.channel_id === channel.id);
  const active = channelThreads.filter((thread) => !thread.thread_archive_at);
  const archived = channelThreads.filter((thread) => !!thread.thread_archive_at);

  const unarchive = async (id: string) => {
    setBusyId(id);
    try {
      await onUnarchive(id);
      shell.showChat();
      await navigate({
        to: "/channels/$channelId",
        params: { channelId: channel.id },
        search: { thread: id },
      });
    } catch {
      toast.error(t("threads.unarchiveError"));
    } finally {
      setBusyId(null);
    }
  };

  const renderThread = (thread: ThreadSummary) => (
    <div key={thread.id} className="flex min-w-0 items-center gap-1">
      <Link
        to="/channels/$channelId"
        params={{ channelId: channel.id }}
        search={{ thread: thread.id }}
        aria-current={shell.threadRootId === thread.id ? "page" : undefined}
        className={cn(
          sidebarItemClassName(shell.threadRootId === thread.id, false),
          "flex-1 text-xs",
        )}
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
      {thread.thread_archive_at && !channel.archive_at ? (
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={t("threads.unarchive")}
          title={t("threads.unarchive")}
          disabled={busyId !== null || !isOnline}
          onClick={() => void unarchive(thread.id)}
        >
          <RotateCcwIcon />
        </Button>
      ) : null}
    </div>
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
