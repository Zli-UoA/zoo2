import { coalescedRefresh } from "../../lib/coalescedRefresh";
import { ActivityPubProfileDetails } from "./ActivityPubProfileDetails";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Link } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";
import { useCollections } from "../../lib/electric/collections-context";
import { createConfiguredApiClient } from "../../lib/api/client";
import { useCurrentUser } from "../../hooks/useCurrentUser";
import { useHomeLive } from "../../hooks/useHomeLive";
import { ActivityPubAvatar, type RemoteAvatar } from "./ActivityPubAvatar";
import { type CustomEmoji } from "./ActivityPubEmoji";
import { Button } from "../ui/button";
import { ExternalLink } from "../ui/external-link";

type Account = {
  id: string;
  user_name: string;
  display_name: string;
  summary?: string;
  avatar?: RemoteAvatar | null;
  avatar_url?: string | null;
  emojis?: CustomEmoji[];
  remote: boolean;
  url?: string;
  following?: boolean | null;
};
export function ActivityPubProfile({
  actorId,
  avatarUrls,
}: {
  actorId: string;
  avatarUrls: ReadonlyMap<string, string | null | undefined>;
}) {
  const { t } = useTranslation();
  const { config } = useCollections();
  const { currentUserId } = useCurrentUser();
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  const request = useRef<AbortController | null>(null);
  const [account, setAccount] = useState<Account | null>(null);
  const [failed, setFailed] = useState(false);
  const [pending, setPending] = useState(false);
  const load = useCallback(async () => {
    request.current?.abort();
    const controller = new AbortController();
    request.current = controller;
    const signal = controller.signal;
    try {
      const value = await client.get<Account>(
        `/api/activitypub/accounts?actor=${encodeURIComponent(actorId)}`,
        { signal },
      );
      if (!signal?.aborted) {
        setAccount(value);
        setFailed(false);
      }
    } catch {
      if (!signal?.aborted) setFailed(true);
    }
  }, [client, actorId]);
  const schedule = useRef<() => void>(() => {});
  useEffect(() => {
    setAccount(null);
    setFailed(false);
    const scheduler = coalescedRefresh(() =>
      document.visibilityState === "hidden" ? undefined : load(),
    );
    schedule.current = scheduler.request;
    scheduler.request();
    return () => {
      schedule.current = () => {};
      scheduler.dispose();
      request.current?.abort();
    };
  }, [load]);
  useHomeLive(() => schedule.current());
  return (
    <section className="space-y-4 border-b p-4">
      {failed ? (
        <p role="alert" className="text-sm text-destructive">
          {t("home.loadError")}{" "}
          <Button variant="link" onClick={() => void load()}>
            {t("home.refresh")}
          </Button>
        </p>
      ) : null}
      {account ? (
        <>
          <ActivityPubProfileDetails
            account={account}
            avatar={
              <ActivityPubAvatar
                name={account.display_name || account.user_name}
                avatar={account.avatar}
                avatarUrl={
                  account.remote
                    ? undefined
                    : avatarUrls.get(account.id) !== undefined
                      ? avatarUrls.get(account.id)
                      : account.avatar_url
                }
              />
            }
          />
          <div className="flex flex-wrap items-center gap-3">
            {account.remote ? (
              <Button
                disabled={pending}
                variant="outline"
                onClick={() => {
                  setPending(true);
                  setFailed(false);
                  void client
                    .request("/api/activitypub/follows", {
                      method: account.following == null ? "POST" : "DELETE",
                      body: { actor: account.id },
                    })
                    .then(() => load())
                    .catch(() => setFailed(true))
                    .finally(() => setPending(false));
                }}
              >
                {t(account.following == null ? "home.follow" : "home.unfollow")}
              </Button>
            ) : null}
            {account.following != null ? (
              <span className="text-sm text-muted-foreground">
                {t(account.following ? "home.following" : "home.followPending")}
              </span>
            ) : null}
            {!account.remote && currentUserId === account.id ? (
              <Link to="/settings" search={{ tab: "activitypub" }} className="text-sm text-link">
                {t("activitypubSettings.editProfile")}
              </Link>
            ) : null}
            {account.remote && account.url ? (
              <ExternalLink href={account.url} className="text-sm text-link">
                {t("activitypubSettings.originalProfile")}
              </ExternalLink>
            ) : null}
          </div>
        </>
      ) : !failed ? (
        <p role="status">{t("home.loading")}</p>
      ) : null}
    </section>
  );
}
