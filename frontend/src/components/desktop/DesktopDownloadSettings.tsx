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
 * 説明だけを出す。
 */
export function DesktopDownloadSettings() {
  const { t } = useTranslation();
  // undefined: 読み込み中。null: 古い版のアプリで、設定できない。
  const [settings, setSettings] = useState<DownloadSettings | null | undefined>();
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let cancelled = false;
    void desktopDownloadSettings().then((next) => {
      if (!cancelled) setSettings(next);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  function save(update: DesktopDownloadSettingsUpdate) {
    setBusy(true);
    return desktopSetDownloadSettings(update)
      .then((saved) => setSettings(saved))
      .catch(() => toast.error(t("downloads.error")))
      .finally(() => setBusy(false));
  }

  if (settings === null) {
    return <span className="text-sm text-muted-foreground">{t("downloads.desktopValue")}</span>;
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
        <div className="flex min-w-0 items-center gap-2">
          <span
            className="max-w-56 truncate text-xs text-muted-foreground"
            title={directory ?? t("downloads.defaultDirectory")}
          >
            {directory ?? t("downloads.defaultDirectory")}
          </span>
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
      )}
    </div>
  );
}
