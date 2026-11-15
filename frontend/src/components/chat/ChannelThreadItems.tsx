import { useEffect, useState } from "react";
import { Link } from "@tanstack/react-router";
import { MessageSquareIcon } from "lucide-react";
import { useTranslation } from "react-i18next";
import { cn } from "cn";

import {
  nextThreadIndexSequence,
  threadSummaryRequestSequence,
  type ThreadSummary,
} from "../../hooks/useThreadIndex";
import { useThreadRoot } from "../../hooks/useThread";
import type { ChannelRow } from "../../lib/electric/types";
import { useShell } from "../layout/shell-context";
import { sidebarItemClassName } from "./sidebarStyles";

export function ChannelThreadItems({
  channel,
  threads,
  onObserveThread,
}: {
  channel: ChannelRow;
  threads: ThreadSummary[];
  onObserveThread?: (thread: ThreadSummary) => void;
}) {
  const { t } = useTranslation();
  const shell = useShell();
  const channelThreads = threads.filter((thread) => thread.channel_id === channel.id);
  const selectedId = shell.selectedChannelId === channel.id ? shell.threadRootId : null;
  const selectedQuery = useThreadRoot(selectedId ? channel.id : null, selectedId);
  const selectedRoot = selectedQuery.data?.[0];
  // 一覧の更新前に選択中の親がアーカイブされた場合も、移動後にツリーへ残さない。
  const [observedArchives, setObservedArchives] = useState(() => new Map<string, number>());
  const observedRootId = selectedRoot?.id;
  const observedArchiveAt = selectedRoot?.thread_archive_at ?? null;
  // 閲覧中は親投稿の状態を優先し、その行の一覧応答が届いた時点でも記録する。
  const observedIndexRow = threads.find((thread) => thread.id === observedRootId);
  useEffect(() => {
    if (!observedRootId) return;
    const observedSequence = nextThreadIndexSequence();
    setObservedArchives((previous) => {
      if (observedArchiveAt) return new Map(previous).set(observedRootId, observedSequence);
      if (!previous.has(observedRootId)) return previous;
      const next = new Map(previous);
      next.delete(observedRootId);
      return next;
    });
  }, [observedRootId, observedArchiveAt, observedIndexRow]);
  // ページ追加や遅い応答では補正を失わず、検知後に開始した取得だけを受け入れる。
  const active = channelThreads.filter(
    (thread) =>
      !thread.thread_archive_at &&
      threadSummaryRequestSequence(thread) > (observedArchives.get(thread.id) ?? -1),
  );
  const selected =
    selectedRoot && !selectedRoot.deleted_at
      ? {
          id: selectedRoot.id,
          channel_id: channel.id,
          content: selectedRoot.content,
          thread_archive_at: selectedRoot.thread_archive_at ?? null,
          reply_count:
            selectedRoot.display_data?.reply_count ??
            channelThreads.find((thread) => thread.id === selectedId)?.reply_count ??
            0,
        }
      : channelThreads.find((thread) => thread.id === selectedId);
  const selectedContent = selected?.content;
  const selectedCount = selected?.reply_count;
  useEffect(() => {
    if (!selectedRoot || selectedRoot.deleted_at || selectedCount === undefined) return;
    onObserveThread?.({
      id: selectedRoot.id,
      channel_id: channel.id,
      content: selectedContent ?? "",
      thread_archive_at: observedArchiveAt,
      reply_count: selectedCount,
    });
  }, [
    selectedRoot?.id,
    selectedRoot?.deleted_at,
    channel.id,
    selectedContent,
    selectedCount,
    observedArchiveAt,
    onObserveThread,
  ]);
  const visible = active.filter(
    (thread) => thread.id !== selectedId || !selectedRoot?.thread_archive_at,
  );
  if (
    selected &&
    (selected.thread_archive_at || selected.reply_count > 0) &&
    !visible.some((thread) => thread.id === selected.id)
  )
    visible.push(selected);
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
      onClick={() => {
        shell.showChat();
      }}
    >
      <MessageSquareIcon className="size-3 shrink-0" aria-hidden="true" />
      <span className="min-w-0 flex-1 truncate">{thread.content || t("threads.untitled")}</span>
      <span className="shrink-0 tabular-nums">{thread.reply_count}</span>
    </Link>
  );

  return (
    <div className="ml-5 flex min-w-0 flex-col border-l pl-2">{visible.map(renderThread)}</div>
  );
}
