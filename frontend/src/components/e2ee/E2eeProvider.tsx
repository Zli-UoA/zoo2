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
import { loadE2eeStatus } from "../../lib/e2ee/setupController";
import { supportsCryptoStore } from "../../lib/e2ee/support";
import { readLastOfflineUserId } from "../../lib/offline/identity";
import { useOfflineStatus } from "../../lib/offline/offline-context";

interface ActiveStore {
  worker: CryptoWorker;
  manager: E2eeDeviceManager;
}

/** ログアウトを取りやめてロックを返したことを、ほかのタブに知らせる。 */
const LOCK_CHANNEL = "zoo-crypto-lock";
/** 起動に失敗したときに試し直す間隔（最初と最大）。 */
const START_RETRY_MS = 5_000;
const START_RETRY_MAX_MS = 300_000;

/**
 * 前のログアウトで消せなかった暗号ストアを消し直す。
 *
 * `namespace` は今ロックを持っている暗号ストア。同じなら、そのロックの下で消す（もう一度取ろうと
 * すると、自分のロックに阻まれる）。別の名前空間なら、使っているタブがないときだけ消す。
 */
async function retryPendingStoreDeletion(worker: CryptoWorker, namespace: string): Promise<void> {
  const pending = readPendingCryptoStoreDeletion();
  if (!pending) return;
  const destroy = async () => {
    await worker.request({ op: "destroy", namespace: pending });
    clearPendingCryptoStoreDeletion();
  };
  if (pending === namespace) {
    await destroy();
    return;
  }
  await navigator.locks.request(cryptoLockName(pending), { ifAvailable: true }, async (lock) => {
    if (lock) await destroy();
  });
}

function wait(ms: number, until: Promise<void>): Promise<void> {
  return Promise.race([until, new Promise<void>((resolve) => setTimeout(resolve, ms))]);
}

/**
 * ログイン後に暗号ストアを開き、端末を登録して、to-device を処理する（#359）。
 *
 * 暗号ストアを開くのは、Web Locks（`zoo-crypto:<DB 名>`）を取れた 1 つのタブだけにする
 * （docs/e2ee.md の「暗号ストア」）。他のタブはロックを待ち、開いているタブが閉じたら引き継ぐ。
 * ログアウトしたタブは、ロックを奪って（`steal`）件数の確認と削除を行う。取りやめたらロックを返し、
 * 奪われたタブはもう一度ロックを待つ。
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
  const [readyWorker, setReadyWorker] = useState<CryptoWorker | null>(null);
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  const leader = useRef<ActiveStore | null>(null);
  const takeover = useRef<{ store: Promise<ActiveStore>; release: () => void } | null>(null);

  const createManager = useCallback(
    (worker: CryptoWorker, managerUserId: string) =>
      new E2eeDeviceManager({
        config,
        client,
        worker,
        userId: managerUserId,
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
    const channel =
      typeof BroadcastChannel === "undefined" ? null : new BroadcastChannel(LOCK_CHANNEL);
    let requesting = false;
    let releaseCurrent: () => void = () => {};

    const lead = () => {
      requesting = true;
      let released = false;
      let resolveHeld: () => void = () => {};
      const held = new Promise<void>((resolve) => {
        resolveHeld = resolve;
      });
      const release = () => {
        released = true;
        resolveHeld();
      };
      releaseCurrent = release;
      navigator.locks
        .request(cryptoLockName(namespace), { signal: controller.signal }, async () => {
          const worker = startCryptoWorker();
          let active: ActiveStore | null = null;
          try {
            await retryPendingStoreDeletion(worker, namespace).catch(() => undefined);
            // 登録や鍵の補充が一時的に失敗しても（503、通信の失敗）、間隔をあけて試し直す。
            let delay = START_RETRY_MS;
            while (!released) {
              const manager = createManager(worker, userId);
              active = { worker, manager };
              leader.current = active;
              try {
                await manager.start();
                setDeviceId(manager.currentDeviceId);
                setReadyWorker(worker);
                // 解錠済みの端末は、起動時に自分の端末への署名を確かめ、なければ署名し直す
                // （docs/e2ee.md の「初回設定と解錠の流れ」。止まった初回設定は暗号化タブで案内する）。
                void loadE2eeStatus({ worker, client, userId, rpId: null }).catch(() => undefined);
                break;
              } catch {
                await manager.stop().catch(() => undefined);
                await wait(delay, held);
                delay = Math.min(delay * 2, START_RETRY_MAX_MS);
              }
            }
            await held;
          } finally {
            setReadyWorker((current) => (current === worker ? null : current));
            if (leader.current === active) leader.current = null;
            await active?.manager.stop().catch(() => undefined);
            worker.terminate();
          }
        })
        // 中止（アンマウント）と、ログアウトする別のタブに奪われたときは、持っていた分を片付ける。
        .catch(() => release())
        .finally(() => {
          requesting = false;
        });
    };

    lead();
    // ログアウトを取りやめたタブがロックを返したら、奪われていたタブはもう一度ロックを待つ。
    if (channel) {
      channel.onmessage = ({ data }: MessageEvent<{ type?: string; namespace?: string }>) => {
        if (data?.type === "released" && data.namespace === namespace && !requesting) lead();
      };
    }
    return () => {
      controller.abort();
      releaseCurrent();
      channel?.close();
    };
  }, [client, createManager, isOnline, namespace, supported, userId]);

  /**
   * ログアウトに使う暗号ストア。このタブが開いていなければ、ロックを奪って開く。
   *
   * オフラインで起動して今のユーザーを確かめられないときも、保存済みのユーザー（オフライン DB の
   * 名前空間と同じもの）の暗号ストアを開く。
   */
  const storeForLogout = useCallback(async (): Promise<ActiveStore | null> => {
    const logoutUserId = userId || readLastOfflineUserId();
    if (!supported || !logoutUserId) return null;
    if (leader.current) return leader.current;
    if (!takeover.current) {
      let resolveHold: () => void = () => {};
      const hold = new Promise<void>((resolve) => {
        resolveHold = resolve;
      });
      const store = new Promise<ActiveStore>((resolve, reject) => {
        void navigator.locks
          .request(cryptoLockName(namespace), { steal: true }, async () => {
            const worker = startCryptoWorker();
            try {
              const manager = createManager(worker, logoutUserId);
              await manager.attach();
              resolve({ worker, manager });
              // ログアウトはページを離れるので、取りやめる（`release`）までロックを持つ。
              await hold;
            } catch (error) {
              reject(error);
            } finally {
              worker.terminate();
            }
          })
          .catch(reject);
      });
      const entry = { store, release: resolveHold };
      takeover.current = entry;
      store.catch(() => {
        if (takeover.current === entry) takeover.current = null;
        resolveHold();
      });
    }
    return takeover.current.store;
  }, [createManager, namespace, supported, userId]);

  const value = useMemo<E2eeContextValue>(
    () => ({
      status,
      deviceId,
      worker: status === "ready" ? readyWorker : null,
      userId: userId || null,
      pendingBackupCount: async () => {
        const store = await storeForLogout();
        if (!store) return 0;
        // 数えたあとで鍵が増えないよう、to-device の処理を止めてから数える。
        await store.manager.suspend();
        return store.manager.pendingBackupCount();
      },
      cancelLogout: () => {
        // このタブが開いていたなら、止めていた to-device の処理に戻る。
        leader.current?.manager.resume();
        const entry = takeover.current;
        if (!entry) return;
        takeover.current = null;
        entry.release();
        const channel =
          typeof BroadcastChannel === "undefined" ? null : new BroadcastChannel(LOCK_CHANNEL);
        channel?.postMessage({ type: "released", namespace });
        channel?.close();
      },
      clearForLogout: async () => {
        if (!supported || namespace === "anonymous") return;
        // 消し切れなければ（開けない、ユーザーが分からない）、次の起動で消し直す。
        markCryptoStoreDeletionPending(namespace);
        const store = await storeForLogout();
        if (!store) return;
        await store.manager.destroy();
        clearPendingCryptoStoreDeletion();
        store.worker.terminate();
      },
    }),
    [deviceId, namespace, readyWorker, status, storeForLogout, supported, userId],
  );

  return <E2eeContext.Provider value={value}>{children}</E2eeContext.Provider>;
}
