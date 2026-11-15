import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import {
  desktopNotificationSettings,
  desktopSetNotifications,
  type DesktopNotificationSettings,
} from "../../lib/desktop";
import { Select } from "../ui/select";

/**
 * 設定画面の、デスクトップ版の通知のオン・オフ。デスクトップ版では Web Push を使わず、
 * アプリが OS の通知を出す。古い版のアプリ（コマンドがない）では、説明だけを出す。
 */
export function DesktopNotificationSelect() {
  const { t } = useTranslation();
  // undefined: 読み込み中。null: 古い版のアプリで、設定できない。
  const [settings, setSettings] = useState<DesktopNotificationSettings | null | undefined>();
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let cancelled = false;
    void desktopNotificationSettings().then((next) => {
      if (!cancelled) setSettings(next);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  if (settings === null) {
    return <span className="text-sm text-muted-foreground">{t("pwa.desktopValue")}</span>;
  }
  return (
    <div className="w-40">
      <Select
        aria-label={t("pwa.desktopTitle")}
        aria-busy={busy}
        disabled={busy || settings === undefined}
        value={settings?.enabled === false ? "off" : "on"}
        onChange={(event) => {
          const enabled = event.target.value === "on";
          setBusy(true);
          void desktopSetNotifications(enabled)
            .then(() => setSettings({ enabled }))
            .catch(() => toast.error(t("pwa.error")))
            .finally(() => setBusy(false));
        }}
      >
        <option value="on">{t("pwa.on")}</option>
        <option value="off">{t("pwa.off")}</option>
      </Select>
    </div>
  );
}
