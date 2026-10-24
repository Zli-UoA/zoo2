import { Link } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";
import { BellIcon } from "lucide-react";

import { useNotifications } from "../../hooks/useNotifications";

/** 左レールの通知タブ。未読件数を表示し、通知画面に切り替える。 */
export function NotificationsButton() {
  const { t } = useTranslation();
  const { unreadCount } = useNotifications();
  return (
    <Link
      to="/notifications"
      aria-label={`${t("notifications.title")} (${t("notifications.unread", { count: unreadCount })})`}
      title={t("notifications.title")}
      className="relative flex size-10 items-center justify-center rounded-xl text-muted-foreground transition-colors outline-none hover:bg-accent hover:text-foreground focus-visible:ring-3 focus-visible:ring-ring/50 data-[status=active]:bg-brand-muted data-[status=active]:text-brand-muted-foreground"
    >
      <BellIcon className="size-5" aria-hidden="true" />
      {unreadCount > 0 ? (
        <span
          aria-hidden="true"
          className="absolute -right-1 -top-1 rounded-full bg-destructive px-1 text-[10px] text-white tabular-nums"
        >
          {unreadCount > 99 ? "99+" : unreadCount}
        </span>
      ) : null}
    </Link>
  );
}
