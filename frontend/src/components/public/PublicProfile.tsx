import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { ActivityPubProfileDetails, type ProfileDetails } from "../home/ActivityPubProfileDetails";
import { HomePostContent } from "../home/HomePostContent";
import { Avatar, AvatarFallback } from "../ui/avatar";
import { Button, buttonVariants } from "../ui/button";
import { Input } from "../ui/input";
import { initials } from "../chat/format";
import { joinUrl, resolvePublicConfig } from "../../lib/config";

export interface PublicProfileData {
  account: Omit<ProfileDetails, "remote"> & { actor_id: string; url: string };
  posts: {
    id: string;
    content: string;
    created_at: string;
    media?: import("../home/HomePostContent").HomeMedia[];
    emojis?: import("../home/ActivityPubEmoji").CustomEmoji[];
  }[];
  next_cursor: string | null;
}

export function PublicProfile({ userId, before }: { userId: string; before?: string }) {
  const { t, i18n } = useTranslation();
  const [data, setData] = useState<PublicProfileData | null>(null);
  const [error, setError] = useState<number | null>(null);
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    const controller = new AbortController();
    setData(null);
    setError(null);
    const query = before ? `?before=${encodeURIComponent(before)}` : "";
    // Anonymous requests never initialize OIDC, current-user stores or Electric.
    void fetch(
      joinUrl(
        resolvePublicConfig().apiBaseUrl,
        `/api/activitypub/users/${encodeURIComponent(userId)}/profile${query}`,
      ),
      {
        signal: controller.signal,
        credentials: "omit",
        headers: { Accept: "application/json" },
      },
    )
      .then(async (response) => {
        if (controller.signal.aborted) return;
        if (!response.ok) {
          setError(response.status);
          return;
        }
        const value = (await response.json()) as PublicProfileData;
        if (!controller.signal.aborted) setData(value);
      })
      .catch(() => {
        if (!controller.signal.aborted) setError(500);
      });
    return () => controller.abort();
  }, [userId, before, attempt]);
  const account = data?.account;
  const name = account?.display_name || account?.user_name || "";
  const address = account ? `@${account.user_name}@${new URL(account.actor_id).host}` : "";
  const path = `/public/users/${encodeURIComponent(userId)}`;
  return (
    <main className="h-dvh overflow-y-auto bg-muted text-foreground">
      <div className="mx-auto max-w-2xl space-y-6 px-4 py-6 sm:py-12">
        <a
          href="/"
          aria-label="Zoo"
          className="inline-flex items-center gap-2.5 text-[2.5rem] font-extrabold leading-none tracking-[-.06em] text-foreground"
        >
          <img src="/icons/zoo.svg" alt="" width={40} height={40} className="size-10 rounded-xl" />
          <span>
            Zoo<span className="text-primary">.</span>
          </span>
        </a>
        {account ? (
          <>
            <title>{name} · Zoo</title>
            <meta name="description" content={account.summary || ""} />
            <meta property="og:title" content={`${name} · Zoo`} />
            <meta property="og:description" content={account.summary || ""} />
            <meta property="og:type" content="profile" />
            <meta property="og:url" content={account.url} />
            <link rel="canonical" href={account.url} />
            <link rel="alternate" type="application/activity+json" href={account.actor_id} />
            <section className="space-y-4 rounded-2xl border bg-card p-5 sm:p-8">
              <ActivityPubProfileDetails
                headingLevel={1}
                account={{ ...account, id: account.actor_id, remote: true }}
                avatar={
                  <Avatar role="img" aria-label={name}>
                    <AvatarFallback aria-hidden="true">{initials(name)}</AvatarFallback>
                  </Avatar>
                }
              />
              <section className="space-y-3 border-t pt-5" aria-labelledby="follow-heading">
                <h2 id="follow-heading" className="font-semibold">
                  {t("publicProfile.followTitle")}
                </h2>
                <p className="text-sm">{t("publicProfile.followDescription")}</p>
                <label htmlFor="public-account" className="block text-sm font-medium">
                  {t("publicProfile.accountAddress")}
                </label>
                <Input id="public-account" className="w-full" readOnly value={address} />
                <p className="text-sm text-muted-foreground">
                  {t("publicProfile.approvalRequired")}
                </p>
              </section>
              <a href={`/home/users/${encodeURIComponent(userId)}`} className={buttonVariants()}>
                {t("publicProfile.openInZoo")}
              </a>
            </section>
            <section aria-labelledby="posts-heading" className="space-y-4">
              <h2 id="posts-heading" className="font-semibold">
                {t("publicProfile.posts")}
              </h2>
              {data.posts.map((post) => (
                <article
                  key={post.id}
                  id={`post-${post.id}`}
                  className="rounded-2xl border bg-card p-5 sm:p-6"
                >
                  <a
                    href={`#post-${post.id}`}
                    className="text-sm text-muted-foreground underline-offset-4 hover:underline"
                  >
                    <time dateTime={post.created_at}>
                      {new Date(post.created_at).toLocaleString(i18n.language)}
                    </time>
                  </a>
                  <HomePostContent content={post.content} emojis={post.emojis} media={post.media} />
                </article>
              ))}
              {!data.posts.length ? (
                <p className="text-sm text-muted-foreground">{t("publicProfile.empty")}</p>
              ) : null}
              <nav aria-label={t("publicProfile.pagination")} className="flex flex-wrap gap-3">
                {before ? (
                  <a className={buttonVariants({ variant: "outline" })} href={path}>
                    {t("publicProfile.latest")}
                  </a>
                ) : null}
                {data.next_cursor ? (
                  <a
                    className={buttonVariants({ variant: "outline" })}
                    href={`${path}?before=${encodeURIComponent(data.next_cursor)}`}
                  >
                    {t("publicProfile.older")}
                  </a>
                ) : null}
              </nav>
            </section>
          </>
        ) : error ? (
          <section role="alert" className="space-y-3 rounded-2xl border bg-card p-5">
            <p>{t(error === 404 ? "publicProfile.notFound" : "home.loadError")}</p>
            {error !== 404 ? (
              <Button variant="outline" onClick={() => setAttempt((value) => value + 1)}>
                {t("home.refresh")}
              </Button>
            ) : null}
          </section>
        ) : (
          <p role="status">{t("home.loading")}</p>
        )}
      </div>
    </main>
  );
}
