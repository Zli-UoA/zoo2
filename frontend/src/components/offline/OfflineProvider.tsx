import {
  useCallback,
  useEffect,
  useMemo,
  useState,
  useSyncExternalStore,
  type ReactNode,
} from "react";

import { useTranslation } from "react-i18next";

import type { AppConfig } from "../../lib/config";
import { getCurrentUserStore } from "../../lib/currentUser";
import { getCollections, type CollectionsBundle } from "../../lib/electric/collections";
import { CollectionsProvider } from "../../lib/electric/collections-context";
import {
  clearPendingOfflineDeletion,
  forgetOfflineUserId,
  markOfflineDeletionPending,
  rememberOfflineUserId,
  resolveOfflineNamespace,
} from "../../lib/offline/identity";
import { useOnlineStatus } from "../../lib/offline/online";
import {
  OfflineContext,
  type OfflineContextValue,
  type OfflinePersistenceStatus,
} from "../../lib/offline/offline-context";
import {
  OfflinePersistenceUnavailableError,
  openOfflinePersistence,
  removeOfflineDatabase,
  retryPendingOfflineDeletion,
} from "../../lib/offline/persistence";
import { prepareOfflineWorker } from "../../lib/pwa/worker-registration";
import { ShellSkeleton } from "../layout/ShellSkeleton";

interface ReadyState {
  key: string;
  bundle: CollectionsBundle;
  persistenceStatus: OfflinePersistenceStatus;
  persistenceError: string | null;
}

function describeError(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/**
 * ユーザー単位の OPFS 永続化を初期化してから、その永続化を使う
 * Electric コレクションを子へ渡す。
 *
 * DB のオープンに失敗してもアプリは動かす（メモリのみにフォールバック）。
 * オンラインの OIDC 起動は現在ユーザーの解決を待ってから名前空間を選ぶ。
 * オフラインは保存済みの名前空間で起動する。初期化途中の再マウントで
 * 入力中のフォームを消さず、別ユーザーのキャッシュも混ぜない。
 */
export function OfflineProvider({ config, children }: { config: AppConfig; children: ReactNode }) {
  const { t } = useTranslation();
  const isOnline = useOnlineStatus();
  const userStore = getCurrentUserStore(config);
  const subscribe = useCallback(
    (listener: () => void) => userStore.subscribe(listener),
    [userStore],
  );
  const getUserState = useCallback(() => userStore.getState(), [userStore]);
  const { currentUserId, isLoading: currentUserLoading } = useSyncExternalStore(
    subscribe,
    getUserState,
    getUserState,
  );
  // オフラインのキャッシュ表示は /me の完了に依存させない。
  const waitingForUser = Boolean(config.oidc && isOnline && currentUserLoading);
  const userUnavailable = Boolean(config.oidc && isOnline && !currentUserLoading && !currentUserId);
  const [retryToken, setRetryToken] = useState(0);
  const [ready, setReady] = useState<ReadyState | null>(null);

  useEffect(() => {
    void userStore.ensureLoaded();
  }, [userStore]);

  // 初回ロード時に Service Worker が制御していなかった assets（フォントや
  // 動的 import の OPFS worker など）も、後から SW のキャッシュへ送る。
  useEffect(() => {
    if (typeof navigator === "undefined" || !("serviceWorker" in navigator)) {
      return;
    }
    let cancelled = false;
    const send = () => {
      if (cancelled) return;
      const controller = navigator.serviceWorker.controller;
      if (!controller) return;
      const urls = new Set<string>();
      for (const entry of performance.getEntriesByType("resource")) {
        try {
          const url = new URL(entry.name);
          if (url.origin === window.location.origin && url.pathname.startsWith("/assets/")) {
            urls.add(url.pathname);
          }
        } catch {
          /* Ignore malformed resource timings. */
        }
      }
      if (urls.size > 0) {
        controller.postMessage({ type: "CACHE_ASSETS", urls: [...urls] });
      }
    };
    const timers = [window.setTimeout(send, 1_000), window.setTimeout(send, 4_000)];
    navigator.serviceWorker.addEventListener("controllerchange", send);
    window.addEventListener("load", send);
    void navigator.serviceWorker.ready.then(send).catch(() => {});
    return () => {
      cancelled = true;
      for (const timer of timers) window.clearTimeout(timer);
      navigator.serviceWorker.removeEventListener("controllerchange", send);
      window.removeEventListener("load", send);
    };
  }, []);

  // OPFS 接続は pagehide で閉じられる。bfcache から復元された場合は
  // 古い接続を使い続けず、アプリを読み直して新しい接続を作る。
  useEffect(() => {
    const handlePageShow = (event: PageTransitionEvent) => {
      if (event.persisted) window.location.reload();
    };
    window.addEventListener("pageshow", handlePageShow);
    return () => window.removeEventListener("pageshow", handlePageShow);
  }, []);

  useEffect(() => {
    if (currentUserId) {
      rememberOfflineUserId(currentUserId);
    }
  }, [currentUserId]);

  // オンラインで本人を確認できない場合は、前ユーザーのキャッシュを開かない。
  const namespace =
    config.oidc && isOnline && !currentUserId
      ? "anonymous"
      : resolveOfflineNamespace({ devUserId: config.devUserId, currentUserId });
  const key = `${namespace}:${retryToken}`;

  useEffect(() => {
    let cancelled = false;
    setReady(null);
    // Do not mount an anonymous shell that will discard open forms when /me resolves.
    if (waitingForUser || userUnavailable) return;
    void (async () => {
      let persistence = null;
      let persistenceStatus: OfflinePersistenceStatus = "unavailable";
      let persistenceError: string | null = null;
      await retryPendingOfflineDeletion();
      // The Worker response may still be cached with the previous deployment's CSP.
      // Upgrade the controlling SW before the first SQLite open, not after rendering the shell.
      try {
        await prepareOfflineWorker();
      } catch {
        // SW setup is optional: OPFS may still work directly, including offline.
      }
      if (cancelled) return;
      try {
        const handle = await openOfflinePersistence(namespace);
        if (cancelled) return;
        persistence = handle.persistence;
        persistenceStatus = "ready";
      } catch (error) {
        if (cancelled) return;
        persistenceError = describeError(error);
        persistenceStatus =
          error instanceof OfflinePersistenceUnavailableError ? "unavailable" : "error";
      }
      const bundle = getCollections(config, { persistence, namespace });
      if (!cancelled) {
        setReady({ key, bundle, persistenceStatus, persistenceError });
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [config, namespace, retryToken, key, waitingForUser, userUnavailable]);

  const retryPersistence = useCallback(() => setRetryToken((token) => token + 1), []);
  const clearUserData = useCallback(async () => {
    // 削除に失敗しても、次のオフライン起動が前のユーザーの DB を選ばないよう
    // 先に lastUser を消す。未完了の削除は次の起動で再試行する。
    forgetOfflineUserId(currentUserId);
    markOfflineDeletionPending(namespace);
    await removeOfflineDatabase(namespace);
    clearPendingOfflineDeletion();
  }, [currentUserId, namespace]);

  const value = useMemo<OfflineContextValue>(
    () => ({
      isOnline,
      persistenceStatus: ready?.persistenceStatus ?? "loading",
      persistenceError: ready?.persistenceError ?? null,
      namespace,
      retryPersistence,
      clearUserData,
    }),
    [clearUserData, isOnline, namespace, ready, retryPersistence],
  );

  if (userUnavailable) {
    return (
      <div role="alert" className="flex h-dvh flex-col items-center justify-center gap-3 p-6">
        <p>{t("offline.userErrorTitle")}</p>
        <button type="button" onClick={() => void userStore.ensureLoaded()}>
          {t("offline.retry")}
        </button>
      </div>
    );
  }

  if (waitingForUser || !ready || ready.key !== key) {
    return <ShellSkeleton />;
  }

  return (
    <OfflineContext.Provider value={value}>
      <CollectionsProvider value={ready.bundle}>{children}</CollectionsProvider>
    </OfflineContext.Provider>
  );
}
