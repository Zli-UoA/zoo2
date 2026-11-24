import { useLiveQuery } from "@tanstack/react-db";
import { useNavigate } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { useCurrentUser } from "../../hooks/useCurrentUser";
import {
  READ_STATE_CHANGED,
  unreadConversations,
  useUnread,
  type ChannelReadState,
} from "../../hooks/useUnread";
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
  const { states, revision, applyRead } = useUnread();
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
  const [loadError, setLoadError] = useState(false);
  const [isLoadingDetails, setIsLoadingDetails] = useState(false);
  const [rangeState, setRangeState] = useState({
    ready: false,
    exhausted: false,
    boundaryTooOld: false,
  });
  const [isWriting, setIsWriting] = useState(false);
  const [boundary, setBoundary] = useState<{
    channelId: string;
    messageId: string | null;
    at: number | null;
  } | null>(null);
  const headingRef = useRef<HTMLHeadingElement>(null);
  const requestId = useRef(0);
  const loadOlderRef = useRef<(() => void) | null>(null);
  const latestRevision = useRef(revision);
  latestRevision.current = revision;
  const revisionAtStart = useRef(revision);
  const latestStates = useRef(states);
  latestStates.current = states;
  const statesAtStart = useRef(new Map<string, ChannelReadState>());

  // 詳細（件数・最古時刻）はキャッチアップを開いたときだけ取得する。
  // 10秒ポーリングは軽い一覧のまま共有し、ここでは再計算しない。
  const start = useCallback((source: readonly ChannelReadState[]) => {
    statesAtStart.current = new Map(latestStates.current.map((state) => [state.channel_id, state]));
    setQueue(unreadConversations(source).map((state) => ({ ...state })));
    setIndex(0);
    setReadCount(0);
    setSkippedCount(0);
    setWriteError(false);
    setBoundary(null);
    setRangeState({ ready: false, exhausted: false, boundaryTooOld: false });
    revisionAtStart.current = latestRevision.current;
  }, []);

  const loadQueue = useCallback(async () => {
    const request = ++requestId.current;
    setIsLoadingDetails(true);
    setLoadError(false);
    try {
      const detailed = await client.get<ChannelReadState[]>("/api/channel-read-states?details=1");
      if (request !== requestId.current) return;
      start(detailed);
    } catch {
      if (request !== requestId.current) return;
      // Keep the queue null so the error view (not the empty view) is shown.
      setQueue(null);
      setLoadError(true);
    } finally {
      if (request === requestId.current) setIsLoadingDetails(false);
    }
  }, [client, start]);

  useEffect(() => {
    void loadQueue();
  }, [loadQueue]);

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
    setRangeState({ ready: false, exhausted: false, boundaryTooOld: false });
    setIndex((value) => value + 1);
  }, []);

  // 別タブ・端末で既読になった会話は、表示時に未読が無ければ飛ばす。
  useEffect(() => {
    if (!current || isWriting || writeError) return;
    // その会話自身の共有状態がキュー確定後に更新され、未読が消えた場合だけ
    // スキップする。別の会話の既読更新ではスキップしない。
    const original = statesAtStart.current.get(current.channel_id);
    const live = states.find((state) => state.channel_id === current.channel_id);
    if (!live || live.has_unread) return;
    if (original ? live !== original : revision <= revisionAtStart.current) goNext();
  }, [states, revision, current, isWriting, writeError, goNext]);

  useEffect(() => {
    headingRef.current?.focus();
  }, [index, queue]);

  const onRangeReady = useCallback(
    (ready: boolean, exhausted: boolean, boundaryTooOld: boolean) => {
      setRangeState({ ready, exhausted, boundaryTooOld });
    },
    [],
  );

  const onBoundaryMessage = useCallback(
    (messageId: string | null, at: number | null) =>
      setBoundary({ channelId: current?.channel_id ?? "", messageId, at }),
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
      const saved = await client.post<ChannelReadState>(
        apiPath`/api/channels/${current.channel_id}/read-state`,
        { message_id: messageId },
      );
      applyRead(saved.data);
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
    setQueue(null);
    void loadQueue();
  };

  return (
    <section
      className="flex h-full min-h-0 flex-col bg-background"
      aria-labelledby="catch-up-title"
    >
      <header className="flex items-center justify-between gap-3 border-b px-4 py-3">
        <div className="min-w-0">
          <h1
            id="catch-up-title"
            className="truncate text-lg font-semibold"
            tabIndex={-1}
            ref={headingRef}
          >
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
        loadError ? (
          <div className="flex flex-1 flex-col items-center justify-center gap-3 p-6 text-center">
            <p role="alert" className="text-sm text-destructive">
              {t("catchUp.loadError")}
            </p>
            <Button variant="outline" onClick={restart} disabled={isLoadingDetails}>
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
          <div className="flex min-h-0 flex-1 flex-col">
            <CatchUpMessagePage
              key={current.channel_id}
              channelId={current.channel_id}
              channelName={channelById.get(current.channel_id)?.channel_name}
              encrypted={channelById.get(current.channel_id)?.encrypted}
              readOnly={!!channelById.get(current.channel_id)?.archive_at}
              firstUnreadAt={current.first_unread_at ?? null}
              onBoundaryMessage={onBoundaryMessage}
              onRangeReady={onRangeReady}
              loadOlderRef={loadOlderRef}
            />
          </div>
          {rangeState.boundaryTooOld ? (
            <p role="status" className="border-t px-4 py-2 text-xs text-muted-foreground">
              {t("catchUp.unreadChanged")}
            </p>
          ) : null}
          {rangeState.exhausted && !rangeState.ready && !rangeState.boundaryTooOld ? (
            <div className="flex items-center justify-between gap-2 border-t px-4 py-2">
              <p role="status" className="text-xs text-muted-foreground">
                {t("catchUp.unreadTooOld")}
              </p>
              <Button variant="ghost" size="sm" onClick={() => loadOlderRef.current?.()}>
                {t("catchUp.loadMore")}
              </Button>
            </div>
          ) : null}
          {writeError ? (
            <p role="alert" className="border-t px-4 py-2 text-sm text-destructive">
              {t("catchUp.readError")}
            </p>
          ) : null}
          <footer className="flex gap-2 border-t p-3">
            <Button
              className="min-h-11 flex-1"
              onClick={() => void markRead()}
              disabled={
                isWriting ||
                !rangeState.ready ||
                rangeState.boundaryTooOld ||
                boundary?.channelId !== current.channel_id ||
                !boundary.messageId
              }
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
    </section>
  );
}
