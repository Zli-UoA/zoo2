import {
  ActivityPubText,
  ActivityPubEmoji,
  FederatedAccount,
  type CustomEmoji,
} from "./ActivityPubEmoji";
import { HomeReactionPicker, type ReactionChoice } from "./HomeReactionPicker";
import { useHomeLive } from "../../hooks/useHomeLive";
import { Link } from "@tanstack/react-router";
import { HomeComposer, type HomeComposerHandle } from "./HomeComposer";
import { Input } from "../ui/input";
import { ExternalLink } from "../ui/external-link";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  HomeIcon,
  RefreshCwIcon,
  UserSearchIcon,
  ArrowDownIcon,
  Trash2Icon,
  PencilIcon,
  MoreHorizontalIcon,
  ReplyIcon,
  Repeat2Icon,
  SmilePlusIcon,
} from "lucide-react";
import { createConfiguredApiClient } from "../../lib/api/client";
import { useCollections } from "../../lib/electric/collections-context";
import { useCurrentUser } from "../../hooks/useCurrentUser";
import { ScreenHeader } from "../layout/PaneHeader";
import { Avatar, AvatarFallback } from "../ui/avatar";
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
import { initials } from "../chat/format";

export interface HomePost {
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
  parent_id?: string | null;
  emojis?: CustomEmoji[];
  actor_emojis?: CustomEmoji[];
  reactions?: { reaction: string; emoji: CustomEmoji | null; count: number; selected: boolean }[];
  replies?: number;
  reposts?: number;
  reposted?: boolean;
  reposted_by?: string | null;
  repost_created_at?: string;
}

export function HomeScreen() {
  const { t, i18n } = useTranslation();
  const { config } = useCollections();
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  const { currentUserId } = useCurrentUser();
  const composerRef = useRef<HomeComposerHandle>(null);
  const [reactionPost, setReactionPost] = useState<HomePost | null>(null);
  const [pendingRepost, setPendingRepost] = useState<string | null>(null);
  const [timeline, setTimeline] = useState("home");
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
      }[]
    | null
  >(null);
  const [searchPending, setSearchPending] = useState(false);
  const [searchError, setSearchError] = useState(false);
  const searchVersion = useRef(0);
  const [pull, setPull] = useState(0);
  const [pulling, setPulling] = useState(false);
  const touchStart = useRef<number | null>(null);
  const [account, setAccount] = useState("");
  const [controlsPending, setControlsPending] = useState(false);
  const [controlsError, setControlsError] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editText, setEditText] = useState("");
  const [editError, setEditError] = useState(false);
  const [editPending, setEditPending] = useState(false);
  const loadControls = useCallback(async () => {
    const results = await Promise.allSettled([
      client.get<typeof federation>("/api/activitypub/status"),
      client.get<typeof follows>("/api/activitypub/follows"),
    ]);
    if (results[0].status === "fulfilled") setFederation(results[0].value);
    if (results[1].status === "fulfilled") setFollows(results[1].value);
    setControlsError(results.some((r) => r.status === "rejected"));
  }, [client]);
  useEffect(() => {
    void loadControls();
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
  const [posts, setPosts] = useState<HomePost[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadingOlder, setLoadingOlder] = useState(false);
  const [hasMore, setHasMore] = useState(false);
  const [error, setError] = useState(false);
  const [pendingLike, setPendingLike] = useState<string | null>(null);
  const [reactionError, setReactionError] = useState<string | null>(null);
  const [deleteId, setDeleteId] = useState<string | null>(null);
  const [deleting, setDeleting] = useState(false);
  const [deleteError, setDeleteError] = useState(false);
  const scrollRoot = useRef<HTMLDivElement>(null);
  const generation = useRef(0);
  const refreshRequest = useRef<AbortController | null>(null);
  const olderRequest = useRef<AbortController | null>(null);

  const anchor = useRef<{ id: string; offset: number } | null>(null);
  useLayoutEffect(() => {
    if (!anchor.current || !scrollRoot.current) return;
    const root = scrollRoot.current;
    const element = Array.from(root.querySelectorAll<HTMLElement>("[data-home-post]")).find(
      (node) => node.dataset.homePost === anchor.current?.id,
    );
    if (element)
      root.scrollTop +=
        element.getBoundingClientRect().top -
        root.getBoundingClientRect().top -
        anchor.current.offset;
    anchor.current = null;
  }, [posts]);
  const loadedCount = useRef(50);
  const loadedTail = useRef<{ id: string; at: number } | null>(null);
  const deferredRefresh = useRef(false);
  const refresh = useCallback(
    async (retain = false) => {
      if (retain && olderRequest.current) {
        deferredRefresh.current = true;
        return;
      }
      deferredRefresh.current = false;
      const limit = retain ? Math.max(50, loadedCount.current) : 50;
      const version = ++generation.current;
      refreshRequest.current?.abort();
      olderRequest.current?.abort();
      setLoadingOlder(false);
      const controller = new AbortController();
      refreshRequest.current = controller;
      try {
        const rows: HomePost[] = [];
        const tail = retain ? loadedTail.current : null;
        let reachedTail = !tail;
        let more = false;
        while (rows.length < limit || !reachedTail) {
          const size = Math.min(500, Math.max(50, limit - rows.length));
          const before = rows.length
            ? `&before=${encodeURIComponent(rows[rows.length - 1].entry_id ?? rows[rows.length - 1].id)}`
            : "";
          const page = await client.get<HomePost[]>(
            `/api/home?timeline=${timeline}${size !== 50 ? `&limit=${size}` : ""}${before}`,
            { signal: controller.signal },
          );
          rows.push(...page);
          const last = rows.at(-1);
          if (tail && last) {
            const at = Date.parse(last.repost_created_at ?? last.created_at);
            reachedTail = at < tail.at || (at === tail.at && (last.entry_id ?? last.id) <= tail.id);
          }
          more = page.length === size;
          if (!more) break;
        }
        if (version !== generation.current) return;
        // Replace the current window so deletions on other devices disappear too.
        const root = scrollRoot.current;
        if (retain && root && root.scrollTop > 0) {
          const top = root.getBoundingClientRect().top;
          const first = Array.from(root.querySelectorAll<HTMLElement>("[data-home-post]")).find(
            (node) =>
              node.getBoundingClientRect().bottom > top &&
              rows.some((post) => (post.entry_id ?? post.id) === node.dataset.homePost),
          );
          if (first)
            anchor.current = {
              id: first.dataset.homePost!,
              offset: first.getBoundingClientRect().top - top,
            };
        }
        loadedCount.current = rows.length;
        const last = rows.at(-1);
        loadedTail.current = last
          ? {
              id: last.entry_id ?? last.id,
              at: Date.parse(last.repost_created_at ?? last.created_at),
            }
          : null;
        setPosts(rows);
        setHasMore(more);
        setError(false);
      } catch {
        if (!controller.signal.aborted) setError(true);
      } finally {
        if (version === generation.current) setLoading(false);
      }
    },
    [client, timeline],
  );

  useEffect(() => {
    setLoading(true);
    setPosts([]);
    void refresh();
    return () => {
      generation.current++;
      deferredRefresh.current = false;
      refreshRequest.current?.abort();
      olderRequest.current?.abort();
    };
  }, [refresh]);
  useHomeLive(() => {
    void refresh(true);
    void loadControls();
  });

  const older = async () => {
    if (loadingOlder || !posts.length) return;
    setLoadingOlder(true);
    const version = generation.current;
    const controller = new AbortController();
    olderRequest.current = controller;
    try {
      const rows = await client.get<HomePost[]>(
        `/api/home?timeline=${timeline}&before=${encodeURIComponent(posts[posts.length - 1].entry_id ?? posts[posts.length - 1].id)}`,
        { signal: controller.signal },
      );
      if (version !== generation.current) return;
      loadedCount.current += rows.length;
      const last = rows.at(-1);
      if (last)
        loadedTail.current = {
          id: last.entry_id ?? last.id,
          at: Date.parse(last.repost_created_at ?? last.created_at),
        };
      setPosts((current) => [
        ...current,
        ...rows.filter(
          (p) => !current.some((old) => (old.entry_id ?? old.id) === (p.entry_id ?? p.id)),
        ),
      ]);
      setHasMore(rows.length === 50);
      setError(false);
    } catch {
      if (!controller.signal.aborted) setError(true);
    } finally {
      if (olderRequest.current === controller) {
        olderRequest.current = null;
        setLoadingOlder(false);
        if (deferredRefresh.current) void refresh(true);
      }
    }
  };
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
        titleId="home-title"
        title={t("home.title")}
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
      <div
        ref={scrollRoot}
        className="min-h-0 flex-1 overflow-y-auto overscroll-y-contain pb-24"
        onTouchStart={(event) => {
          touchStart.current =
            event.touches.length === 1 && (scrollRoot.current?.scrollTop ?? 0) <= 0
              ? event.touches[0].clientY
              : null;
        }}
        onTouchMove={(event) => {
          if (touchStart.current === null || pulling) return;
          if ((scrollRoot.current?.scrollTop ?? 0) > 0 || event.touches.length !== 1) {
            touchStart.current = null;
            setPull(0);
            return;
          }
          setPull(Math.min(100, Math.max(0, (event.touches[0].clientY - touchStart.current) / 2)));
        }}
        onTouchCancel={() => {
          touchStart.current = null;
          setPull(0);
        }}
        onTouchEnd={() => {
          touchStart.current = null;
          if (pull >= 60 && !pulling) {
            setPulling(true);
            void refresh().finally(() => {
              setPulling(false);
              setPull(0);
            });
          } else setPull(0);
        }}
      >
        {pull > 0 || pulling ? (
          <div
            role="status"
            className="flex items-center justify-center gap-2 text-xs text-muted-foreground md:hidden"
            style={{ height: pulling ? 60 : pull }}
          >
            {pulling ? (
              <RefreshCwIcon className="size-4 animate-spin" />
            ) : (
              <ArrowDownIcon className="size-4" />
            )}
            {t(pulling ? "home.loading" : pull >= 60 ? "home.releaseRefresh" : "home.pullRefresh")}
          </div>
        ) : null}
        <div className="mx-auto w-full max-w-2xl">
          <div className="flex gap-2 border-b p-3" aria-label={t("home.timeline")}>
            {(["home", "local", "social"] as const).map((value) => (
              <Button
                key={value}
                variant={timeline === value ? "secondary" : "ghost"}
                aria-pressed={timeline === value}
                onClick={() => setTimeline(value)}
              >
                {t(`home.timelineLabels.${value}`)}
              </Button>
            ))}
          </div>
          {error ? (
            <p role="alert" className="p-4 text-sm text-destructive">
              {t("home.loadError")}{" "}
              <Button variant="link" onClick={() => void refresh()}>
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
            <ol aria-label={t("home.timeline")}>
              {posts.map((post) => (
                <li
                  key={post.entry_id ?? post.id}
                  data-home-post={post.entry_id ?? post.id}
                  className="border-b p-4"
                >
                  {post.reposted_by ? (
                    <p className="mb-2 flex items-center gap-2 text-xs text-muted-foreground">
                      <Repeat2Icon className="size-3" />
                      {t("home.repostedBy", { name: post.reposted_by })}
                    </p>
                  ) : null}
                  <article className="flex min-w-0 gap-3">
                    <Avatar>
                      <AvatarFallback>
                        {initials(post.display_name || post.user_name)}
                      </AvatarFallback>
                    </Avatar>
                    <div className="min-w-0 flex-1">
                      <div className="flex flex-wrap items-baseline gap-x-2 gap-y-1 text-xs text-muted-foreground">
                        <span className="break-all text-sm font-semibold text-foreground">
                          <ActivityPubText
                            text={post.display_name || post.user_name}
                            emojis={post.actor_emojis}
                          />
                        </span>
                        <FederatedAccount
                          userName={post.user_name}
                          actorId={post.user_id}
                          remote={post.remote}
                        />
                        <span className="inline-flex min-w-0 max-w-full items-baseline gap-2">
                          <time className="shrink-0" dateTime={post.created_at}>
                            {new Date(post.created_at).toLocaleString(i18n.language)}
                          </time>
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
                                setEditingId(null);
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
                              editPending || !editText.trim() || Array.from(editText).length > 40000
                            }
                          >
                            {t("home.save")}
                          </Button>
                          <Button
                            variant="ghost"
                            disabled={editPending}
                            onClick={() => setEditingId(null)}
                          >
                            {t("common.cancel")}
                          </Button>
                          {editError ? (
                            <p role="alert" className="text-xs text-destructive">
                              {t("home.controlsError")}
                            </p>
                          ) : null}
                        </form>
                      ) : (
                        <p className="mt-2 whitespace-pre-wrap break-words text-sm [overflow-wrap:anywhere]">
                          <ActivityPubText
                            text={post.content.trim() ? post.content : t("home.attachmentsOnly")}
                            emojis={post.emojis}
                          />
                        </p>
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
                              disabled={pendingLike !== null}
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
                                <span>{r.reaction}</span>
                              )}
                              <span>{r.count}</span>
                            </Button>
                          ))}
                        </div>
                      ) : null}
                      <div
                        role="group"
                        aria-label={t("home.actions")}
                        className="mt-2 flex items-center justify-between gap-2 text-muted-foreground"
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
                                size="icon-sm"
                                aria-label={t("home.postActions")}
                              />
                            }
                          >
                            <MoreHorizontalIcon />
                          </DropdownMenuTrigger>
                          <DropdownMenuContent align="end">
                            {post.user_id === currentUserId && !post.remote ? (
                              <>
                                <DropdownMenuItem
                                  onClick={() => {
                                    setEditingId(post.entry_id ?? post.id);
                                    setEditText(post.content);
                                    setEditError(false);
                                  }}
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
                            <DropdownMenuItem
                              onClick={() => {
                                if (!navigator.clipboard) {
                                  setReactionError(post.id);
                                  return;
                                }
                                void navigator.clipboard
                                  .writeText(post.content)
                                  .catch(() => setReactionError(post.id));
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
                      {post.remote && post.url?.startsWith("https://") ? (
                        <ExternalLink className="text-xs" href={post.url}>
                          {t("home.original")}
                        </ExternalLink>
                      ) : null}
                    </div>
                  </article>
                </li>
              ))}
            </ol>
          )}
          {hasMore ? (
            <div className="flex justify-center p-4">
              <Button variant="outline" disabled={loadingOlder} onClick={() => void older()}>
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
                    <Avatar>
                      <AvatarFallback>
                        {initials(user.display_name || user.user_name)}
                      </AvatarFallback>
                    </Avatar>
                    <div className="min-w-0">
                      <h3 className="break-all font-semibold">
                        <ActivityPubText
                          text={user.display_name || user.user_name}
                          emojis={user.emojis}
                        />
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
        ref={composerRef}
        federationEnabled={federation.enabled}
        onPost={async (post) => {
          await client.request("/api/home", { method: "POST", body: post });
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
            setDeleteId(null);
            await refresh();
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
