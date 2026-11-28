import { toast } from "sonner";
import { createHomeTimelineCache } from "../../lib/homeTimelineCache";
import { useVirtualizer } from "@tanstack/react-virtual";
import { coalescedRefresh } from "../../lib/coalescedRefresh";
import { useUsers } from "../../hooks/useRealtime";
import { useKitchenCatalog } from "../../hooks/useKitchenCatalog";
import { ReactionEmoji } from "../chat/messages/ReactionEmoji";
import { ActivityPubBadge } from "./ActivityPubBadge";
import { HomePostContent } from "./HomePostContent";
import {
  timelineEntryId,
  mergeTimelineWindow,
  countNewTimelineEntries,
  visibleTimelineTail,
} from "../../lib/homeTimeline";
import {
  ActivityPubText,
  ActivityPubEmoji,
  FederatedAccount,
  type CustomEmoji,
} from "./ActivityPubEmoji";
import { ActivityPubProfile } from "./ActivityPubProfile";
import { HomeReactionPicker, type ReactionChoice } from "./HomeReactionPicker";
import { useHomeLive } from "../../hooks/useHomeLive";
import { Link, useNavigate, useRouter } from "@tanstack/react-router";
import {
  homePostSettingsKey,
  homeTimelineKey,
  loadHomeTimeline,
  saveHomeTimeline,
} from "../../lib/homePreferences";
import { HomeComposer, type HomeComposerHandle } from "./HomeComposer";
import { usePostEditState } from "./usePostEditState";
import { usePullToRefresh } from "./usePullToRefresh";
import { Input } from "../ui/input";
import { ExternalLink } from "../ui/external-link";
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
} from "react";
import { useTranslation } from "react-i18next";
import {
  HomeIcon,
  ArrowLeftIcon,
  RefreshCwIcon,
  UserSearchIcon,
  ArrowDownIcon,
  Trash2Icon,
  PencilIcon,
  MoreHorizontalIcon,
  ReplyIcon,
  Repeat2Icon,
  SmilePlusIcon,
  ExternalLinkIcon,
} from "lucide-react";
import { ApiError, createConfiguredApiClient } from "../../lib/api/client";
import { useCollections } from "../../lib/electric/collections-context";
import { THREADS_CHANGED } from "../../lib/electric/threads";
import { useCurrentUser } from "../../hooks/useCurrentUser";
import { ScreenHeader } from "../layout/PaneHeader";
import { ActivityPubAvatar, type RemoteAvatar } from "./ActivityPubAvatar";
import { Button } from "../ui/button";
import { Textarea } from "../ui/textarea";
import { Spinner } from "../ui/spinner";
import { ConfirmActionDialog } from "../ui/confirm-action-dialog";
import { Dialog, DialogContent, DialogTitle } from "../ui/dialog";
import {
  DropdownMenu,
  DropdownMenuTrigger,
  DropdownMenuContent,
  DropdownMenuItem,
} from "../ui/dropdown-menu";

import type { AttachmentMetadata } from "../../lib/attachments";

export interface HomePost {
  quote?: import("./HomePostContent").HomeQuote | null;
  media?: import("./HomePostContent").HomeMedia[];
  id: string;
  user_id: string;
  user_name: string;
  display_name: string;
  content: string;
  created_at: string;
  apub?: boolean;
  visibility?: string;
  channel_id?: string | null;
  channel_name?: string | null;
  remote?: boolean;
  url?: string | null;
  likes?: number;
  liked?: boolean;
  entry_id?: string;
  sort_key?: string;
  parent_id?: string | null;
  parent?: HomePost | null;
  attachments?: AttachmentMetadata[];
  tree_depth?: number;
  emojis?: CustomEmoji[];
  actor_emojis?: CustomEmoji[];
  avatar?: RemoteAvatar | null;
  avatar_url?: string | null;
  reactions?: {
    reaction: string;
    emoji: CustomEmoji | null;
    count: number;
    selected: boolean;
    can_react?: boolean;
  }[];
  replies?: number;
  reposts?: number;
  reposted?: boolean;
  reposted_by?: string | null;
  repost_created_at?: string;
}

const timelineCache = createHomeTimelineCache<HomePost>();

type HomeScreenProps = { postId?: string; actorId?: string };
export function HomeScreen(props: HomeScreenProps) {
  const { currentUserId } = useCurrentUser();
  return <HomeScreenContent key={currentUserId} {...props} currentUserId={currentUserId} />;
}
function HomeScreenContent({
  postId,
  actorId,
  currentUserId,
}: HomeScreenProps & { currentUserId: string | null }) {
  const { t, i18n } = useTranslation();
  const navigate = useNavigate();
  const router = useRouter();
  const [historyKey] = useState(
    () => router.state.location.state.__TSR_key ?? router.state.location.href,
  );
  const goBack = () => {
    if ((router.state.location.state.__TSR_index ?? 0) > 0 && router.history.canGoBack())
      router.history.back();
    else void navigate({ to: "/home", replace: true });
  };
  const { config } = useCollections();
  const usersQuery = useUsers();
  const avatarUrls = useMemo(
    () => new Map((usersQuery.data ?? []).map((user) => [user.id, user.avatar_url])),
    [usersQuery.data],
  );
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  const composerRef = useRef<HomeComposerHandle>(null);
  const [reactionPost, setReactionPost] = useState<HomePost | null>(null);
  const [pendingRepost, setPendingRepost] = useState<string | null>(null);
  const timelineStorageKey = currentUserId
    ? homeTimelineKey(config.apiBaseUrl, currentUserId)
    : null;
  const [timeline, setTimeline] = useState(() => loadHomeTimeline(timelineStorageKey));
  const timelineQuery = actorId ? `social&author=${encodeURIComponent(actorId)}` : timeline;
  const [federation, setFederation] = useState<{
    enabled: boolean;
  }>({ enabled: false });
  const [follows, setFollows] = useState<
    { actor_id: string; display_name: string; accepted: boolean; direction: string }[]
  >([]);
  const [searchOpen, setSearchOpen] = useState(false);
  const [searchResults, setSearchResults] = useState<
    | {
        id: string;
        user_name: string;
        display_name: string;
        remote: boolean;
        url?: string;
        created_at?: string;
        emojis?: CustomEmoji[];
        avatar?: RemoteAvatar | null;
        avatar_url?: string | null;
      }[]
    | null
  >(null);
  const [searchPending, setSearchPending] = useState(false);
  const [searchError, setSearchError] = useState(false);
  const searchVersion = useRef(0);
  const [account, setAccount] = useState("");
  const [controlsPending, setControlsPending] = useState(false);
  const [controlsError, setControlsError] = useState(false);
  const { pull, pulling, onTouchStart, onTouchMove, onTouchCancel, onTouchEnd } = usePullToRefresh(
    () => refresh(),
    () => scrollRoot.current?.scrollTop ?? 0,
  );
  const {
    editingId,
    editText,
    setEditText,
    editError,
    setEditError,
    editPending,
    setEditPending,
    startEdit,
    cancelEdit,
  } = usePostEditState();
  const loadControls = useCallback(async () => {
    const results = await Promise.allSettled([
      client.get<typeof federation>("/api/activitypub/status"),
      client.get<typeof follows>("/api/activitypub/follows"),
    ]);
    if (results[0].status === "fulfilled") setFederation(results[0].value);
    if (results[1].status === "fulfilled") setFollows(results[1].value);
    setControlsError(results.some((r) => r.status === "rejected"));
  }, [client]);
  const scheduleControls = useRef<() => void>(() => {});
  useEffect(() => {
    const scheduler = coalescedRefresh(
      () => (document.visibilityState === "hidden" ? undefined : loadControls()),
      60_000,
    );
    scheduleControls.current = scheduler.request;
    scheduler.request();
    return () => {
      scheduleControls.current = () => {};
      scheduler.dispose();
    };
  }, [loadControls]);
  const controlAction = async (action: () => Promise<unknown>) => {
    if (controlsPending) return;
    setControlsPending(true);
    setControlsError(false);
    try {
      await action();
      await loadControls();
    } catch {
      setControlsError(true);
    } finally {
      setControlsPending(false);
    }
  };
  const cacheScope = JSON.stringify([config.apiBaseUrl, currentUserId]) + ":";
  const cacheKey = cacheScope + JSON.stringify([historyKey, timelineQuery]);
  const detailCacheKey = (id: string) => cacheScope + JSON.stringify(["post", id]);
  const preparePost = (post: HomePost) => {
    // Hand off the DTO in account-scoped tab memory, without persisting it in browser history.
    timelineCache.save(detailCacheKey(post.id), {
      posts: [{ ...post, entry_id: post.id, tree_depth: 0, reposted_by: null }],
      hasMore: false,
      scrollOffset: 0,
      listOffset: 0,
      measurements: [],
    });
  };
  const initialDetailKey = postId ? detailCacheKey(postId) : null;
  const restored = useMemo(
    () => timelineCache.get(initialDetailKey ?? cacheKey),
    [cacheKey, initialDetailKey],
  );
  const [posts, setPosts] = useState<HomePost[]>(() => restored?.posts ?? []);
  const [newPosts, setNewPosts] = useState(0);
  const { catalog } = useKitchenCatalog(
    posts.some((post) => post.reactions?.some((r) => r.reaction.startsWith("kitchen:"))),
  );
  const [loading, setLoading] = useState(!restored);
  const [loadingOlder, setLoadingOlder] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [hasMore, setHasMore] = useState(restored?.hasMore ?? false);
  const [error, setError] = useState(false);
  const [viewportError, setViewportError] = useState(false);
  const [olderError, setOlderError] = useState(restored?.olderError ?? false);
  const [pendingLike, setPendingLike] = useState<string | null>(null);
  const [reactionError, setReactionError] = useState<string | null>(null);
  const [deleteId, setDeleteId] = useState<string | null>(null);
  const [deleting, setDeleting] = useState(false);
  const [deleteError, setDeleteError] = useState(false);
  const scrollRoot = useRef<HTMLDivElement>(null);
  const generation = useRef(0);
  const refreshRequest = useRef<AbortController | null>(null);
  const olderRequest = useRef<AbortController | null>(null);

  const deletedPosts = useRef(new Set<string>());
  const olderErrorRef = useRef(olderError);
  olderErrorRef.current = olderError;
  const headRequested = useRef(true);
  const hasMoreRef = useRef(hasMore);
  hasMoreRef.current = hasMore;
  const postsRef = useRef(posts);
  postsRef.current = posts;
  const visibleIds = useRef<string[]>([]);
  const lastHead = useRef<HomePost[]>([]);
  const deferredRefresh = useRef(false);
  const scheduleRefresh = useRef<(includeHead?: boolean) => void>(() => {});
  const refresh = useCallback(
    async (retain = false, includeHead = true) => {
      if (retain && (olderRequest.current || refreshRequest.current)) {
        deferredRefresh.current = true;
        headRequested.current ||= includeHead;
        return;
      }
      deferredRefresh.current = false;
      if (!retain) {
        if (scrollRoot.current) scrollRoot.current.scrollTop = 0;
        setNewPosts(0);
        setOlderError(false);
      }
      const version = ++generation.current;
      refreshRequest.current?.abort();
      olderRequest.current?.abort();
      olderRequest.current = null;
      setLoadingOlder(false);
      const controller = new AbortController();
      refreshRequest.current = controller;
      setRefreshing(true);
      const current = postsRef.current;
      const checkedIds = retain && !postId ? visibleIds.current.slice(0, 100) : [];
      try {
        const fetchedHead =
          includeHead || !current.length || postId
            ? await client.get<HomePost[]>(
                postId
                  ? `/api/home/${encodeURIComponent(postId)}/thread`
                  : `/api/home?timeline=${timelineQuery}`,
                { signal: controller.signal },
              )
            : null;
        const head = (fetchedHead ?? []).filter((post) => !deletedPosts.current.has(post.id));
        const complete = !!fetchedHead && !postId && fetchedHead.length < 50;
        const headIds = new Set(head.map(timelineEntryId));
        const missingIds = complete ? [] : checkedIds.filter((id) => !headIds.has(id));
        const query = new URLSearchParams(`timeline=${timelineQuery}`);
        let checked: HomePost[] = [];
        let checkedSuccessfully = true;
        try {
          if (missingIds.length)
            checked = await client.request<HomePost[]>("/api/home/entries", {
              method: "POST",
              body: {
                entries: missingIds,
                timeline: query.get("timeline"),
                author: query.get("author"),
              },
              signal: controller.signal,
            });
        } catch {
          if (controller.signal.aborted || version !== generation.current) return;
          checkedSuccessfully = false;
        }
        if (version !== generation.current) return;
        const atTop = (scrollRoot.current?.scrollTop ?? 0) < 80;
        if (fetchedHead && retain && !postId && current.length && !atTop) {
          const added = countNewTimelineEntries(
            head,
            lastHead.current.length ? lastHead.current : current,
          );
          if (added) setNewPosts((value) => value + added);
        }
        if (fetchedHead) {
          lastHead.current = head;
          if (atTop) setNewPosts(0);
        }
        const rows =
          retain && !postId
            ? mergeTimelineWindow(
                current,
                head,
                complete ? current.map(timelineEntryId) : checkedSuccessfully ? checkedIds : [],
                checked,
                complete || (!!fetchedHead && atTop),
              )
            : head;
        const visibleRows = rows.filter((post) => !deletedPosts.current.has(post.id));
        postsRef.current = visibleRows;
        setPosts(visibleRows);
        if (
          fetchedHead &&
          (!retain ||
            complete ||
            !current.length ||
            postId ||
            (atTop &&
              !head.some((p) =>
                current.some((old) => timelineEntryId(old) === timelineEntryId(p)),
              )))
        ) {
          setHasMore(!postId && head.length === 50);
        }
        if (fetchedHead) setError(false);
        setViewportError(!checkedSuccessfully);
      } catch (cause) {
        if (!controller.signal.aborted) {
          if (postId && cause instanceof ApiError && [403, 404].includes(cause.status)) {
            timelineCache.forget(cacheScope + JSON.stringify(["post", postId]));
            postsRef.current = [];
            setPosts([]);
          }
          setError(true);
        }
      } finally {
        if (version === generation.current) setLoading(false);
        if (refreshRequest.current === controller) {
          refreshRequest.current = null;
          setRefreshing(false);
          if (deferredRefresh.current && document.visibilityState !== "hidden")
            scheduleRefresh.current(false);
        }
      }
    },
    [client, timelineQuery, postId, cacheScope],
  );

  useEffect(() => {
    const snapshot = restored;
    setLoading(!snapshot);
    setPosts(snapshot?.posts ?? []);
    postsRef.current = snapshot?.posts ?? [];
    visibleIds.current = [];
    lastHead.current = [];
    setHasMore(snapshot?.hasMore ?? false);
    setOlderError(snapshot?.olderError ?? false);
    setError(false);
    setViewportError(false);
    setNewPosts(0);
    headRequested.current = true;
    if (scrollRoot.current) scrollRoot.current.scrollTop = snapshot?.scrollOffset ?? 0;
    // Start the initial load through the same gate as live/viewport refreshes.
    const scheduler = coalescedRefresh(() => {
      if (document.visibilityState === "hidden") return;
      const includeHead = headRequested.current;
      headRequested.current = false;
      return refresh(true, includeHead);
    });
    scheduleRefresh.current = (includeHead = true) => {
      headRequested.current ||= includeHead;
      scheduler.request();
    };
    scheduler.request();
    return () => {
      scheduler.dispose();
      scheduleRefresh.current = () => {};
      generation.current++;
      deferredRefresh.current = false;
      refreshRequest.current?.abort();
      olderRequest.current?.abort();
      refreshRequest.current = null;
      olderRequest.current = null;
    };
  }, [refresh, cacheKey, postId, restored]);
  useHomeLive(() => {
    scheduleRefresh.current();
    scheduleControls.current();
  });

  const older = useCallback(async () => {
    const current = postsRef.current;
    if (olderRequest.current || refreshRequest.current || !current.length || postId) return;
    setLoadingOlder(true);
    const version = generation.current;
    const controller = new AbortController();
    olderRequest.current = controller;
    try {
      const rows = await client.get<HomePost[]>(
        `/api/home?timeline=${timelineQuery}&before=${encodeURIComponent(timelineEntryId(current[current.length - 1]))}`,
        { signal: controller.signal },
      );
      if (version !== generation.current) return;
      const ids = new Set(current.map(timelineEntryId));
      const next = [
        ...current,
        ...rows.filter(
          (row) => !ids.has(timelineEntryId(row)) && !deletedPosts.current.has(row.id),
        ),
      ];
      postsRef.current = next;
      setPosts(next);
      setHasMore(rows.length === 50 && next.length > current.length);
      setOlderError(false);
    } catch {
      if (!controller.signal.aborted) setOlderError(true);
    } finally {
      if (olderRequest.current === controller) {
        olderRequest.current = null;
        setLoadingOlder(false);
        if (deferredRefresh.current && document.visibilityState !== "hidden")
          scheduleRefresh.current(false);
      }
    }
  }, [client, timelineQuery, postId]);

  const listRef = useRef<HTMLOListElement>(null);
  const [listOffset, setListOffset] = useState(restored?.listOffset ?? 0);
  useLayoutEffect(() => {
    const list = listRef.current;
    const root = scrollRoot.current;
    if (!list || !root) return;
    const measure = () =>
      setListOffset(
        list.getBoundingClientRect().top - root.getBoundingClientRect().top + root.scrollTop,
      );
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(root);
    // Profile, timeline controls and pull-to-refresh can change the list's offset.
    if (list.parentElement) observer.observe(list.parentElement);
    return () => observer.disconnect();
  }, [loading, actorId]);
  const virtualizer = useVirtualizer({
    count: posts.length,
    initialOffset: restored?.scrollOffset ?? 0,
    initialMeasurementsCache: restored?.measurements,
    getScrollElement: () => scrollRoot.current,
    estimateSize: () => 240,
    getItemKey: (index) => timelineEntryId(posts[index]),
    overscan: 6,
    scrollMargin: listOffset,
    // Keep the reading key across prepends/deletions, but never pin resize
    // measurements to the bottom as newly appended rows replace estimates.
    anchorTo: (scrollRoot.current?.scrollTop ?? 0) > 80 ? "end" : "start",
    followOnAppend: false,
    scrollEndThreshold: Number.NEGATIVE_INFINITY,
    useAnimationFrameWithResizeObserver: true,
  });
  const virtualizerRef = useRef(virtualizer);
  virtualizerRef.current = virtualizer;
  useLayoutEffect(() => {
    const root = scrollRoot.current;
    if (!root || postId) return;
    return () => {
      const measured = virtualizerRef.current;
      timelineCache.save(cacheKey, {
        posts: postsRef.current,
        hasMore: hasMoreRef.current,
        olderError: olderErrorRef.current,
        scrollOffset: root.scrollTop,
        listOffset: measured.options.scrollMargin ?? 0,
        measurements: measured.takeSnapshot(),
        anchor: (() => {
          const top = root.getBoundingClientRect().top;
          const row = Array.from(root.querySelectorAll<HTMLElement>("[data-home-post]")).find(
            (node) => node.getBoundingClientRect().bottom > top,
          );
          return row
            ? { id: row.dataset.homePost!, offset: row.getBoundingClientRect().top - top }
            : undefined;
        })(),
      });
    };
  }, [cacheKey, postId]);
  const restoreAnchor = useRef(restored?.anchor);
  const virtualRows = virtualizer.getVirtualItems();
  useLayoutEffect(() => {
    const root = scrollRoot.current,
      anchor = restoreAnchor.current;
    if (!root || !anchor) return;
    const row = Array.from(root.querySelectorAll<HTMLElement>("[data-home-post]")).find(
      (node) => node.dataset.homePost === anchor.id,
    );
    if (!row) return;
    root.scrollTop +=
      row.getBoundingClientRect().top - root.getBoundingClientRect().top - anchor.offset;
    restoreAnchor.current = undefined;
  }, [posts, listOffset, virtualRows]);
  const viewportKey = virtualRows.map((row) => timelineEntryId(posts[row.index])).join("\n");
  useEffect(() => {
    visibleIds.current = viewportKey ? viewportKey.split("\n") : [];
    // Revalidate history as it enters the viewport, so edits/deletions are not lost.
    // This uses the same five-second gate as live updates, with no polling.
    const headIds = new Set(lastHead.current.map(timelineEntryId));
    if (viewportKey && !loading && visibleIds.current.some((id) => !headIds.has(id)))
      scheduleRefresh.current(false);
  }, [viewportKey, loading]);
  useEffect(() => {
    const root = scrollRoot.current;
    // Timeline changes and anchor restoration can reset the DOM scroll position
    // before the virtualizer observes it. Read the current viewport here, after
    // layout effects, rather than fetching from a previous timeline's range.
    const lastVisibleIndex = root
      ? visibleTimelineTail(virtualRows, root.scrollTop, root.clientHeight)
      : -1;
    if (
      !loading &&
      !refreshing &&
      !loadingOlder &&
      !error &&
      !olderError &&
      hasMore &&
      lastVisibleIndex >= 0 &&
      lastVisibleIndex >= posts.length - 10 &&
      document.visibilityState === "visible"
    ) {
      void older();
    }
  }, [
    virtualRows,
    posts.length,
    hasMore,
    loading,
    refreshing,
    loadingOlder,
    error,
    olderError,
    older,
  ]);
  const react = (post: HomePost, choice: ReactionChoice, remove = false) => {
    if (pendingLike) return;
    setPendingLike(post.id);
    setReactionError(null);
    void client
      .request("/api/activitypub/reactions", {
        method: remove ? "DELETE" : "POST",
        body: { post_id: post.id, ...choice },
      })
      .then(() => refresh(true))
      .catch(() => setReactionError(post.id))
      .finally(() => setPendingLike(null));
  };
  return (
    <section className="relative flex min-h-0 flex-1 flex-col" aria-labelledby="home-title">
      <ScreenHeader
        icon={HomeIcon}
        leading={
          postId || actorId ? (
            <Button variant="ghost" size="icon" aria-label={t("home.back")} onClick={goBack}>
              <ArrowLeftIcon />
            </Button>
          ) : undefined
        }
        titleId="home-title"
        title={t(postId ? "activitypubSettings.post" : actorId ? "home.userDetails" : "home.title")}
        actions={
          <>
            <Button
              variant="ghost"
              size="icon"
              aria-label={t("home.userSearch")}
              onClick={() => setSearchOpen(true)}
            >
              <UserSearchIcon />
            </Button>
            <Button
              variant="ghost"
              size="icon"
              aria-label={t("home.refresh")}
              onClick={() => void refresh()}
            >
              <RefreshCwIcon />
            </Button>
          </>
        }
      />
      {newPosts > 0 && !postId ? (
        <div className="absolute inset-x-0 top-20 z-10 flex justify-center pointer-events-none">
          <span className="pointer-events-auto inline-flex rounded-full shadow-md">
            <Button
              shape="circle"
              onClick={() => {
                virtualizer.scrollToOffset(0);
                setNewPosts(0);
                void refresh(true);
              }}
            >
              <ArrowDownIcon className="rotate-180" />
              {t("home.newPosts", { count: newPosts })}
            </Button>
          </span>
        </div>
      ) : null}
      <div
        ref={scrollRoot}
        data-home-scroll
        onScroll={() => {
          if ((scrollRoot.current?.scrollTop ?? 0) < 80 && newPosts > 0)
            scheduleRefresh.current(true);
        }}
        className="min-h-0 flex-1 overflow-y-auto overscroll-y-contain pb-24 [overflow-anchor:none]"
        onTouchStart={onTouchStart}
        onTouchMove={onTouchMove}
        onTouchCancel={onTouchCancel}
        onTouchEnd={onTouchEnd}
      >
        <div className="mx-auto w-full max-w-2xl">
          {actorId ? <ActivityPubProfile actorId={actorId} avatarUrls={avatarUrls} /> : null}
          {!postId && !actorId ? (
            <div className="flex gap-2 border-b p-3" aria-label={t("home.timeline")}>
              {(["home", "local", "social"] as const).map((value) => (
                <Button
                  key={value}
                  variant={timeline === value ? "secondary" : "ghost"}
                  aria-pressed={timeline === value}
                  onClick={() => {
                    if (value === timeline) {
                      void refresh();
                      return;
                    }
                    timelineCache.forget(cacheScope + JSON.stringify([historyKey, value]));
                    restoreAnchor.current = undefined;
                    virtualizer.scrollToOffset(0);
                    setTimeline(value);
                    saveHomeTimeline(timelineStorageKey, value);
                  }}
                >
                  {t(`home.timelineLabels.${value}`)}
                </Button>
              ))}
            </div>
          ) : null}
          {pull > 0 || pulling ? (
            <div
              role="status"
              className="flex h-(--pull-height) items-center justify-center gap-2 text-xs text-muted-foreground md:hidden"
              style={{ "--pull-height": `${pulling ? 60 : pull}px` } as CSSProperties}
            >
              {pulling ? (
                <RefreshCwIcon className="size-4 animate-spin" />
              ) : (
                <ArrowDownIcon className="size-4" />
              )}
              {t(
                pulling ? "home.loading" : pull >= 60 ? "home.releaseRefresh" : "home.pullRefresh",
              )}
            </div>
          ) : null}
          {error || viewportError ? (
            <p role="alert" className="p-4 text-sm text-destructive">
              {t("home.loadError")}{" "}
              <Button variant="link" onClick={() => void refresh(!error, error)}>
                {t("home.refresh")}
              </Button>
            </p>
          ) : null}
          {loading ? (
            <div className="flex justify-center p-8">
              <Spinner label={t("home.loading")} />
            </div>
          ) : posts.length === 0 && !error ? (
            <div className="space-y-2 px-4 py-14 text-center">
              <HomeIcon className="mx-auto size-8 text-muted-foreground" aria-hidden="true" />
              <h2 className="font-medium">{t("home.empty")}</h2>
              <p className="text-sm text-muted-foreground">{t("home.emptyHint")}</p>
            </div>
          ) : (
            <ol
              ref={listRef}
              aria-label={t("home.timeline")}
              className="relative h-(--timeline-height) [overflow-anchor:var(--timeline-overflow-anchor)]"
              style={
                {
                  "--timeline-height": `${virtualizer.getTotalSize()}px`,
                  "--timeline-overflow-anchor": "none",
                } as CSSProperties
              }
            >
              {virtualRows.map((virtualRow) => {
                const post = posts[virtualRow.index];
                return (
                  <li
                    key={post.entry_id ?? post.id}
                    data-home-post={post.entry_id ?? post.id}
                    data-index={virtualRow.index}
                    ref={virtualizer.measureElement}
                    aria-posinset={virtualRow.index + 1}
                    aria-setsize={hasMore ? -1 : posts.length}
                    className="absolute top-0 left-0 w-full border-b pe-4 pt-4 pb-2 ps-(--row-indent) translate-y-(--row-offset)"
                    style={
                      {
                        "--row-offset": `${virtualRow.start - listOffset}px`,
                        "--row-indent": postId
                          ? `${16 + Math.min(post.tree_depth ?? 0, 6) * 16}px`
                          : "1rem",
                      } as CSSProperties
                    }
                    aria-current={postId === post.id ? "true" : undefined}
                  >
                    {!postId && post.parent_id ? (
                      <div className="mb-3 rounded border-s-2 ps-3 text-sm text-muted-foreground">
                        <Link
                          to="/home/posts/$postId"
                          params={{ postId: post.parent_id }}
                          onClick={() => {
                            if (post.parent) preparePost(post.parent);
                          }}
                          className="flex items-center gap-1 hover:underline"
                        >
                          <ReplyIcon className="size-3" />
                          {t("home.replyTo", {
                            name:
                              post.parent?.display_name ||
                              post.parent?.user_name ||
                              t("activitypubSettings.post"),
                          })}
                        </Link>
                        {post.parent ? (
                          <HomePostContent
                            content={post.parent.content}
                            emojis={post.parent.emojis}
                            attachments={post.parent.attachments}
                            media={post.parent.media}
                            quote={post.parent.quote}
                          />
                        ) : null}
                      </div>
                    ) : null}
                    {post.reposted_by ? (
                      <p className="mb-2 flex items-center gap-2 text-xs text-muted-foreground">
                        <Repeat2Icon className="size-3" />
                        {t("home.repostedBy", { name: post.reposted_by })}
                      </p>
                    ) : null}
                    <article
                      className="flex min-w-0 cursor-pointer gap-3"
                      onClick={(event) => {
                        if (editingId === (post.entry_id ?? post.id) || post.id === postId) return;
                        const target = event.target;
                        // Dialog portals bubble clicks through React even outside the article.
                        if (
                          !(target instanceof Element) ||
                          !event.currentTarget.contains(target) ||
                          target.closest("a,button,input,textarea,[role=button],pre") ||
                          window.getSelection()?.toString()
                        )
                          return;
                        preparePost(post);
                        void navigate({ to: "/home/posts/$postId", params: { postId: post.id } });
                      }}
                    >
                      <Link
                        to="/home/users/$actorId"
                        params={{ actorId: post.user_id }}
                        className="shrink-0 rounded-full focus-visible:ring-2 focus-visible:ring-ring"
                      >
                        <ActivityPubAvatar
                          avatar={post.remote ? post.avatar : null}
                          avatarUrl={
                            post.remote
                              ? undefined
                              : avatarUrls.get(post.user_id) !== undefined
                                ? avatarUrls.get(post.user_id)
                                : post.avatar_url
                          }
                          name={post.display_name || post.user_name}
                        />
                      </Link>
                      <div className="min-w-0 flex-1">
                        <div className="flex flex-wrap items-baseline gap-x-2 gap-y-1 text-xs text-muted-foreground">
                          <Link
                            to="/home/users/$actorId"
                            params={{ actorId: post.user_id }}
                            className="break-all text-sm font-semibold text-foreground hover:underline"
                          >
                            <ActivityPubText
                              text={post.display_name || post.user_name}
                              emojis={post.actor_emojis}
                            />
                          </Link>
                          <Link
                            to="/home/users/$actorId"
                            params={{ actorId: post.user_id }}
                            className="hover:underline"
                          >
                            <FederatedAccount
                              userName={post.user_name}
                              actorId={post.user_id}
                              remote={post.remote}
                            />
                          </Link>
                          <span className="inline-flex min-w-0 max-w-full items-center gap-2">
                            <Link
                              to="/home/posts/$postId"
                              params={{ postId: post.id }}
                              onClick={() => preparePost(post)}
                              className="shrink-0 hover:underline"
                            >
                              <time dateTime={post.created_at}>
                                {new Date(post.created_at).toLocaleString(i18n.language)}
                              </time>
                            </Link>
                            <ActivityPubBadge
                              visibility={post.visibility ?? "local"}
                              federated={post.apub ?? post.remote ?? false}
                            />
                            {post.channel_id && !post.remote ? (
                              <Link
                                className="min-w-0 truncate text-link"
                                title={`#${post.channel_name ?? post.channel_id}`}
                                to="/channels/$channelId"
                                params={{ channelId: post.channel_id }}
                                search={{ message: post.id }}
                              >
                                #{post.channel_name ?? post.channel_id}
                              </Link>
                            ) : null}
                          </span>
                        </div>
                        {editingId === (post.entry_id ?? post.id) ? (
                          <form
                            className="mt-2 space-y-2"
                            onSubmit={(e) => {
                              e.preventDefault();
                              if (editPending) return;
                              setEditPending(true);
                              setEditError(false);
                              void client
                                .request(`/api/home/${encodeURIComponent(post.id)}`, {
                                  method: "PATCH",
                                  body: { content: editText },
                                })
                                .then(async () => {
                                  window.dispatchEvent(new Event(THREADS_CHANGED));
                                  cancelEdit();
                                  await refresh();
                                })
                                .catch(() => setEditError(true))
                                .finally(() => setEditPending(false));
                            }}
                          >
                            <Textarea
                              aria-label={t("home.edit")}
                              value={editText}
                              disabled={editPending}
                              onChange={(e) => setEditText(e.target.value)}
                            />
                            <Button
                              type="submit"
                              disabled={
                                editPending ||
                                !editText.trim() ||
                                Array.from(editText).length > 40000
                              }
                            >
                              {t("home.save")}
                            </Button>
                            <Button variant="ghost" disabled={editPending} onClick={cancelEdit}>
                              {t("common.cancel")}
                            </Button>
                            {editError ? (
                              <p role="alert" className="text-xs text-destructive">
                                {t("home.controlsError")}
                              </p>
                            ) : null}
                          </form>
                        ) : (
                          <HomePostContent
                            quote={post.quote}
                            content={post.content}
                            emojis={post.emojis}
                            attachments={post.attachments}
                            media={post.media}
                          />
                        )}
                        {post.reactions?.length ? (
                          <div className="mt-2 flex flex-wrap gap-1">
                            {post.reactions.map((r) => (
                              <Button
                                key={`${r.reaction}:${r.emoji?.image_id ?? ""}`}
                                variant={r.selected ? "secondary" : "outline"}
                                size="sm"
                                aria-label={`${r.reaction} ${r.count}`}
                                aria-pressed={r.selected}
                                disabled={
                                  pendingLike !== null || (r.can_react === false && !r.selected)
                                }
                                onClick={() =>
                                  react(
                                    post,
                                    {
                                      reaction: r.reaction,
                                      ...(r.emoji ? { emoji_id: r.emoji.image_id } : {}),
                                    },
                                    r.selected,
                                  )
                                }
                              >
                                {r.emoji ? (
                                  <ActivityPubEmoji emoji={r.emoji} />
                                ) : (
                                  <ReactionEmoji emoji={r.reaction} catalog={catalog} />
                                )}
                                <span>{r.count}</span>
                              </Button>
                            ))}
                          </div>
                        ) : null}
                        <div
                          role="group"
                          aria-label={t("home.actions")}
                          className="mt-1 flex items-center justify-between gap-2 text-muted-foreground"
                        >
                          <Button
                            variant="ghost"
                            size="sm"
                            aria-label={`${t("home.reply")} ${post.replies ?? 0}`}
                            onClick={() => composerRef.current?.reply(post)}
                          >
                            <ReplyIcon />
                            <span>{post.replies ?? 0}</span>
                          </Button>
                          <Button
                            variant="ghost"
                            size="sm"
                            aria-label={`${t(post.reposted ? "home.undoRepost" : "home.repost")} ${post.reposts ?? 0}`}
                            aria-pressed={post.reposted ?? false}
                            disabled={pendingRepost !== null || post.visibility === "followers"}
                            title={
                              post.visibility === "followers" ? t("home.privateRepost") : undefined
                            }
                            onClick={() => {
                              if (pendingRepost) return;
                              setPendingRepost(post.id);
                              setReactionError(null);
                              void client
                                .request("/api/activitypub/reposts", {
                                  method: post.reposted ? "DELETE" : "POST",
                                  body: { post_id: post.id },
                                })
                                .then(() => refresh(true))
                                .catch(() => setReactionError(post.id))
                                .finally(() => setPendingRepost(null));
                            }}
                          >
                            <Repeat2Icon />
                            <span>{post.reposts ?? 0}</span>
                          </Button>
                          <Button
                            variant="ghost"
                            size="sm"
                            aria-label={`${t("home.react")} ${post.likes ?? 0}`}
                            onClick={() => setReactionPost(post)}
                          >
                            <SmilePlusIcon />
                            <span>{post.likes ?? 0}</span>
                          </Button>
                          <DropdownMenu>
                            <DropdownMenuTrigger
                              render={
                                <Button
                                  variant="ghost"
                                  size="icon"
                                  aria-label={t("home.postActions")}
                                />
                              }
                            >
                              <MoreHorizontalIcon />
                            </DropdownMenuTrigger>
                            <DropdownMenuContent
                              align="end"
                              onClick={(event) => event.stopPropagation()}
                            >
                              {post.user_id === currentUserId && !post.remote ? (
                                <>
                                  <DropdownMenuItem
                                    onClick={() =>
                                      startEdit(post.entry_id ?? post.id, post.content)
                                    }
                                  >
                                    <PencilIcon />
                                    {t("home.edit")}
                                  </DropdownMenuItem>
                                  <DropdownMenuItem
                                    variant="destructive"
                                    onClick={() => {
                                      setDeleteError(false);
                                      setDeleteId(post.id);
                                    }}
                                  >
                                    <Trash2Icon />
                                    {t("home.delete")}
                                  </DropdownMenuItem>
                                </>
                              ) : null}
                              {post.remote && post.url?.startsWith("https://") ? (
                                <DropdownMenuItem
                                  render={
                                    <ExternalLink
                                      href={post.url}
                                      className="text-popover-foreground"
                                    />
                                  }
                                >
                                  <ExternalLinkIcon />
                                  {t("home.original")}
                                </DropdownMenuItem>
                              ) : null}
                              <DropdownMenuItem
                                onClick={() => {
                                  if (!navigator.clipboard) {
                                    toast.error(t("messages.copyFailed"));
                                    return;
                                  }
                                  void navigator.clipboard
                                    .writeText(post.content)
                                    .then(() => toast.success(t("messages.copied")))
                                    .catch(() => toast.error(t("messages.copyFailed")));
                                }}
                              >
                                {t("home.copyContent")}
                              </DropdownMenuItem>
                            </DropdownMenuContent>
                          </DropdownMenu>
                        </div>
                        {reactionError === post.id ? (
                          <p role="alert" className="text-xs text-destructive">
                            {t("home.controlsError")}
                          </p>
                        ) : null}
                      </div>
                    </article>
                  </li>
                );
              })}
            </ol>
          )}
          {hasMore ? (
            <div className="flex flex-col items-center gap-2 p-4">
              {olderError ? (
                <p role="alert" className="text-sm text-destructive">
                  {t("home.loadError")}
                </p>
              ) : null}
              <Button
                variant="outline"
                disabled={loadingOlder || refreshing}
                onClick={() => void older()}
              >
                {t(loadingOlder ? "home.loading" : "home.older")}
              </Button>
            </div>
          ) : null}
        </div>
      </div>
      <Dialog
        open={searchOpen}
        onOpenChange={(open) => {
          setSearchOpen(open);
          if (!open) {
            searchVersion.current++;
            setSearchPending(false);
          }
        }}
      >
        <DialogContent>
          <DialogTitle>{t("home.userSearch")}</DialogTitle>
          <form
            className="mt-4 flex gap-2"
            onSubmit={(event) => {
              event.preventDefault();
              const version = ++searchVersion.current;
              setSearchPending(true);
              setSearchError(false);
              setSearchResults(null);
              void client
                .get<NonNullable<typeof searchResults>>(
                  `/api/activitypub/search?q=${encodeURIComponent(account.trim())}`,
                )
                .then((users) => {
                  if (version === searchVersion.current) setSearchResults(users);
                })
                .catch(() => {
                  if (version === searchVersion.current) setSearchError(true);
                })
                .finally(() => {
                  if (version === searchVersion.current) setSearchPending(false);
                });
            }}
          >
            <Input
              aria-label={t("home.userSearch")}
              placeholder="alice / @user@example.social"
              value={account}
              onChange={(e) => setAccount(e.target.value)}
            />
            <Button type="submit" disabled={searchPending || !account.trim()}>
              {t("home.search")}
            </Button>
          </form>
          {searchPending ? <Spinner label={t("home.loading")} /> : null}
          {searchError || controlsError ? (
            <p role="alert" className="text-sm text-destructive">
              {t("home.controlsError")}
            </p>
          ) : null}
          <div className="max-h-[60dvh] space-y-3 overflow-y-auto">
            {searchResults?.length === 0 ? <p className="text-sm">{t("home.noUsers")}</p> : null}
            {searchResults?.map((user) => {
              const follow = follows.find(
                (f) => f.direction === "outgoing" && f.actor_id === user.id,
              );
              return (
                <article
                  key={user.id}
                  aria-label={t("home.userDetails")}
                  className="space-y-2 rounded-lg border p-4"
                >
                  <div className="flex items-center gap-3">
                    <Link
                      to="/home/users/$actorId"
                      params={{ actorId: user.id }}
                      onClick={() => setSearchOpen(false)}
                    >
                      <ActivityPubAvatar
                        avatar={user.remote ? user.avatar : null}
                        avatarUrl={
                          user.remote
                            ? undefined
                            : avatarUrls.get(user.id) !== undefined
                              ? avatarUrls.get(user.id)
                              : user.avatar_url
                        }
                        name={user.display_name || user.user_name}
                      />
                    </Link>
                    <div className="min-w-0">
                      <h3 className="break-all font-semibold">
                        <Link
                          to="/home/users/$actorId"
                          params={{ actorId: user.id }}
                          onClick={() => setSearchOpen(false)}
                          className="hover:underline"
                        >
                          <ActivityPubText
                            text={user.display_name || user.user_name}
                            emojis={user.emojis}
                          />
                        </Link>
                      </h3>
                      <p className="break-all text-sm text-muted-foreground">
                        <FederatedAccount
                          userName={user.user_name}
                          actorId={user.id}
                          remote={user.remote}
                        />
                      </p>
                    </div>
                  </div>
                  {user.created_at ? (
                    <p className="text-xs text-muted-foreground">
                      {t("home.joinedAt", {
                        date: new Date(user.created_at).toLocaleDateString(i18n.language),
                      })}
                    </p>
                  ) : null}
                  {user.remote && user.url?.startsWith("https://") ? (
                    <ExternalLink className="break-all text-xs" href={user.url}>
                      {user.url}
                    </ExternalLink>
                  ) : null}
                  {user.remote ? (
                    <Button
                      size="sm"
                      disabled={controlsPending || !federation.enabled}
                      onClick={() =>
                        void controlAction(() =>
                          client.request("/api/activitypub/follows", {
                            method: follow ? "DELETE" : "POST",
                            body: { actor: user.id },
                          }),
                        )
                      }
                    >
                      {t(follow ? "home.unfollow" : "home.follow")}
                    </Button>
                  ) : null}
                  {follow ? (
                    <p className="text-xs">
                      {t(follow.accepted ? "home.following" : "home.followPending")}
                    </p>
                  ) : null}
                </article>
              );
            })}
          </div>
        </DialogContent>
      </Dialog>
      {reactionPost ? (
        <HomeReactionPicker
          key={reactionPost.id}
          postId={reactionPost.id}
          remote={reactionPost.remote}
          onClose={() => setReactionPost(null)}
          onSelect={(choice) => react(reactionPost, choice)}
        />
      ) : null}
      <HomeComposer
        key={currentUserId ? homePostSettingsKey(config.apiBaseUrl, currentUserId) : "loading"}
        settingsKey={currentUserId ? homePostSettingsKey(config.apiBaseUrl, currentUserId) : null}
        ref={composerRef}
        federationEnabled={federation.enabled}
        onPost={async (post) => {
          await client.request("/api/home", { method: "POST", body: post });
          if (post.in_reply_to) window.dispatchEvent(new Event(THREADS_CHANGED));
          void refresh();
        }}
      />
      <ConfirmActionDialog
        open={deleteId !== null}
        onOpenChange={(open) => {
          if (!open) setDeleteId(null);
        }}
        title={t("home.delete")}
        description={t("home.deleteDescription")}
        confirmLabel={t("home.delete")}
        pending={deleting}
        error={deleteError ? t("home.deleteError") : undefined}
        onConfirm={async () => {
          if (!deleteId || deleting) return;
          setDeleting(true);
          setDeleteError(false);
          try {
            await client.request(`/api/home/${encodeURIComponent(deleteId)}`, { method: "DELETE" });
            window.dispatchEvent(new Event(THREADS_CHANGED));
            deletedPosts.current.add(deleteId);
            const clearReferences = (post: HomePost): HomePost => ({
              ...post,
              parent: post.parent?.id === deleteId ? null : post.parent,
              quote: post.quote?.post?.id === deleteId ? { ...post.quote, post: null } : post.quote,
            });
            timelineCache.remove(cacheScope, deleteId, clearReferences);
            // Cancel reads started before the successful deletion: they must not resurrect it.
            generation.current++;
            refreshRequest.current?.abort();
            olderRequest.current?.abort();
            refreshRequest.current = null;
            olderRequest.current = null;
            setRefreshing(false);
            setLoadingOlder(false);
            const remaining = postsRef.current
              .filter((post) => post.id !== deleteId)
              .map(clearReferences);
            postsRef.current = remaining;
            setPosts(remaining);
            const removedSelectedPost = postId === deleteId;
            setDeleteId(null);
            toast.success(t("home.deleted"));
            if (removedSelectedPost) goBack();
            else scheduleRefresh.current();
          } catch {
            setDeleteError(true);
          } finally {
            setDeleting(false);
          }
        }}
      />
    </section>
  );
}
