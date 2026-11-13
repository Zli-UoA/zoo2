import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
  type ReactNode,
} from "react";

import { createConfiguredApiClient } from "../../lib/api/client";
import type { AppConfig } from "../../lib/config";
import { getCurrentUserStore } from "../../lib/currentUser";
import { startCryptoWorker, type CryptoWorker } from "../../lib/e2ee/cryptoWorker";
import { E2eeDeviceManager } from "../../lib/e2ee/deviceManager";
import { deviceDisplayName } from "../../lib/e2ee/displayName";
import { E2eeContext, type E2eeContextValue } from "../../lib/e2ee/e2ee-context";
import {
  clearPendingCryptoStoreDeletion,
  cryptoLockName,
  markCryptoStoreDeletionPending,
  readPendingCryptoStoreDeletion,
} from "../../lib/e2ee/identity";
import { supportsCryptoStore } from "../../lib/e2ee/support";
import { useOfflineStatus } from "../../lib/offline/offline-context";

interface ActiveStore {
  worker: CryptoWorker;
  manager: E2eeDeviceManager;
}

/** 前のログアウトで消せなかった暗号ストアを消し直す（使っているタブがなければ）。 */
async function retryPendingStoreDeletion(worker: CryptoWorker): Promise<void> {
  const pending = readPendingCryptoStoreDeletion();
  if (!pending) return;
  await navigator.locks.request(cryptoLockName(pending), { ifAvailable: true }, async (lock) => {
    if (!lock) return;
    await worker.request({ op: "destroy", namespace: pending });
    clearPendingCryptoStoreDeletion();
  });
}

/**
 * ログイン後に暗号ストアを開き、端末を登録して、to-device を処理する（#359）。
 *
 * 暗号ストアを開くのは、Web Locks（`zoo-crypto:<DB 名>`）を取れた 1 つのタブだけにする
 * （docs/e2ee.md の「暗号ストア」）。他のタブはロックを待ち、開いているタブが閉じたら引き継ぐ。
 * ログアウトしたタブは、ロックを奪って（`steal`）件数の確認と削除を行う。
 */
export function E2eeProvider({ config, children }: { config: AppConfig; children: ReactNode }) {
  const { namespace, isOnline } = useOfflineStatus();
  const userStore = getCurrentUserStore(config);
  const subscribe = useCallback(
    (listener: () => void) => userStore.subscribe(listener),
    [userStore],
  );
  const getUserState = useCallback(() => userStore.getState(), [userStore]);
  const { currentUserId } = useSyncExternalStore(subscribe, getUserState, getUserState);
  const userId = config.devUserId || currentUserId;
  const supported = useMemo(() => supportsCryptoStore(), []);
  const [status, setStatus] = useState<E2eeContextValue["status"]>(
    supported ? "idle" : "unsupported",
  );
  const [deviceId, setDeviceId] = useState<string | null>(null);
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  const leader = useRef<ActiveStore | null>(null);
  const takeover = useRef<Promise<ActiveStore> | null>(null);

  const createManager = useCallback(
    (worker: CryptoWorker, userId: string) =>
      new E2eeDeviceManager({
        config,
        client,
        worker,
        userId,
        namespace,
        displayName: deviceDisplayName(),
        onStatus: (next) => {
          setStatus(next);
          if (next === "stopped") setDeviceId(null);
        },
      }),
    [client, config, namespace],
  );

  useEffect(() => {
    if (!supported || !userId || !isOnline) return;
    const controller = new AbortController();
    let release: () => void = () => {};
    const held = new Promise<void>((resolve) => {
      release = resolve;
    });
    navigator.locks
      .request(cryptoLockName(namespace), { signal: controller.signal }, async () => {
        const worker = startCryptoWorker();
        const manager = createManager(worker, userId);
        const active = { worker, manager };
        leader.current = active;
        try {
          await retryPendingStoreDeletion(worker).catch(() => undefined);
          await manager
            .start()
            .then(() => setDeviceId(manager.currentDeviceId))
            .catch(() => undefined);
          await held;
        } finally {
          if (leader.current === active) leader.current = null;
          await manager.stop().catch(() => undefined);
          worker.terminate();
        }
      })
      // 中止（アンマウント）と、ログアウトする別のタブに奪われたときは、持っていた分を片付ける。
      .catch(() => release());
    return () => {
      controller.abort();
      release();
    };
  }, [createManager, isOnline, namespace, supported, userId]);

  /** ログアウトに使う暗号ストア。このタブが開いていなければ、ロックを奪って開く。 */
  const storeForLogout = useCallback(async (): Promise<ActiveStore | null> => {
    if (!supported || !userId) return null;
    if (leader.current) return leader.current;
    takeover.current ??= new Promise<ActiveStore>((resolve, reject) => {
      void navigator.locks
        .request(cryptoLockName(namespace), { steal: true }, async () => {
          const worker = startCryptoWorker();
          const manager = createManager(worker, userId);
          try {
            await manager.attach();
            resolve({ worker, manager });
          } catch (error) {
            worker.terminate();
            reject(error);
            return;
          }
          // ログアウトはページを離れるので、ロックはそのときまで持つ。
          await new Promise(() => {});
        })
        .catch(reject);
    }).catch((error: unknown) => {
      takeover.current = null;
      throw error;
    });
    return takeover.current;
  }, [createManager, namespace, supported, userId]);

  const value = useMemo<E2eeContextValue>(
    () => ({
      status,
      deviceId,
      pendingBackupCount: async () => {
        const store = await storeForLogout();
        return store ? store.manager.pendingBackupCount() : 0;
      },
      clearForLogout: async () => {
        if (!supported || !userId) return;
        // 消し切れなければ、次の起動で消し直す。
        markCryptoStoreDeletionPending(namespace);
        const store = await storeForLogout();
        if (!store) return;
        await store.manager.destroy();
        clearPendingCryptoStoreDeletion();
        store.worker.terminate();
      },
    }),
    [deviceId, namespace, status, storeForLogout, supported, userId],
  );

  return <E2eeContext.Provider value={value}>{children}</E2eeContext.Provider>;
}
