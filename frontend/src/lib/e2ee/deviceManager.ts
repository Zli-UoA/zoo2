import { ApiError, type ApiClient } from "../api/client";
import type { AppConfig } from "../config";
import { deleteDevice, queryDeviceKeys, registerDevice, uploadKeys, ackToDevice } from "./api";
import type { KeyCounts } from "./api";
import type { CryptoWorker } from "./cryptoWorker";
import {
  clearPendingDeviceDeletion,
  markDeviceDeletionPending,
  readPendingDeviceDeletions,
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
  /** ログアウトで、処理中のもの（登録の要求など）を待つ時間。 */
  settleTimeoutMs?: number;
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
 *    （ワンタイム鍵を使われた）ら、3 をもう一度行う。3 が失敗したら、成功するまで試し直す。
 */
export class E2eeDeviceManager {
  private subscription: ToDeviceSubscription | null = null;
  private stopped = false;
  private suspended = false;
  private deviceId: string | null = null;
  private latestRows: ToDeviceRow[] = [];
  private processing: Promise<void> | null = null;
  private starting: Promise<void> | null = null;
  private dirty = false;
  /** 鍵を補充しなければならない（新しい Olm セッションを作った）。補充できるまで残す。 */
  private needsReplenish = false;
  private retryTimer: ReturnType<typeof setTimeout> | null = null;
  /** 止めたら、通信中の要求（登録を除く）を中止する。 */
  private readonly abort = new AbortController();
  /** to-device の処理の要求。ログアウトの確認で止めるときにも中止し、再開したら作り直す。 */
  private processAbort = new AbortController();
  /** 登録の要求の結果がまだ分からない。分からないうちは、ログアウトで端末を消さない。 */
  private registering = false;

  constructor(private readonly options: E2eeDeviceManagerOptions) {}

  get currentDeviceId(): string | null {
    return this.deviceId;
  }

  start(): Promise<void> {
    this.starting = this.run().finally(() => {
      this.starting = null;
    });
    return this.starting;
  }

  private async run(): Promise<void> {
    const { worker, client, userId, namespace } = this.options;
    const signal = this.abort.signal;
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
        if (!isDeviceGone(error) || this.stopped) throw error;
        // 別の端末から消された。暗号ストアを作り直して、新しい端末として登録する。
        await worker.request({ op: "destroy", namespace });
        opened = await worker.request({ op: "open", namespace, userId });
        this.deviceId = opened.deviceId;
        counts = await this.ensureRegistered(opened.registered, opened.deviceId);
      }
      if (this.stopped) return;
      await this.replenish(counts);
      if (this.stopped) return;
      signal.throwIfAborted();
      this.subscribe();
      this.options.onStatus?.("ready");
    } catch (error) {
      if (!this.stopped) this.options.onStatus?.("error", error);
      throw error;
    }
  }

  private subscribe(): void {
    const deviceId = this.deviceId;
    if (!deviceId || this.subscription || this.stopped || this.suspended) return;
    const subscribe = this.options.subscribe ?? subscribeToDevice;
    this.subscription = subscribe(this.options.config, this.options.userId, deviceId, (rows) => {
      this.latestRows = rows;
      this.schedule();
    });
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
      return uploadKeys(client, deviceId, null, this.abort.signal);
    }
    const request = await worker.request({ op: "registrationRequest" });
    // 登録は中止しない。中止しても、サーバーが受け取っていれば端末はできる。ログアウトは、
    // この要求の結果を待ってから端末を消す（`destroy`）。
    this.registering = true;
    let counts: KeyCounts;
    try {
      counts = await registerDevice(client, {
        ...request,
        display_name: this.options.displayName,
      });
    } catch (error) {
      // 失敗が分かった（応答をなくしたなら、サーバーにはあるかもしれないが、削除は冪等）。
      this.registering = false;
      throw error;
    }
    this.registering = false;
    await worker.request({ op: "markRegistered" });
    return counts;
  }

  private async replenish(counts: KeyCounts, signal = this.abort.signal): Promise<void> {
    const deviceId = this.deviceId;
    if (!deviceId) return;
    const upload = await this.options.worker.request({
      op: "prepareKeyUpload",
      oneTimeKeyCount: counts.one_time_key_counts.signed_curve25519 ?? 0,
      hasUnusedFallbackKey: counts.unused_fallback_key_types.includes("signed_curve25519"),
    });
    if (upload) {
      await uploadKeys(this.options.client, deviceId, upload, signal);
      await this.options.worker.request({ op: "markKeysPublished" });
    }
    this.needsReplenish = false;
  }

  private schedule(): void {
    this.dirty = true;
    if (this.processing || this.stopped || this.suspended) return;
    this.processing = (async () => {
      try {
        while (this.dirty && !this.stopped && !this.suspended) {
          this.dirty = false;
          await this.process(this.latestRows);
        }
      } catch {
        // オフラインなどで失敗したら、少し待って残っている行と鍵の補充を試し直す。
        if (!this.stopped && !this.suspended) {
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
    if (!deviceId) return;
    const { worker, client } = this.options;
    const signal = this.processAbort.signal;
    let rowsFailure: unknown = null;
    if (rows.length > 0) {
      try {
        const senders = [...new Set(rows.map((row) => row.sender_user_id))].sort();
        const deviceKeys = await queryDeviceKeys(client, senders, signal);
        if (this.stopped) return;
        const result = await worker.request({ op: "processToDevice", rows, deviceKeys });
        if (result.newOlmSessions > 0) this.needsReplenish = true;
        if (result.processed.length > 0) {
          await ackToDevice(client, deviceId, result.processed, signal);
          await worker.request({ op: "acknowledged", ids: result.processed });
        }
      } catch (error) {
        rowsFailure = error;
      }
    }
    // 補充は受信の行とは別に持ち、行の処理（keys/query や ack）が失敗しても行う。ack や補充が
    // 失敗して試し直すと、同じ行は処理済みとして返るので、行の結果だけでは補充し直せない。
    if (this.needsReplenish && !this.stopped && !signal.aborted) {
      await this.replenish(await uploadKeys(client, deviceId, null, signal), signal);
    }
    if (rowsFailure) throw rowsFailure;
  }

  /** 鍵バックアップにまだ送っていない受信用セッションの数。 */
  pendingBackupCount(): Promise<number> {
    return this.options.worker.request({ op: "pendingBackupCount" });
  }

  /**
   * ログアウトの確認の前に、to-device の処理を止めて、処理中のものを終わらせる。確認から削除まで
   * 新しい鍵を保存させない（数えたあとで増えた鍵を、確かめずに消さないため）。
   */
  async suspend(): Promise<void> {
    this.suspended = true;
    // 通信中の要求（keys/query、ack、補充）を中止する。暗号ストアの書き込みは Worker の中で
    // 1 つずつ終わるので、処理中のものが終わるのを待てばよい。
    this.processAbort.abort();
    this.subscription?.unsubscribe();
    this.subscription = null;
    if (this.retryTimer) clearTimeout(this.retryTimer);
    this.retryTimer = null;
    await this.processing?.catch(() => undefined);
  }

  /** ログアウトを取りやめたら、to-device の処理に戻る。 */
  resume(): void {
    if (!this.suspended) return;
    this.suspended = false;
    this.processAbort = new AbortController();
    this.subscribe();
  }

  /** 購読を止め、暗号ストアを閉じる。 */
  async stop(): Promise<void> {
    this.halt();
    await this.settle();
    await this.options.worker.request({ op: "close" }).catch(() => undefined);
    this.options.onStatus?.("stopped");
  }

  /**
   * ログアウト：暗号ストアを消してから、端末の削除 API を呼ぶ（docs/e2ee.md の「ログアウトと
   * 端末の削除」）。API が失敗したら `device_id` を残し、同じユーザーの次のログインで消し直す。
   */
  async destroy(): Promise<void> {
    this.halt();
    // 登録の要求が終わってから消す（先に消すと、あとから届いた登録で端末ができてしまう）。
    await this.settle();
    const deviceId = this.deviceId;
    if (deviceId) markDeviceDeletionPending({ userId: this.options.userId, deviceId });
    await this.options.worker.request({ op: "destroy", namespace: this.options.namespace });
    this.options.onStatus?.("stopped");
    // 待っても登録の結果が分からなければ、いま消すと、そのあとに登録が届いて端末が残りうる
    // （404 で削除待ちを外してしまう）。削除待ちに残し、次のログインで消し直す。
    if (deviceId && !this.registering) await this.deleteDeviceOrRemember(deviceId);
  }

  private halt(): void {
    this.stopped = true;
    this.abort.abort();
    this.processAbort.abort();
    this.subscription?.unsubscribe();
    this.subscription = null;
    if (this.retryTimer) clearTimeout(this.retryTimer);
    this.retryTimer = null;
  }

  /** 処理中のもの（起動と to-device）を待つ。通信は中止してあるので、長くは待たない。 */
  private async settle(): Promise<void> {
    const starting = this.starting?.catch(() => undefined);
    let timer: ReturnType<typeof setTimeout> | undefined;
    const timeout = new Promise<void>((resolve) => {
      timer = setTimeout(resolve, this.options.settleTimeoutMs ?? DELETE_TIMEOUT_MS);
    });
    await Promise.race([Promise.all([starting, this.processing?.catch(() => undefined)]), timeout]);
    clearTimeout(timer);
  }

  private async deleteDeviceOrRemember(deviceId: string): Promise<void> {
    const pending = { userId: this.options.userId, deviceId };
    markDeviceDeletionPending(pending);
    await deleteOrKeep(this.options.client, pending);
  }
}

/** 削除 API を呼び、消せたら（404 も含む）削除待ちから外す。失敗したら残す。 */
async function deleteOrKeep(
  client: ApiClient,
  pending: { userId: string; deviceId: string },
): Promise<void> {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), DELETE_TIMEOUT_MS);
  try {
    await deleteDevice(client, pending.deviceId, { signal: controller.signal });
    clearPendingDeviceDeletion(pending);
  } catch (error) {
    // 404 はサーバーに端末がない（登録の前に止まった）。消すものがない。
    if (error instanceof ApiError && error.status === 404) clearPendingDeviceDeletion(pending);
    // それ以外は残し、次のログインで消し直す。
  } finally {
    clearTimeout(timer);
  }
}

/** 前のログアウトで消せなかった端末を、同じユーザーのログインで消し直す。 */
export async function retryPendingDeviceDeletion(client: ApiClient, userId: string): Promise<void> {
  for (const pending of readPendingDeviceDeletions()) {
    if (pending.userId === userId) await deleteOrKeep(client, pending);
  }
}
