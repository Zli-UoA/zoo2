import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { toast } from "sonner";

import { isDesktopSession } from "../../lib/desktop";
import { isShellOutdated } from "../../lib/pwa/shell-update";

/** 新しい版を確かめる間隔。 */
export const UPDATE_CHECK_INTERVAL_MS = 15 * 60 * 1000;
/** ウィンドウを前に出したときに確かめる、最短の間隔。 */
export const UPDATE_CHECK_MIN_GAP_MS = 60 * 1000;
/**
 * 開いてから最初に確かめるまでの時間。再読み込みしても、ブラウザーによっては Service Worker が
 * 保存済みの画面を返し、裏で新しい画面を保存する。そのときも、すぐに知らせられるように。
 */
export const UPDATE_CHECK_FIRST_DELAY_MS = 10 * 1000;
const TOAST_ID = "app-update";

/** 新しい版があることを知らせる。読み込み直すかは利用者が決める (入力の途中や通話中に画面を変えない)。 */
export function showAppUpdateToast(
  t: TFunction,
  reload: () => void = () => window.location.reload(),
) {
  toast(t("appUpdate.title"), {
    id: TOAST_ID,
    description: t("appUpdate.description"),
    duration: Infinity,
    action: { label: t("appUpdate.reload"), onClick: reload },
  });
}

/** デスクトップ版の F5 と Ctrl+R (⌘R) で読み込み直す。WebView には、ブラウザーのような再読み込みのキーがない。 */
export function isReloadShortcut(
  event: Pick<KeyboardEvent, "key" | "ctrlKey" | "metaKey" | "altKey">,
): boolean {
  if (event.key === "F5") return true;
  return (event.ctrlKey || event.metaKey) && !event.altKey && event.key.toLowerCase() === "r";
}

/**
 * デプロイされた画面が新しくなったら知らせる。開いたままのタブやデスクトップ版は、起動したときの
 * 確認 (`reloadIfShellUpdated`) のあと、新しい版に気づく手段がないため。
 * 開いた少し後、一定の間隔、ウィンドウを前に出したときに確かめる。
 */
export function AppUpdateNotice() {
  const { t } = useTranslation();

  useEffect(() => {
    let stopped = false;
    let found = false;
    let checking = false;
    let lastCheck = Date.now();

    const check = async () => {
      if (stopped || found || checking || !navigator.onLine) return;
      checking = true;
      lastCheck = Date.now();
      try {
        if ((await isShellOutdated()) && !stopped && !found) {
          found = true;
          showAppUpdateToast(t);
        }
      } finally {
        checking = false;
      }
    };
    const checkIfDue = () => {
      if (document.visibilityState !== "visible") return;
      if (Date.now() - lastCheck >= UPDATE_CHECK_MIN_GAP_MS) void check();
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (!isDesktopSession() || !isReloadShortcut(event)) return;
      event.preventDefault();
      window.location.reload();
    };

    // 裏に回ったタブでは取得しない。表示に戻ったときに `visibilitychange` で確かめる。
    const checkIfVisible = () => {
      if (document.visibilityState === "visible") void check();
    };
    const first = setTimeout(checkIfVisible, UPDATE_CHECK_FIRST_DELAY_MS);
    const timer = setInterval(checkIfVisible, UPDATE_CHECK_INTERVAL_MS);
    document.addEventListener("visibilitychange", checkIfDue);
    window.addEventListener("focus", checkIfDue);
    window.addEventListener("online", checkIfDue);
    window.addEventListener("keydown", onKeyDown);
    return () => {
      stopped = true;
      clearTimeout(first);
      clearInterval(timer);
      document.removeEventListener("visibilitychange", checkIfDue);
      window.removeEventListener("focus", checkIfDue);
      window.removeEventListener("online", checkIfDue);
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [t]);

  return null;
}
