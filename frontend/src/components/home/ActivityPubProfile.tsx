import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Link } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";
import { useCollections } from "../../lib/electric/collections-context";
import { createConfiguredApiClient } from "../../lib/api/client";
import { useCurrentUser } from "../../hooks/useCurrentUser";
import { useHomeLive } from "../../hooks/useHomeLive";
import { ActivityPubAvatar, type RemoteAvatar } from "./ActivityPubAvatar";
import {
  ActivityPubText,
  ActivityPubContent,
  FederatedAccount,
  type CustomEmoji,
} from "./ActivityPubEmoji";
import { Button } from "../ui/button";
import { ExternalLink } from "../ui/external-link";

type Account = {
  id: string;
  user_name: string;
  display_name: string;
  summary?: string;
  avatar?: RemoteAvatar | null;
  emojis?: CustomEmoji[];
  remote: boolean;
  url?: string;
  following?: boolean | null;
};
export function ActivityPubProfile({ actorId }: { actorId: string }) {
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
  useEffect(() => {
    setAccount(null);
    setFailed(false);
    void load();
    return () => request.current?.abort();
  }, [load]);
  useHomeLive(() => void load());
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
          <div className="flex items-center gap-3">
            <ActivityPubAvatar
              name={account.display_name || account.user_name}
              avatar={account.avatar}
            />
            <div className="min-w-0">
              <h2 className="break-words text-lg font-semibold">
                <ActivityPubText
                  text={account.display_name || account.user_name}
                  emojis={account.emojis}
                />
              </h2>
              <FederatedAccount
                userName={account.user_name}
                actorId={account.id}
                remote={account.remote}
              />
            </div>
          </div>
          {account.summary ? (
            <div className="mfm-body whitespace-pre-wrap break-words text-sm leading-relaxed">
              <ActivityPubContent text={account.summary} emojis={account.emojis} />
            </div>
          ) : null}
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
