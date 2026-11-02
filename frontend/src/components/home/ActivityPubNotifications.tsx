import { Link, useNavigate } from "@tanstack/react-router";
import {
  AtSignIcon,
  MessageCircleIcon,
  Repeat2Icon,
  UserPlusIcon,
  UserCheckIcon,
  HomeIcon,
} from "lucide-react";
import { ActivityPubEmoji, ActivityPubText, ActivityPubContent } from "./ActivityPubEmoji";
import { ActivityPubAvatar } from "./ActivityPubAvatar";
import { useMemo, useState, type RefObject } from "react";
import { useTranslation } from "react-i18next";
import type { ActivityPubNotification } from "../../hooks/useActivityPubNotifications";
import { useReadVisibleNotification } from "../chat/notifications/useReadVisibleNotification";
import {
  NotificationCardFrame,
  NotificationReadStatus,
} from "../chat/notifications/NotificationCard";
import { Button } from "../ui/button";

export function ActivityPubNotificationCard({
  item,
  onRead,
  scrollRoot,
  readEnabled = true,
  act,
}: {
  item: ActivityPubNotification;
  onRead: (ids: readonly string[]) => void;
  scrollRoot: RefObject<HTMLDivElement | null>;
  readEnabled?: boolean;
  act: (path: string, body: unknown) => Promise<void>;
}) {
  const { t, i18n } = useTranslation();
  const navigate = useNavigate();
  const unread = useMemo(
    () => (item.is_read ? [] : [`activitypub:${item.id}`]),
    [item.id, item.is_read],
  );
  const { ref } = useReadVisibleNotification(unread, onRead, scrollRoot, readEnabled);
  const [pending, setPending] = useState(false);
  const [failed, setFailed] = useState(false);
  const action = async (accept: boolean) => {
    if (pending) return;
    setPending(true);
    setFailed(false);
    try {
      await act("/api/activitypub/follows/respond", {
        actor: item.actor_id,
        activity_id: item.activity_id,
        accept,
      });
    } catch {
      setFailed(true);
    } finally {
      setPending(false);
    }
  };
  const kind = t(`home.notificationKinds.${item.kind}`);
  const label = `${item.display_name} ${kind}`;
  const Icon =
    item.kind === "reply"
      ? MessageCircleIcon
      : item.kind === "repost"
        ? Repeat2Icon
        : item.kind === "follow_accepted"
          ? UserCheckIcon
          : item.kind === "follow_request"
            ? UserPlusIcon
            : AtSignIcon;
  const authorId = item.actor_id.replace(/^local:/, "");
  return (
    <NotificationCardFrame
      ref={ref}
      aria-label={label}
      data-unread={!item.is_read || undefined}
      className={item.post_id ? "cursor-pointer" : ""}
      onClick={(event) => {
        if (
          !item.post_id ||
          event.defaultPrevented ||
          (event.target as Element).closest("a,button,input,textarea,select,[tabindex]") ||
          window.getSelection()?.toString()
        )
          return;
        void navigate({ to: "/home/posts/$postId", params: { postId: item.post_id } });
      }}
    >
      <div className="flex items-start gap-3">
        <div className="relative shrink-0">
          <Link
            to="/home/users/$actorId"
            params={{ actorId: authorId }}
            className="block rounded-full focus-visible:ring-2 focus-visible:ring-ring"
          >
            <ActivityPubAvatar avatar={item.avatar} name={item.display_name} />
          </Link>
          <span className="absolute -right-1 -bottom-1 flex size-5 items-center justify-center rounded-full bg-muted text-muted-foreground ring-2 ring-background">
            {item.kind === "like" ? (
              <span
                role="img"
                aria-label={item.emoji?.name || item.reaction || "❤️"}
                className="flex size-4 items-center justify-center overflow-hidden [&_img]:max-w-full! [&_img]:object-contain"
                data-notification-reaction
              >
                {item.emoji ? (
                  <ActivityPubEmoji
                    emoji={item.emoji}
                    className="inline-block size-4 align-middle"
                  />
                ) : (
                  <span className="text-base leading-none">{item.reaction || "❤️"}</span>
                )}
              </span>
            ) : (
              <Icon className="size-3" aria-hidden="true" />
            )}
          </span>
        </div>
        <div className="min-w-0 flex-1 space-y-2 sm:space-y-1">
          <div className="flex min-w-0 items-center gap-2">
            <Link
              to="/home/users/$actorId"
              params={{ actorId: authorId }}
              className="min-w-0 flex-1 truncate text-sm font-semibold hover:underline"
            >
              <ActivityPubText text={item.display_name} emojis={item.actor_emojis} />
            </Link>
            <time
              className="shrink-0 text-[11px] text-muted-foreground tabular-nums"
              dateTime={item.created_at}
              title={new Date(item.created_at).toLocaleString(i18n.language)}
            >
              {new Date(item.created_at).toLocaleString(i18n.language, {
                month: "short",
                day: "numeric",
                hour: "2-digit",
                minute: "2-digit",
                hourCycle: "h23",
              })}
            </time>
            <NotificationReadStatus read={item.is_read} />
          </div>
          {item.content ? (
            <div className="mfm-body line-clamp-3 whitespace-pre-wrap break-words text-sm leading-relaxed">
              <ActivityPubContent text={item.content} emojis={item.content_emojis} />
            </div>
          ) : null}
          {item.post_id ? (
            <Link
              to="/home/posts/$postId"
              params={{ postId: item.post_id }}
              aria-label={`${t("home.title")} · ${t("activitypubSettings.post")} — ${label}`}
              className="flex w-fit items-center gap-1.5 text-xs text-muted-foreground hover:underline"
            >
              <HomeIcon className="size-4" aria-hidden="true" />
              {t("home.title")}
            </Link>
          ) : null}
          {item.kind === "follow_request" && item.pending ? (
            <div className="flex flex-wrap gap-2">
              <Button size="sm" disabled={pending} onClick={() => void action(true)}>
                {t("home.approve")}
              </Button>
              <Button
                size="sm"
                variant="outline"
                disabled={pending}
                onClick={() => void action(false)}
              >
                {t("home.reject")}
              </Button>
            </div>
          ) : null}
          {failed ? (
            <p role="alert" className="text-sm text-destructive">
              {t("home.controlsError")}
            </p>
          ) : null}
        </div>
      </div>
    </NotificationCardFrame>
  );
}
