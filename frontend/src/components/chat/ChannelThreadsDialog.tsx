import { useEffect, useMemo, useState, type FormEvent } from "react";
import { useNavigate } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";
import { HashIcon, PlusIcon, SearchIcon } from "lucide-react";

import { createConfiguredApiClient } from "../../lib/api/client";
import { useCollections } from "../../lib/electric/collections-context";
import { localizeApiError } from "../../lib/i18n/apiError";
import { Button } from "../ui/button";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "../ui/dialog";
import { Input } from "../ui/input";
import { Textarea } from "../ui/textarea";
import type { ChannelRow } from "../../lib/electric/types";

type ThreadSummary = {
  channel_id: string;
  title: string;
  archived_at: string | null;
  activity_at: string;
  member_count: number;
  is_member: boolean;
  last_message: string | null;
};

/** Parent channel の公開スレッド一覧・作成・検索をまとめる。 */
export function ChannelThreadsDialog({
  channel,
  open,
  onOpenChange,
}: {
  channel: ChannelRow;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { config } = useCollections();
  const api = useMemo(() => createConfiguredApiClient(config), [config]);
  const [archived, setArchived] = useState(false);
  const [threads, setThreads] = useState<ThreadSummary[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [searchDraft, setSearchDraft] = useState("");
  const [search, setSearch] = useState("");
  const [creating, setCreating] = useState(false);
  const [title, setTitle] = useState("");
  const [firstPost, setFirstPost] = useState("");
  const [createError, setCreateError] = useState<string | null>(null);
  const [creatingThread, setCreatingThread] = useState(false);

  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    setLoading(true);
    setError(null);
    const params = new URLSearchParams({ archived: String(archived) });
    if (search.trim()) params.set("search", search.trim());
    void api
      .get<{ items: ThreadSummary[] }>(`/api/channels/${channel.id}/threads?${params}`)
      .then((result) => {
        if (!cancelled) setThreads(result.items);
      })
      .catch((cause: unknown) => {
        if (!cancelled) setError(localizeApiError(cause, t));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [api, archived, channel.id, open, search, t]);

  async function createThread(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!title.trim() || !firstPost.trim() || creatingThread) return;
    setCreateError(null);
    setCreatingThread(true);
    try {
      const result = await api.post<{ channel_id: string }>(`/api/channels/${channel.id}/threads`, {
        title: title.trim(),
        initial_post: firstPost.trim(),
      });
      onOpenChange(false);
      await navigate({
        to: "/channels/$channelId",
        params: { channelId: result.data.channel_id },
      });
    } catch (cause) {
      setCreateError(localizeApiError(cause, t));
    } finally {
      setCreatingThread(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[min(80vh,44rem)] overflow-hidden sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>{creating ? t("threads.create") : t("threads.title")}</DialogTitle>
          <DialogDescription>
            {creating ? t("threads.createDescription") : t("threads.listDescription")}
          </DialogDescription>
        </DialogHeader>
        {creating ? (
          <form
            className="flex min-h-0 flex-col gap-3"
            onSubmit={(event) => void createThread(event)}
          >
            <Input
              value={title}
              onChange={(event) => setTitle(event.target.value)}
              placeholder={t("threads.titlePlaceholder")}
              aria-label={t("threads.titlePlaceholder")}
              maxLength={100}
              required
            />
            <Textarea
              value={firstPost}
              onChange={(event) => setFirstPost(event.target.value)}
              placeholder={t("threads.firstPostPlaceholder")}
              aria-label={t("threads.firstPostPlaceholder")}
              required
            />
            {createError ? <p className="text-sm text-destructive">{createError}</p> : null}
            <div className="flex justify-end gap-2">
              <Button
                type="button"
                variant="ghost"
                disabled={creatingThread}
                onClick={() => setCreating(false)}
              >
                {t("common.cancelShort")}
              </Button>
              <Button type="submit" disabled={!title.trim() || !firstPost.trim() || creatingThread}>
                {creatingThread ? t("common.loading") : t("threads.create")}
              </Button>
            </div>
          </form>
        ) : (
          <div className="flex min-h-0 flex-col gap-3">
            <form
              className="flex gap-2"
              onSubmit={(event) => {
                event.preventDefault();
                setSearch(searchDraft);
              }}
            >
              <Input
                value={searchDraft}
                onChange={(event) => setSearchDraft(event.target.value)}
                placeholder={t("threads.searchPlaceholder")}
                aria-label={t("threads.searchPlaceholder")}
              />
              <Button type="submit" variant="outline" size="icon" aria-label={t("threads.search")}>
                <SearchIcon />
              </Button>
            </form>
            <div className="flex items-center justify-between gap-2">
              <div className="flex gap-1" role="group" aria-label={t("threads.title")}>
                <Button
                  size="sm"
                  variant={!archived ? "secondary" : "ghost"}
                  onClick={() => setArchived(false)}
                >
                  {t("threads.active")}
                </Button>
                <Button
                  size="sm"
                  variant={archived ? "secondary" : "ghost"}
                  onClick={() => setArchived(true)}
                >
                  {t("threads.archivedList")}
                </Button>
              </div>
              <Button size="sm" onClick={() => setCreating(true)}>
                <PlusIcon /> {t("threads.create")}
              </Button>
            </div>
            <div className="min-h-24 flex-1 overflow-y-auto rounded-md border">
              {loading ? (
                <p className="p-4 text-sm text-muted-foreground">{t("threads.loading")}</p>
              ) : null}
              {error ? <p className="p-4 text-sm text-destructive">{error}</p> : null}
              {!loading && !error && threads.length === 0 ? (
                <p className="p-4 text-sm text-muted-foreground">{t("threads.emptyList")}</p>
              ) : null}
              {threads.map((thread) => (
                <button
                  key={thread.channel_id}
                  type="button"
                  className="flex w-full items-start gap-3 border-b p-3 text-left last:border-b-0 hover:bg-muted/60"
                  onClick={() => {
                    onOpenChange(false);
                    void navigate({
                      to: "/channels/$channelId",
                      params: { channelId: thread.channel_id },
                    });
                  }}
                >
                  <HashIcon className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-sm font-medium">{thread.title}</span>
                    <span className="mt-0.5 block truncate text-xs text-muted-foreground">
                      {thread.last_message ?? t("threads.empty")}
                    </span>
                  </span>
                  <span className="shrink-0 text-xs text-muted-foreground">
                    {thread.member_count}
                  </span>
                </button>
              ))}
            </div>
          </div>
        )}
      </DialogContent>
    </Dialog>
  );
}
