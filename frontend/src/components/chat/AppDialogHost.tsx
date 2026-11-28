import { lazy, Suspense, useEffect, useState } from "react";

// ダイアログ本体は開いたときだけ読み込む。イベントの購読はここで先に始め、
// チャンク取得中に来た要求も取りこぼさない。
const LazyAppsDialog = lazy(() =>
  import("./AppsDialog").then((module) => ({ default: module.AppsDialog })),
);

/**
 * 左レールに常時マウントする、アプリ／メッセージ操作ダイアログの入口。
 * 重い `AppsDialog` は開く要求を受けてから読み込む。
 */
export function AppDialogHost() {
  const [open, setOpen] = useState(false);
  const [appId, setAppId] = useState<string>();
  const [messageId, setMessageId] = useState<string>();

  useEffect(() => {
    const openApp = (event: Event) => {
      setMessageId(undefined);
      setAppId((event as CustomEvent<string>).detail);
      setOpen(true);
    };
    const openMessageActions = (event: Event) => {
      setAppId(undefined);
      setMessageId((event as CustomEvent<string>).detail);
      setOpen(true);
    };
    window.addEventListener("zoo:open-app", openApp);
    window.addEventListener("zoo:message-actions", openMessageActions);
    return () => {
      window.removeEventListener("zoo:open-app", openApp);
      window.removeEventListener("zoo:message-actions", openMessageActions);
    };
  }, []);

  if (!open) return null;
  return (
    <Suspense fallback={null}>
      <LazyAppsDialog initialId={appId} messageId={messageId} onClose={() => setOpen(false)} />
    </Suspense>
  );
}
