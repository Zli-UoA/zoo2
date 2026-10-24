import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "@tanstack/react-router";
import { BellIcon } from "lucide-react";
import { toast } from "sonner";

import { useNotifications } from "../../hooks/useNotifications";
import { useActiveChannels, useUsers } from "../../hooks/useRealtime";
import { useLocale } from "../i18n/LocaleProvider";
import { useShell } from "../layout/shell-context";
import { Button } from "../ui/button";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "../ui/dialog";
import { fullTimestamp } from "./messages/messageFormat";

export function NotificationsButton() {
  const { t } = useTranslation();
  const { locale } = useLocale();
  const navigate = useNavigate();
  const shell = useShell();
  const [open, setOpen] = useState(false);
  const [pending, setPending] = useState(false);
  const { notifications, unreadCount, markRead, isLoading, isError } = useNotifications();
  const users = useUsers().data ?? [];
  const channels = useActiveChannels().data ?? [];

  async function read(id?: string) {
    setPending(true);
    try {
      await markRead(id);
    } catch {
      toast.error(t("notifications.readError"));
    } finally {
      setPending(false);
    }
  }

  return (
    <>
      <Button
        variant="ghost"
        size="icon"
        className="relative"
        onClick={() => setOpen(true)}
        aria-label={t("notifications.unread", { count: unreadCount })}
        title={t("notifications.title")}
      >
        <BellIcon />
        {unreadCount > 0 ? (
          <span
            aria-hidden="true"
            className="absolute -right-1 -top-1 rounded-full bg-destructive px-1 text-[10px] text-white tabular-nums"
          >
            {unreadCount > 99 ? "99+" : unreadCount}
          </span>
        ) : null}
      </Button>
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent className="flex max-h-[80dvh] flex-col sm:max-w-lg">
          <DialogHeader>
            <DialogTitle>{t("notifications.title")}</DialogTitle>
            <DialogDescription>
              {t("notifications.unread", { count: unreadCount })}
            </DialogDescription>
          </DialogHeader>
          <Button
            variant="outline"
            size="sm"
            disabled={pending || unreadCount === 0}
            onClick={() => void read()}
          >
            {t("notifications.readAll")}
          </Button>
          <div className="min-h-0 overflow-y-auto" aria-busy={isLoading}>
            {isError ? (
              <p role="alert">{t("notifications.loadError")}</p>
            ) : isLoading ? (
              <p>{t("common.loading")}</p>
            ) : notifications.length === 0 ? (
              <p>{t("notifications.empty")}</p>
            ) : (
              <ul className="space-y-2">
                {notifications.map((notification) => {
                  const actor = users.find((user) => user.id === notification.actor_id);
                  const channel = channels.find((row) => row.id === notification.channel_id);
                  const kind = notification.notification_type;
                  const label = t(
                    kind === "mention"
                      ? "notifications.mention"
                      : kind === "reply"
                        ? "notifications.reply"
                        : "notifications.reaction",
                    { actor: actor?.display_name || actor?.user_name || notification.actor_id },
                  );
                  return (
                    <li
                      key={notification.id}
                      className={`rounded-lg border p-3 ${notification.is_read ? "" : "border-brand/40 bg-brand-muted/30"}`}
                    >
                      <p className="text-sm font-medium">{label}</p>
                      <p className="text-xs text-muted-foreground">
                        {channel ? `#${channel.channel_name}` : t("notifications.unavailable")}
                      </p>
                      <time
                        className="text-xs text-muted-foreground"
                        dateTime={notification.created_at}
                      >
                        {fullTimestamp(notification.created_at, locale)}
                      </time>
                      <div className="mt-2 flex gap-2">
                        <Button
                          size="sm"
                          variant="ghost"
                          disabled={!channel}
                          onClick={() => {
                            if (!notification.is_read) void read(notification.id);
                            setOpen(false);
                            shell.showChat();
                            void navigate({
                              to: "/channels/$channelId",
                              params: { channelId: notification.channel_id },
                              search: { message: notification.message_id },
                            });
                          }}
                        >
                          {t("notifications.openMessage")}
                        </Button>
                        {!notification.is_read ? (
                          <Button
                            size="sm"
                            variant="outline"
                            disabled={pending}
                            onClick={() => void read(notification.id)}
                          >
                            {t("notifications.read")}
                          </Button>
                        ) : null}
                      </div>
                    </li>
                  );
                })}
              </ul>
            )}
          </div>
        </DialogContent>
      </Dialog>
    </>
  );
}
