import { ApiError, type ApiClient } from "../api/client";
import type { AppConfig } from "../config";
import { deleteDevice, queryDeviceKeys, registerDevice, uploadKeys, ackToDevice } from "./api";
import type { KeyCounts } from "./api";
import type { CryptoWorker } from "./cryptoWorker";
import {
  clearPendingDeviceDeletion,
  markDeviceDeletionPending,
  readPendingDeviceDeletion,
} from "./identity";
import type { ToDeviceRow } from "./protocol";
import { subscribeToDevice, type ToDeviceSubscription } from "./toDeviceShape";

export type E2eeDeviceStatus = "starting" | "ready" | "error" | "stopped";

export interface E2eeDeviceManagerOptions {
  config: AppConfig;
  client: ApiClient;
  worker: CryptoWorker;
  userId: string;
  namespace: string;
  /** 端末名（`display_name`）。本人にだけ見える。 */
  displayName: string;
  subscribe?: typeof subscribeToDevice;
  /** to-device の処理が失敗したときに、もう一度試すまでの時間。 */
  retryDelayMs?: number;
  onStatus?: (status: E2eeDeviceStatus, error?: unknown) => void;
}

/** ログアウトで端末の削除 API を待つ時間。オフラインならすぐ失敗し、次のログインで消し直す。 */
const DELETE_TIMEOUT_MS = 5_000;

function isDeviceGone(error: unknown): boolean {
  // 登録：削除済みの device_id は 409。鍵のアップロードと ack：削除済みの端末は 404。
  return (
    error instanceof ApiError &&
    (error.status === 404 ||
      (error.status === 409 &&
        (error.body as { code?: unknown } | null)?.code === "E2EE_DEVICE_EXISTS"))
  );
}

/**
 * ログイン後の端末の登録と、鍵の補充、to-device の処理（#359）。
 *
 * Web Locks を取れたタブで 1 つだけ動かす（`E2eeProvider`）。
 *
 * 1. 暗号ストアを開く。端末がなければ作る。
 * 2. 登録していなければ `POST /api/e2ee/devices` で登録する（応答をなくしても同じ鍵で送り直す）。
 * 3. サーバーのワンタイム鍵が 50 個に足りなければ作って送る。フォールバック鍵が使われたら作り直す。
 * 4. to-device の Shape を購読し、処理したメッセージを ack で消す。Olm セッションを新しく作った
 *    （ワンタイム鍵を使われた）ら、3 をもう一度行う。
 */
export class E2eeDeviceManager {
  private subscription: ToDeviceSubscription | null = null;
  private stopped = false;
  private deviceId: string | null = null;
  private latestRows: ToDeviceRow[] = [];
  private processing: Promise<void> | null = null;
  private dirty = false;
  private retryTimer: ReturnType<typeof setTimeout> | null = null;

  constructor(private readonly options: E2eeDeviceManagerOptions) {}

  get currentDeviceId(): string | null {
    return this.deviceId;
  }

  async start(): Promise<void> {
    const { worker, client, userId, namespace } = this.options;
    this.options.onStatus?.("starting");
    try {
      await retryPendingDeviceDeletion(client, userId);
      let opened = await worker.request({ op: "open", namespace, userId });
      // 登録が途中で失敗しても、ログアウトで消せるように先に覚える。
      this.deviceId = opened.deviceId;
      if (opened.replacedDeviceId) {
        // 保存鍵を失って作り直した。前の端末はもう使えないので消す。
        await this.deleteDeviceOrRemember(opened.replacedDeviceId);
      }
      let counts: KeyCounts;
      try {
        counts = await this.ensureRegistered(opened.registered, opened.deviceId);
      } catch (error) {
        if (!isDeviceGone(error)) throw error;
        // 別の端末から消された。暗号ストアを作り直して、新しい端末として登録する。
        await worker.request({ op: "destroy", namespace });
        opened = await worker.request({ op: "open", namespace, userId });
        this.deviceId = opened.deviceId;
        counts = await this.ensureRegistered(opened.registered, opened.deviceId);
      }
      if (this.stopped) return;
      await this.replenish(counts);
      if (this.stopped) return;
      const subscribe = this.options.subscribe ?? subscribeToDevice;
      this.subscription = subscribe(this.options.config, userId, opened.deviceId, (rows) => {
        this.latestRows = rows;
        this.schedule();
      });
      this.options.onStatus?.("ready");
    } catch (error) {
      if (!this.stopped) this.options.onStatus?.("error", error);
      throw error;
    }
  }

  /**
   * 登録も購読もせずに暗号ストアだけを開く。ログアウトしようとしたタブが、別のタブからロックを
   * 奪って、件数の確認と削除だけを行うときに使う。
   */
  async attach(): Promise<void> {
    const opened = await this.options.worker.request({
      op: "open",
      namespace: this.options.namespace,
      userId: this.options.userId,
    });
    // 登録の応答をなくした端末は、サーバーにはあるのに手元では未登録のまま。どちらでも
    // 削除 API を呼ぶ（冪等で、サーバーになければ 404 になり、消すものがないと分かる）。
    this.deviceId = opened.deviceId;
  }

  private async ensureRegistered(registered: boolean, deviceId: string): Promise<KeyCounts> {
    const { worker, client } = this.options;
    if (registered) {
      // 補充に要る今の数を、空のアップロードで読む。
      return uploadKeys(client, deviceId, null);
    }
    const request = await worker.request({ op: "registrationRequest" });
    const counts = await registerDevice(client, {
      ...request,
      display_name: this.options.displayName,
    });
    await worker.request({ op: "markRegistered" });
    return counts;
  }

  private async replenish(counts: KeyCounts): Promise<void> {
    const deviceId = this.deviceId;
    if (!deviceId) return;
    const upload = await this.options.worker.request({
      op: "prepareKeyUpload",
      oneTimeKeyCount: counts.one_time_key_counts.signed_curve25519 ?? 0,
      hasUnusedFallbackKey: counts.unused_fallback_key_types.includes("signed_curve25519"),
    });
    if (!upload) return;
    await uploadKeys(this.options.client, deviceId, upload);
    await this.options.worker.request({ op: "markKeysPublished" });
  }

  private schedule(): void {
    this.dirty = true;
    if (this.processing || this.stopped) return;
    this.processing = (async () => {
      try {
        while (this.dirty && !this.stopped) {
          this.dirty = false;
          await this.process(this.latestRows);
        }
      } catch {
        // オフラインなどで失敗したら、少し待って残っている行で試し直す。
        if (!this.stopped) {
          this.retryTimer = setTimeout(() => {
            this.retryTimer = null;
            this.schedule();
          }, this.options.retryDelayMs ?? 30_000);
        }
      } finally {
        this.processing = null;
      }
    })();
  }

  private async process(rows: ToDeviceRow[]): Promise<void> {
    const deviceId = this.deviceId;
    if (!deviceId || rows.length === 0) return;
    const { worker, client } = this.options;
    const senders = [...new Set(rows.map((row) => row.sender_user_id))].sort();
    const deviceKeys = await queryDeviceKeys(client, senders);
    if (this.stopped) return;
    const result = await worker.request({ op: "processToDevice", rows, deviceKeys });
    if (result.processed.length > 0) {
      await ackToDevice(client, deviceId, result.processed);
      await worker.request({ op: "acknowledged", ids: result.processed });
    }
    if (result.newOlmSessions > 0 && !this.stopped) {
      await this.replenish(await uploadKeys(client, deviceId, null));
    }
  }

  /** 鍵バックアップにまだ送っていない受信用セッションの数。 */
  pendingBackupCount(): Promise<number> {
    return this.options.worker.request({ op: "pendingBackupCount" });
  }

  /** 購読を止め、暗号ストアを閉じる。 */
  async stop(): Promise<void> {
    this.halt();
    await this.processing?.catch(() => undefined);
    await this.options.worker.request({ op: "close" }).catch(() => undefined);
    this.options.onStatus?.("stopped");
  }

  /**
   * ログアウト：暗号ストアを消してから、端末の削除 API を呼ぶ（docs/e2ee.md の「ログアウトと
   * 端末の削除」）。API が失敗したら `device_id` を残し、同じユーザーの次のログインで消し直す。
   */
  async destroy(): Promise<void> {
    this.halt();
    await this.processing?.catch(() => undefined);
    const deviceId = this.deviceId;
    if (deviceId) markDeviceDeletionPending({ userId: this.options.userId, deviceId });
    await this.options.worker.request({ op: "destroy", namespace: this.options.namespace });
    this.options.onStatus?.("stopped");
    if (deviceId) await this.deleteDeviceOrRemember(deviceId);
  }

  private halt(): void {
    this.stopped = true;
    this.subscription?.unsubscribe();
    this.subscription = null;
    if (this.retryTimer) clearTimeout(this.retryTimer);
    this.retryTimer = null;
  }

  private async deleteDeviceOrRemember(deviceId: string): Promise<void> {
    markDeviceDeletionPending({ userId: this.options.userId, deviceId });
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), DELETE_TIMEOUT_MS);
    try {
      await deleteDevice(this.options.client, deviceId, { signal: controller.signal });
      clearPendingDeviceDeletion();
    } catch (error) {
      // 404 はサーバーに端末がない（登録の前に止まった）。消すものがない。
      if (error instanceof ApiError && error.status === 404) clearPendingDeviceDeletion();
      // それ以外は印を残し、次のログインで消し直す。
    } finally {
      clearTimeout(timer);
    }
  }
}

/** 前のログアウトで消せなかった端末を、同じユーザーのログインで消し直す。 */
export async function retryPendingDeviceDeletion(client: ApiClient, userId: string): Promise<void> {
  const pending = readPendingDeviceDeletion();
  if (!pending || pending.userId !== userId) return;
  try {
    await deleteDevice(client, pending.deviceId);
    clearPendingDeviceDeletion();
  } catch (error) {
    if (error instanceof ApiError && error.status === 404) clearPendingDeviceDeletion();
    // それ以外は次の起動で試し直す。
  }
}
