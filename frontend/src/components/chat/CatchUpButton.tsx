import { Link } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";
import { BookOpenCheckIcon } from "lucide-react";

import { useUnread } from "../../hooks/useUnread";

/** 左レールのキャッチアップ入口。未読のある会話数をバッジに出す。 */
export function CatchUpButton() {
  const { t } = useTranslation();
  const { unreadConversationCount } = useUnread();
  return (
    <Link
      to="/catch-up"
      aria-label={
        unreadConversationCount > 0
          ? t("catchUp.navUnread", { count: unreadConversationCount })
          : t("catchUp.title")
      }
      title={t("catchUp.title")}
      className="relative flex size-10 items-center justify-center rounded-xl text-muted-foreground transition-colors outline-none hover:bg-accent hover:text-foreground focus-visible:ring-3 focus-visible:ring-ring/50 data-[status=active]:bg-brand-muted data-[status=active]:text-brand-muted-foreground"
    >
      <BookOpenCheckIcon className="size-5" aria-hidden="true" />
      {unreadConversationCount > 0 ? (
        <span
          aria-hidden="true"
          className="absolute -right-1 -top-1 rounded-full bg-destructive px-1 text-2xs text-white tabular-nums"
        >
          {unreadConversationCount > 99 ? "99+" : unreadConversationCount}
        </span>
      ) : null}
    </Link>
  );
}
