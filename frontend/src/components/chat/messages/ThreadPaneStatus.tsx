import { useTranslation } from "react-i18next";
import { XIcon } from "lucide-react";
import { Button } from "../../ui/button";
import { PaneHeader } from "../../layout/PaneHeader";

/** 親投稿の取得中・取得失敗でも、選択中のペインと復旧操作を表示する。 */
export function ThreadPaneStatus({
  isError,
  onRetry,
  onClose,
}: {
  isError: boolean;
  onRetry: () => void;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  return (
    <aside
      aria-label={t("threads.title")}
      className="absolute inset-0 z-20 flex min-h-0 min-w-0 flex-col border-l bg-background sm:static sm:z-auto sm:w-80 sm:shrink-0 xl:w-96"
    >
      <PaneHeader className="gap-2">
        <h2 className="min-w-0 flex-1 text-sm font-semibold">{t("threads.title")}</h2>
        <Button variant="ghost" size="icon-sm" aria-label={t("threads.close")} onClick={onClose}>
          <XIcon />
        </Button>
      </PaneHeader>
      <div className="flex flex-col items-start gap-2 p-3">
        <p role={isError ? "alert" : "status"} className="text-xs text-muted-foreground">
          {t(isError ? "messages.loadError" : "threads.loading")}
        </p>
        {isError ? (
          <Button variant="outline" size="sm" onClick={onRetry}>
            {t("offline.retry")}
          </Button>
        ) : null}
      </div>
    </aside>
  );
}
