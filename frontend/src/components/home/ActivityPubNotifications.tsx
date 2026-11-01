import { Link } from "@tanstack/react-router";
import { ActivityPubEmoji, ActivityPubText } from "./ActivityPubEmoji";
import { ActivityPubAvatar } from "./ActivityPubAvatar";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useActivityPubNotifications } from "../../hooks/useActivityPubNotifications";
import { Button } from "../ui/button";

export function ActivityPubNotifications({
  notifications,
}: {
  notifications: ReturnType<typeof useActivityPubNotifications>;
}) {
  const { t, i18n } = useTranslation();
  const { items, unread_count, error, refresh, act, next_cursor, loadingMore, loadMore } =
    notifications;
  const [pending, setPending] = useState(false);
  const [failed, setFailed] = useState(false);
  const action = async (path: string, body: unknown) => {
    if (pending) return;
    setPending(true);
    setFailed(false);
    try {
      await act(path, body);
    } catch {
      setFailed(true);
    } finally {
      setPending(false);
    }
  };
  if (!items.length && !error) return null;
  return (
    <section aria-label={t("home.apubNotifications")} className="border-b p-4">
      <div className="mb-3 flex items-center justify-between gap-2">
        <h2 className="text-sm font-semibold">{t("home.apubNotifications")}</h2>
        {unread_count > 0 ? (
          <Button
            size="sm"
            variant="ghost"
            disabled={pending}
            onClick={() => void action("/api/activitypub/notifications/read", {})}
          >
            {t("home.readAll")}
          </Button>
        ) : null}
      </div>
      {error || failed ? (
        <p role="alert" className="text-sm text-destructive">
          {t("home.controlsError")}{" "}
          <Button variant="link" onClick={() => void refresh()}>
            {t("home.refresh")}
          </Button>
        </p>
      ) : null}
      <ul className="space-y-3">
        {items.map((item) => (
          <li
            key={item.id}
            className="space-y-2 rounded-lg border p-3 data-[unread]:bg-accent/50"
            data-unread={!item.is_read || undefined}
          >
            <div className="flex items-center gap-3">
              <Link
                to="/home/users/$actorId"
                params={{ actorId: item.actor_id.replace(/^local:/, "") }}
                className="shrink-0 rounded-full focus-visible:ring-2 focus-visible:ring-ring"
              >
                <ActivityPubAvatar avatar={item.avatar} name={item.display_name} />
              </Link>
              <p className="min-w-0 break-words text-sm">
                <strong>
                  <Link
                    to="/home/users/$actorId"
                    params={{ actorId: item.actor_id.replace(/^local:/, "") }}
                    className="hover:underline"
                  >
                    <ActivityPubText text={item.display_name} emojis={item.actor_emojis} />
                  </Link>
                </strong>{" "}
                {t(`home.notificationKinds.${item.kind}`)}
              </p>
            </div>
            <time className="text-xs text-muted-foreground" dateTime={item.created_at}>
              {new Date(item.created_at).toLocaleString(i18n.language)}
            </time>
            {item.kind === "like" && item.reaction ? (
              <p className="text-lg">
                {item.emoji ? <ActivityPubEmoji emoji={item.emoji} /> : item.reaction}
              </p>
            ) : null}
            {item.content ? (
              <p className="line-clamp-3 whitespace-pre-wrap break-words text-sm">{item.content}</p>
            ) : null}
            <div className="flex flex-wrap gap-2">
              {item.kind === "follow_request" && item.pending ? (
                <>
                  <Button
                    size="sm"
                    disabled={pending}
                    onClick={() =>
                      void action("/api/activitypub/follows/respond", {
                        actor: item.actor_id,
                        activity_id: item.activity_id,
                        accept: true,
                      })
                    }
                  >
                    {t("home.approve")}
                  </Button>
                  <Button
                    size="sm"
                    variant="outline"
                    disabled={pending}
                    onClick={() =>
                      void action("/api/activitypub/follows/respond", {
                        actor: item.actor_id,
                        activity_id: item.activity_id,
                        accept: false,
                      })
                    }
                  >
                    {t("home.reject")}
                  </Button>
                </>
              ) : null}
              {!item.is_read ? (
                <Button
                  size="sm"
                  variant="ghost"
                  disabled={pending}
                  onClick={() =>
                    void action("/api/activitypub/notifications/read", { id: item.id })
                  }
                >
                  {t("home.markRead")}
                </Button>
              ) : null}
            </div>
          </li>
        ))}
      </ul>
      {next_cursor ? (
        <Button
          className="mt-3"
          variant="outline"
          disabled={pending || loadingMore}
          onClick={() => void loadMore()}
        >
          {t(loadingMore ? "home.loading" : "home.moreNotifications")}
        </Button>
      ) : null}
    </section>
  );
}
