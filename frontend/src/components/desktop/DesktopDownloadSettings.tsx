import { Switch } from "@base-ui/react/switch";
import { FolderIcon } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import {
  desktopDownloadSettings,
  desktopPickDownloadDirectory,
  desktopSetDownloadSettings,
  type DesktopDownloadSettings as DownloadSettings,
  type DesktopDownloadSettingsUpdate,
} from "../../lib/desktop";
import { Button } from "../ui/button";

/**
 * 設定画面の、デスクトップ版の添付の保存先。既定は保存のたびに OS の「名前を付けて保存」を出す。
 * オフにすると、選んだフォルダーへ確認なしで保存する。
 *
 * 保存先を決めるのは Rust 側だけで、ここは表示と選択だけを行う。古い版のアプリ（コマンドがない）では、
 * アプリが決めた保存先に保存されることと、更新が必要なことを出す。設定を読めなかっただけのときは、
 * 古い版と決めつけずに再読み込みを出す。
 */
export function DesktopDownloadSettings() {
  const { t } = useTranslation();
  // undefined: 読み込み中。null: 古い版のアプリで、設定できない。
  const [settings, setSettings] = useState<DownloadSettings | null | undefined>();
  // 設定を読めなかっただけのとき。古い版と決めつけず、再読み込みを出す。
  const [loadFailed, setLoadFailed] = useState(false);
  const [reload, setReload] = useState(0);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let cancelled = false;
    setLoadFailed(false);
    void desktopDownloadSettings()
      .then((next) => {
        if (!cancelled) setSettings(next);
      })
      .catch(() => {
        if (!cancelled) setLoadFailed(true);
      });
    return () => {
      cancelled = true;
    };
  }, [reload]);

  function save(update: DesktopDownloadSettingsUpdate) {
    setBusy(true);
    return desktopSetDownloadSettings(update)
      .then((saved) => setSettings(saved))
      .catch(() => toast.error(t("downloads.error")))
      .finally(() => setBusy(false));
  }

  if (loadFailed) {
    return (
      <div className="flex flex-col items-end gap-2 text-right">
        <span className="max-w-56 text-xs text-muted-foreground">{t("downloads.loadError")}</span>
        <Button type="button" variant="outline" size="sm" onClick={() => setReload((n) => n + 1)}>
          {t("downloads.retry")}
        </Button>
      </div>
    );
  }

  if (settings === null) {
    return (
      <div className="flex flex-col items-end gap-1 text-right">
        <span className="text-sm text-muted-foreground">{t("downloads.olderAppValue")}</span>
        <span className="max-w-56 text-xs text-muted-foreground">
          {t("downloads.olderAppDescription")}
        </span>
      </div>
    );
  }

  const askEveryTime = settings?.ask_every_time ?? true;
  const directory = settings?.directory ?? null;

  return (
    <div className="flex flex-col items-end gap-2">
      <div className="flex items-center gap-2">
        <span className="text-xs text-muted-foreground">{t("downloads.askEveryTime")}</span>
        <Switch.Root
          checked={askEveryTime}
          aria-label={t("downloads.askEveryTime")}
          disabled={busy || settings === undefined}
          onCheckedChange={(checked) => {
            if (settings) void save({ ask_every_time: checked });
          }}
          className="relative inline-flex h-6 w-11 shrink-0 cursor-pointer items-center rounded-full bg-input outline-none transition-colors focus-visible:ring-3 focus-visible:ring-ring/50 disabled:cursor-not-allowed disabled:opacity-50 data-[checked]:bg-primary"
        >
          <Switch.Thumb className="block size-5 translate-x-0.5 rounded-full bg-background shadow transition-transform data-[checked]:translate-x-[22px]" />
        </Switch.Root>
      </div>
      {askEveryTime ? null : (
        <div className="flex w-72 max-w-full flex-col items-end gap-2">
          <span
            className="w-full truncate text-right text-xs text-muted-foreground"
            title={directory ?? t("downloads.defaultDirectory")}
          >
            {directory ?? t("downloads.defaultDirectory")}
          </span>
          <div className="flex flex-wrap justify-end gap-2">
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={busy || settings === undefined}
              onClick={() => {
                setBusy(true);
                void desktopPickDownloadDirectory()
                  .then((saved) => {
                    if (saved) setSettings(saved);
                  })
                  .catch(() => toast.error(t("downloads.error")))
                  .finally(() => setBusy(false));
              }}
            >
              <FolderIcon className="size-3.5" aria-hidden="true" />
              {t("downloads.chooseFolder")}
            </Button>
            {directory ? (
              <Button
                type="button"
                variant="ghost"
                size="sm"
                disabled={busy || settings === undefined}
                onClick={() => void save({ ask_every_time: askEveryTime, reset_directory: true })}
              >
                {t("downloads.resetDirectory")}
              </Button>
            ) : null}
          </div>
        </div>
      )}
    </div>
  );
}
