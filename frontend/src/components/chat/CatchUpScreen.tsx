import { useLiveQuery } from "@tanstack/react-db";
import { useNavigate } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { useCurrentUser } from "../../hooks/useCurrentUser";
import { READ_STATE_CHANGED, useUnread, type ChannelReadState } from "../../hooks/useUnread";
import { useActiveChannels, useUsers } from "../../hooks/useRealtime";
import { apiPath } from "../../lib/api/path";
import { createConfiguredApiClient } from "../../lib/api/client";
import { useCollections } from "../../lib/electric/collections-context";
import { useLocale } from "../i18n/LocaleProvider";
import { Button } from "../ui/button";
import { Skeleton } from "../ui/skeleton";
import { buildDmLabels } from "./dm";
import { CatchUpMessagePage } from "./CatchUpMessagePage";

/**
 * SlackモバイルのCatch up相当。未読の会話を古い順に1件ずつ出し、既読にするか
 * 未読のまま次へ進むかを選ぶ。キューは開いた時点で固定し、再計算しない。
 */
export function CatchUpScreen() {
  const { t } = useTranslation();
  const { locale } = useLocale();
  const navigate = useNavigate();
  const { config } = useCollections();
  const { currentUserId } = useCurrentUser();
  const { states, unreadStates, isReady, isError, refresh, applyRead } = useUnread();
  const channelsQuery = useActiveChannels();
  const usersQuery = useUsers();
  const { channelUsers } = useCollections();
  const membershipsQuery = useLiveQuery((q) => q.from({ channelUsers }));
  const client = useMemo(() => createConfiguredApiClient(config), [config]);

  const [queue, setQueue] = useState<ChannelReadState[] | null>(null);
  const [index, setIndex] = useState(0);
  const [readCount, setReadCount] = useState(0);
  const [skippedCount, setSkippedCount] = useState(0);
  const [writeError, setWriteError] = useState(false);
  const [isWriting, setIsWriting] = useState(false);
  const [boundary, setBoundary] = useState<{ channelId: string; messageId: string | null } | null>(
    null,
  );
  const headingRef = useRef<HTMLHeadingElement>(null);

  const start = useCallback(() => {
    setQueue(unreadStates.map((state) => ({ ...state })));
    setIndex(0);
    setReadCount(0);
    setSkippedCount(0);
    setWriteError(false);
    setBoundary(null);
  }, [unreadStates]);

  useEffect(() => {
    if (queue === null && isReady && unreadStates.length > 0) start();
  }, [queue, isReady, unreadStates.length, start]);

  const channelById = useMemo(
    () => new Map((channelsQuery.data ?? []).map((channel) => [channel.id, channel])),
    [channelsQuery.data],
  );
  const labels = useMemo(() => {
    const channels = [...channelById.values()];
    const dmLabels = buildDmLabels({
      channels,
      memberships: membershipsQuery.data ?? [],
      users: usersQuery.data ?? [],
      currentUserId,
      locale,
      t,
    });
    const result = new Map<string, string>();
    for (const channel of channels) {
      result.set(
        channel.id,
        channel.is_dm
          ? (dmLabels.get(channel.id) ?? t("catchUp.conversation"))
          : `#${channel.channel_name}`,
      );
    }
    return result;
  }, [channelById, membershipsQuery.data, usersQuery.data, currentUserId, locale, t]);

  const current = queue && queue[index];
  const total = queue?.length ?? 0;
  const goNext = useCallback(() => {
    setWriteError(false);
    setBoundary(null);
    setIndex((value) => value + 1);
  }, []);

  // 別タブ・端末で既読になった会話は、表示時に未読が無ければ飛ばす。
  useEffect(() => {
    if (!current || isWriting || writeError) return;
    const live = states.find((state) => state.channel_id === current.channel_id);
    if (live && !live.has_unread) goNext();
  }, [states, current, isWriting, writeError, goNext]);

  useEffect(() => {
    headingRef.current?.focus();
  }, [index, queue]);

  const onBoundaryMessage = useCallback(
    (messageId: string | null) => setBoundary({ channelId: current?.channel_id ?? "", messageId }),
    [current?.channel_id],
  );

  const markRead = async () => {
    if (!current || isWriting) return;
    const messageId = boundary?.channelId === current.channel_id ? boundary.messageId : null;
    if (!messageId) {
      setWriteError(true);
      return;
    }
    setIsWriting(true);
    setWriteError(false);
    try {
      await client.post<unknown>(apiPath`/api/channels/${current.channel_id}/read-state`, {
        message_id: messageId,
      });
      applyRead(current.channel_id);
      window.dispatchEvent(new Event(READ_STATE_CHANGED));
      setReadCount((value) => value + 1);
      goNext();
    } catch {
      setWriteError(true);
    } finally {
      setIsWriting(false);
    }
  };

  const keepUnread = () => {
    setSkippedCount((value) => value + 1);
    goNext();
  };

  const restart = () => {
    refresh();
    setQueue(null);
  };

  return (
    <main className="flex h-full min-h-0 flex-col bg-background">
      <header className="flex items-center justify-between gap-3 border-b px-4 py-3">
        <div className="min-w-0">
          <h1 className="truncate text-lg font-semibold" tabIndex={-1} ref={headingRef}>
            {t("catchUp.title")}
          </h1>
          {current ? (
            <p className="truncate text-sm text-muted-foreground">
              {current.unread_count_truncated
                ? t("catchUp.overCount", { count: current.unread_count })
                : t("catchUp.count", { count: current.unread_count })}
            </p>
          ) : null}
        </div>
        <Button variant="ghost" size="sm" onClick={() => void navigate({ to: "/" })}>
          {t("catchUp.close")}
        </Button>
      </header>

      {queue === null ? (
        isError ? (
          <div className="flex flex-1 flex-col items-center justify-center gap-3 p-6 text-center">
            <p role="alert" className="text-sm text-destructive">
              {t("catchUp.loadError")}
            </p>
            <Button variant="outline" onClick={restart}>
              {t("catchUp.retry")}
            </Button>
          </div>
        ) : (
          <div
            className="flex flex-1 flex-col gap-3 p-4"
            role="status"
            aria-label={t("catchUp.loading")}
          >
            <Skeleton className="h-8 w-48" />
            <Skeleton className="h-24 w-full" />
            <Skeleton className="h-24 w-full" />
          </div>
        )
      ) : total === 0 ? (
        <div className="flex flex-1 flex-col items-center justify-center gap-2 p-6 text-center">
          <h2 className="text-base font-medium">{t("catchUp.empty")}</h2>
          <p className="text-sm text-muted-foreground">{t("catchUp.emptyDescription")}</p>
          <Button variant="outline" onClick={restart}>
            {t("catchUp.refresh")}
          </Button>
        </div>
      ) : !current ? (
        <div className="flex flex-1 flex-col items-center justify-center gap-3 p-6 text-center">
          <h2 className="text-xl font-semibold">{t("catchUp.complete")}</h2>
          <p className="text-sm text-muted-foreground">
            {t("catchUp.summary", { read: readCount, skipped: skippedCount })}
          </p>
          <Button onClick={() => void navigate({ to: "/" })}>{t("catchUp.close")}</Button>
        </div>
      ) : (
        <>
          <div className="flex items-center justify-between gap-3 border-b px-4 py-2">
            <h2 className="min-w-0 truncate text-base font-medium">
              {labels.get(current.channel_id) ?? t("catchUp.conversation")}
            </h2>
            <p className="shrink-0 text-xs text-muted-foreground" aria-live="polite">
              {t("catchUp.progress", { current: index + 1, total })}
            </p>
          </div>
          <div className="min-h-0 flex-1">
            <CatchUpMessagePage
              key={current.channel_id}
              channelId={current.channel_id}
              firstUnreadAt={current.first_unread_at}
              onBoundaryMessage={onBoundaryMessage}
            />
          </div>
          {writeError ? (
            <p role="alert" className="border-t px-4 py-2 text-sm text-destructive">
              {t("catchUp.readError")}
            </p>
          ) : null}
          <footer className="flex gap-2 border-t p-3">
            <Button
              className="min-h-11 flex-1"
              onClick={() => void markRead()}
              disabled={isWriting}
            >
              {t("catchUp.markRead")}
            </Button>
            <Button
              className="min-h-11 flex-1"
              variant="outline"
              onClick={keepUnread}
              disabled={isWriting}
            >
              {t("catchUp.keepUnread")}
            </Button>
          </footer>
        </>
      )}
    </main>
  );
}
