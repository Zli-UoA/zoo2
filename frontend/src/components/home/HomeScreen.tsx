import { Link } from "@tanstack/react-router";
import { HomeComposer } from "./HomeComposer";
import { Input } from "../ui/input";
import { ExternalLink } from "../ui/external-link";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { HomeIcon, RefreshCwIcon, Trash2Icon, PencilIcon } from "lucide-react";
import { createConfiguredApiClient } from "../../lib/api/client";
import { useCollections } from "../../lib/electric/collections-context";
import { useCurrentUser } from "../../hooks/useCurrentUser";
import { ScreenHeader } from "../layout/PaneHeader";
import { Avatar, AvatarFallback } from "../ui/avatar";
import { Button } from "../ui/button";
import { Textarea } from "../ui/textarea";
import { Spinner } from "../ui/spinner";
import { ConfirmActionDialog } from "../ui/confirm-action-dialog";
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
}

export function HomeScreen() {
  const { t, i18n } = useTranslation();
  const { config } = useCollections();
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  const { currentUserId } = useCurrentUser();
  const [timeline, setTimeline] = useState("home");
  const [federation, setFederation] = useState<{
    enabled: boolean;
    actor?: string;
    delivery?: { pending: number; failed: number };
  }>({ enabled: false });
  const [follows, setFollows] = useState<
    { actor_id: string; display_name: string; accepted: boolean; direction: string }[]
  >([]);
  const [account, setAccount] = useState("");
  const [pattern, setPattern] = useState("");
  const [canManage, setCanManage] = useState(false);
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
      client.get<{ ltl_channel_pattern: string; can_manage: boolean }>("/api/home/settings"),
    ]);
    if (results[0].status === "fulfilled") setFederation(results[0].value);
    if (results[1].status === "fulfilled") setFollows(results[1].value);
    if (results[2].status === "fulfilled") {
      setPattern(results[2].value.ltl_channel_pattern);
      setCanManage(results[2].value.can_manage);
    }
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
  const [deleteId, setDeleteId] = useState<string | null>(null);
  const [deleting, setDeleting] = useState(false);
  const [deleteError, setDeleteError] = useState(false);
  const scrollRoot = useRef<HTMLDivElement>(null);
  const generation = useRef(0);
  const refreshRequest = useRef<AbortController | null>(null);
  const olderRequest = useRef<AbortController | null>(null);

  const refresh = useCallback(async () => {
    const version = ++generation.current;
    refreshRequest.current?.abort();
    olderRequest.current?.abort();
    setLoadingOlder(false);
    const controller = new AbortController();
    refreshRequest.current = controller;
    try {
      const rows = await client.get<HomePost[]>(`/api/home?timeline=${timeline}`, {
        signal: controller.signal,
      });
      if (version !== generation.current) return;
      // Replace the current window so deletions on other devices disappear too.
      setPosts(rows);
      setHasMore(rows.length === 50);
      setError(false);
    } catch {
      if (!controller.signal.aborted) setError(true);
    } finally {
      if (version === generation.current) setLoading(false);
    }
  }, [client, timeline]);

  useEffect(() => {
    setLoading(true);
    setPosts([]);
    void refresh();
    const timer = setInterval(() => {
      if (
        document.visibilityState === "visible" &&
        !olderRequest.current &&
        (scrollRoot.current?.scrollTop ?? 0) < 20
      )
        void refresh();
    }, 30000);
    return () => {
      clearInterval(timer);
      generation.current++;
      refreshRequest.current?.abort();
      olderRequest.current?.abort();
    };
  }, [refresh]);

  const older = async () => {
    if (loadingOlder || !posts.length) return;
    setLoadingOlder(true);
    const version = generation.current;
    const controller = new AbortController();
    olderRequest.current = controller;
    try {
      const rows = await client.get<HomePost[]>(
        `/api/home?timeline=${timeline}&before=${encodeURIComponent(posts[posts.length - 1].id)}`,
        { signal: controller.signal },
      );
      if (version !== generation.current) return;
      setPosts((current) => [
        ...current,
        ...rows.filter((p) => !current.some((old) => old.id === p.id)),
      ]);
      setHasMore(rows.length === 50);
      setError(false);
    } catch {
      if (!controller.signal.aborted) setError(true);
    } finally {
      if (olderRequest.current === controller) {
        olderRequest.current = null;
        setLoadingOlder(false);
      }
    }
  };
  return (
    <section className="relative flex h-full min-h-0 flex-col" aria-labelledby="home-title">
      <ScreenHeader
        icon={HomeIcon}
        titleId="home-title"
        title={t("home.title")}
        actions={
          <Button
            variant="ghost"
            size="icon"
            aria-label={t("home.refresh")}
            onClick={() => void refresh()}
          >
            <RefreshCwIcon />
          </Button>
        }
      />
      <div ref={scrollRoot} className="min-h-0 flex-1 overflow-y-auto pb-24">
        <div className="mx-auto w-full max-w-2xl">
          <details className="border-b p-4 text-sm">
            <summary className="cursor-pointer">{t("home.federation")}</summary>
            <div className="mt-3 space-y-3">
              <p className="break-all text-xs text-muted-foreground">
                {federation.enabled ? federation.actor : t("home.federationDisabled")}
              </p>
              {federation.delivery ? (
                <p className="text-xs">{t("home.delivery", federation.delivery)}</p>
              ) : null}
              <form
                className="flex flex-wrap gap-2"
                onSubmit={(e) => {
                  e.preventDefault();
                  void controlAction(async () => {
                    await client.request("/api/activitypub/follows", {
                      method: "POST",
                      body: { actor: account },
                    });
                    setAccount("");
                  });
                }}
              >
                <Input
                  aria-label={t("home.remoteAccount")}
                  value={account}
                  onChange={(e) => setAccount(e.target.value)}
                  placeholder="@user@example.social"
                  className="min-w-0 flex-1"
                  disabled={controlsPending || !federation.enabled}
                />
                <Button
                  type="submit"
                  disabled={controlsPending || !federation.enabled || !account.trim()}
                >
                  {t("home.follow")}
                </Button>
              </form>
              <ul className="space-y-2">
                {follows
                  .filter((f) => f.direction === "outgoing")
                  .map((f) => (
                    <li key={f.actor_id} className="flex items-center gap-2">
                      <span className="min-w-0 flex-1 break-all text-xs">
                        {f.display_name} · {t(f.accepted ? "home.following" : "home.followPending")}
                      </span>
                      <Button
                        size="sm"
                        variant="outline"
                        disabled={controlsPending}
                        onClick={() =>
                          void controlAction(() =>
                            client.request("/api/activitypub/follows", {
                              method: "DELETE",
                              body: { actor: f.actor_id },
                            }),
                          )
                        }
                      >
                        {t("home.unfollow")}
                      </Button>
                    </li>
                  ))}
              </ul>
              {canManage ? (
                <form
                  className="space-y-2"
                  onSubmit={(e) => {
                    e.preventDefault();
                    void controlAction(() =>
                      client.request("/api/home/settings", {
                        method: "PUT",
                        body: { ltl_channel_pattern: pattern },
                      }),
                    );
                  }}
                >
                  <label htmlFor="ltl-pattern" className="text-xs">
                    {t("home.channelRule")}
                  </label>
                  <Input
                    id="ltl-pattern"
                    value={pattern}
                    onChange={(e) => setPattern(e.target.value)}
                    placeholder="public-%"
                    maxLength={200}
                    disabled={controlsPending}
                  />
                  <p className="text-xs text-muted-foreground">{t("home.ruleHint")}</p>
                  <Button disabled={controlsPending} type="submit" variant="outline">
                    {t("home.save")}
                  </Button>
                </form>
              ) : null}
              {controlsError ? (
                <p role="alert" className="text-xs text-destructive">
                  {t("home.controlsError")}
                </p>
              ) : null}
            </div>
          </details>
          <div className="flex gap-2 border-b p-3" aria-label={t("home.timeline")}>
            <Button
              variant={timeline === "home" ? "secondary" : "ghost"}
              aria-pressed={timeline === "home"}
              onClick={() => setTimeline("home")}
            >
              {t("home.title")}
            </Button>
            <Button
              variant={timeline === "local" ? "secondary" : "ghost"}
              aria-pressed={timeline === "local"}
              onClick={() => setTimeline("local")}
            >
              {t("home.localTimeline")}
            </Button>
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
                <li key={post.id} className="border-b p-4">
                  <article className="flex min-w-0 gap-3">
                    <Avatar>
                      <AvatarFallback>
                        {initials(post.display_name || post.user_name)}
                      </AvatarFallback>
                    </Avatar>
                    <div className="min-w-0 flex-1">
                      <div className="flex flex-wrap items-baseline gap-x-2 gap-y-1 text-xs text-muted-foreground">
                        <span className="break-all text-sm font-semibold text-foreground">
                          {post.display_name || post.user_name}
                        </span>
                        <span className="break-all">@{post.user_name}</span>
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
                        <span className="ml-auto inline-flex items-center gap-1">
                          {post.user_id === currentUserId && !post.remote ? (
                            <Button
                              variant="ghost"
                              size="icon-sm"
                              aria-label={t("home.edit")}
                              onClick={() => {
                                setEditingId(post.id);
                                setEditText(post.content);
                                setEditError(false);
                              }}
                            >
                              <PencilIcon />
                            </Button>
                          ) : null}
                          {post.user_id === currentUserId && !post.remote ? (
                            <Button
                              variant="ghost"
                              size="icon-sm"
                              aria-label={t("home.delete")}
                              onClick={() => {
                                setDeleteError(false);
                                setDeleteId(post.id);
                              }}
                            >
                              <Trash2Icon />
                            </Button>
                          ) : null}
                        </span>
                      </div>
                      {editingId === post.id ? (
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
                          {post.content.trim() ? post.content : t("home.attachmentsOnly")}
                        </p>
                      )}
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
      <HomeComposer
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
