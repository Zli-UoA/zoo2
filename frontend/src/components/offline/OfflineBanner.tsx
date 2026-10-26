import { useTranslation } from "react-i18next";
import { CloudOffIcon } from "lucide-react";

import { useOfflineStatus } from "../../lib/offline/offline-context";
import { Button } from "../ui/button";

/**
 * 画面全体に出すオフライン・保存失敗の帯。表示中もレイアウトを押し下げず、
 * 上部に重ねる。オンラインに戻ると自動で消える。
 */
export function OfflineBanner() {
  const { t } = useTranslation();
  const { isOnline, persistenceStatus, persistenceError, retryPersistence } = useOfflineStatus();

  if (isOnline && persistenceStatus !== "error") {
    return null;
  }

  const offline = !isOnline;
  const title = offline ? t("offline.bannerTitle") : t("offline.persistenceErrorTitle");
  const description = offline
    ? t("offline.bannerDescription")
    : (persistenceError ?? t("offline.persistenceErrorDescription"));

  return (
    <div
      role="status"
      className="pointer-events-none fixed inset-x-0 top-2 z-50 flex justify-center px-3"
    >
      <div className="pointer-events-auto flex max-w-xl items-start gap-2 rounded-lg border bg-popover px-3 py-2 text-popover-foreground shadow-md">
        <CloudOffIcon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
        <div className="min-w-0 flex-1">
          <p className="text-sm font-medium">{title}</p>
          <p className="text-xs break-words text-muted-foreground">{description}</p>
        </div>
        <Button
          variant="outline"
          size="sm"
          onClick={() => {
            if (offline) {
              window.location.reload();
            } else {
              retryPersistence();
            }
          }}
        >
          {t("offline.retry")}
        </Button>
      </div>
    </div>
  );
}
