import type {
  JsonObject,
  SasView,
  VerificationMessage,
  VerificationOutgoing,
} from "@zoo/crypto-wasm";

import type { ApiClient } from "../api/client";
import type { CryptoWorker } from "./cryptoWorker";
import { claimKeys, sendToDevice } from "./megolmApi";
import type { SasUpdate } from "./protocol";
import { queryKeys } from "./revocationApi";
import { saveAcceptedMasterKey } from "./revocationController";
import { ReauthRequiredError } from "./setupController";
import { SAS_NEEDS_UNLOCK } from "./store/verification";

/** 期限切れを確かめ、処理できなかったメッセージを試し直す間隔（ミリ秒）。 */
export const SAS_EXPIRE_INTERVAL_MS = 5_000;
/** 検証の期限（`VERIFICATION_TIMEOUT_MS` と同じ 10 分）。これより古い未処理のメッセージは捨てる。 */
const SAS_TIMEOUT_MS = 10 * 60 * 1000;

/**
 * 処理に送信ユーザーの `keys/query` が要るメッセージ（送信端末が相手のマスター鍵からたどれるかを
 * 確かめる、`SasVerification::from_request` と `ready`）。ほかのメッセージは、相手の端末が決まった
 * あとなので鍵を使わない。
 */
const NEEDS_KEYS = new Set(["m.key.verification.request", "m.key.verification.ready"]);

/** 検証が終わったあと、相手のマスター鍵への署名とピンの保存の状態。 */
export type SasSaveState =
  /** まだ（検証が終わっていない）。 */
  | "idle"
  /** 保存している（再認証を待っているあいだも含む）。 */
  | "saving"
  | "saved"
  /** 再認証できなかった、または古くなった（5 分）。再認証し直して保存する。 */
  | "reauth"
  | "failed";

export type SasErrorCode =
  /** 解錠していないので、受け入れられない。 */
  | "needsUnlock"
  /** 相手に送れなかった。 */
  | "sendFailed"
  /** 相手からのメッセージを処理するための鍵を取れなかった（あとで試し直す）。 */
  | "receiveFailed"
  | "failed";

export interface SasEntry {
  view: SasView;
  /** Worker の処理か送信の途中。ボタンを押せなくする。 */
  busy: boolean;
  save: SasSaveState;
  error: SasErrorCode | null;
}

export interface SasControllerDeps {
  worker: CryptoWorker;
  client: ApiClient;
  userId: string;
  /** この端末の `device_id`（to-device の送信元）。 */
  deviceId: string;
  now?: () => number;
  /** 期限切れを確かめる間隔。テストでは 0 にして止める。 */
  expireIntervalMs?: number;
}

function codeOf(error: unknown): string | undefined {
  const code = (error as { code?: unknown } | null)?.code;
  return typeof code === "string" ? code : undefined;
}

/**
 * 他のユーザーの絵文字 SAS 検証（#368）を、画面のスレッドで進める。
 *
 * - 状態機械と鍵は Worker（`SasVerifications`）にあり、ここは届いたメッセージを Worker に渡し、
 *   Worker が返したメッセージを暗号化して to-device で送る。Worker の処理と送信は 1 つずつ順に行う
 *   （同じ相手への `mac` と `done` の順を守るため）。
 * - 双方が確かめ終わったら（`done`）、相手のマスター鍵に署名して、`replace: true` のピンと署名を
 *   保存する（#362 の受け入れと同じく、直前の再認証が要る）。再認証は「一致する」を押したときに
 *   始める。
 */
export class SasController {
  private readonly entries = new Map<string, SasEntry>();
  private readonly listeners = new Set<() => void>();
  private readonly savedListeners = new Set<(userId: string) => void>();
  /** 「一致する」を押したときに始めた再認証（検証が終わったら、その ID トークンで保存する）。 */
  private readonly reauths = new Map<string, Promise<string | null>>();
  private snapshot: SasEntry[] = [];
  /**
   * 届いたが、まだ Worker に渡していないメッセージ（届いた順）。Olm のメッセージは 2 回は復号
   * できないので、`keys/query` が一時的に失敗しても捨てずに残し、次の確認で試し直す。
   */
  private inbox: { message: VerificationMessage; receivedAt: number }[] = [];
  private queue: Promise<unknown> = Promise.resolve();
  private timer: ReturnType<typeof setInterval> | null = null;
  private disposed = false;

  constructor(private readonly deps: SasControllerDeps) {
    const interval = deps.expireIntervalMs ?? SAS_EXPIRE_INTERVAL_MS;
    if (interval > 0) {
      this.timer = setInterval(() => void this.expire().catch(() => undefined), interval);
    }
  }

  /** `useSyncExternalStore` 用。 */
  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  getSnapshot = (): SasEntry[] => this.snapshot;

  /** 相手のマスター鍵への署名を保存できたとき（検証済みの表示を読み直す）。 */
  onSaved(listener: (userId: string) => void): () => void {
    this.savedListeners.add(listener);
    return () => this.savedListeners.delete(listener);
  }

  /** `userId` に検証を求める。`transaction_id` を返す。 */
  request(userId: string): Promise<string> {
    return this.serial(async () => {
      const keysQuery = await queryKeys(this.deps.client, [userId]);
      const update = await this.deps.worker.request({
        op: "sasRequest",
        targetUserId: userId,
        keysQuery,
        now: this.now(),
      });
      await this.apply(update);
      return update.view.transaction_id;
    });
  }

  /** to-device で届いた検証のメッセージ（`E2eeDeviceManager` の `onVerifications`）。 */
  handleMessages(messages: VerificationMessage[]): Promise<void> {
    const receivedAt = this.now();
    for (const message of messages) this.inbox.push({ message, receivedAt });
    return this.drain();
  }

  /**
   * 届いたメッセージを、届いた順に Worker に渡す。`keys/query` を取れなければ、そのメッセージと
   * あとのメッセージを残して止める（順を変えない）。残したものは、期限の確認のたびに試し直す。
   */
  private drain(): Promise<void> {
    return this.serial(async () => {
      // 送信ユーザーの `keys/query` は、この回の中では使い回す。
      const keys = new Map<string, JsonObject>();
      while (this.inbox.length > 0) {
        const { message, receivedAt } = this.inbox[0]!;
        const txnId = message.content.transaction_id;
        if (this.now() - receivedAt > SAS_TIMEOUT_MS) {
          this.inbox.shift();
          continue;
        }
        let keysQuery: JsonObject = {};
        if (NEEDS_KEYS.has(message.type)) {
          const sender = message.sender_user_id;
          try {
            keysQuery = keys.get(sender) ?? (await queryKeys(this.deps.client, [sender]));
          } catch {
            if (typeof txnId === "string") this.patch(txnId, { error: "receiveFailed" });
            return;
          }
          keys.set(sender, keysQuery);
        }
        this.inbox.shift();
        try {
          const update = await this.deps.worker.request({
            op: "sasReceive",
            message,
            keysQuery,
            now: this.now(),
          });
          if (update) {
            if (this.entries.get(update.view.transaction_id)?.error === "receiveFailed") {
              this.patch(update.view.transaction_id, { error: null });
            }
            await this.apply(update);
          }
        } catch {
          // Worker が受け付けないメッセージは、試し直しても通らない。捨てて続きを処理する。
        }
      }
    });
  }

  accept(txnId: string): Promise<void> {
    return this.act(txnId, () => this.deps.worker.request({ op: "sasAccept", txnId }));
  }

  /**
   * 絵文字が一致すると答えた。`reauth` は、このクリックの中で開いた再認証（ID トークン）。検証が
   * 終わったら、その ID トークンで署名とピンを保存する。
   */
  confirm(txnId: string, reauth: Promise<string | null>): Promise<void> {
    // 待っているあいだに失敗しても、未処理の reject にしない（保存のときに扱う）。
    reauth.catch(() => undefined);
    this.reauths.set(txnId, reauth);
    return this.act(txnId, () => this.deps.worker.request({ op: "sasConfirm", txnId }));
  }

  mismatch(txnId: string): Promise<void> {
    return this.act(txnId, () => this.deps.worker.request({ op: "sasMismatch", txnId }));
  }

  cancel(txnId: string): Promise<void> {
    return this.act(txnId, async () => {
      const update = await this.deps.worker.request({ op: "sasCancel", txnId });
      return update ?? undefined;
    });
  }

  /** 画面を閉じる。終わっていなければ取りやめてから忘れる。 */
  async dismiss(txnId: string): Promise<void> {
    const entry = this.entries.get(txnId);
    if (entry && !finished(entry.view)) await this.cancel(txnId).catch(() => undefined);
    await this.serial(async () => {
      await this.deps.worker.request({ op: "sasForget", txnId }).catch(() => undefined);
      this.entries.delete(txnId);
      this.reauths.delete(txnId);
      this.publish();
    });
  }

  /** 再認証し直して、署名とピンを保存し直す。 */
  retrySave(txnId: string, reauth: Promise<string | null>): Promise<void> {
    reauth.catch(() => undefined);
    this.reauths.set(txnId, reauth);
    return this.save(txnId);
  }

  /** 10 分を過ぎた検証を取りやめる（`m.timeout`）。 */
  async expire(): Promise<void> {
    if (this.inbox.length > 0) await this.drain();
    if (this.entries.size === 0) return;
    return this.serial(async () => {
      const updates = await this.deps.worker.request({ op: "sasExpire", now: this.now() });
      for (const update of updates) await this.apply(update);
    });
  }

  dispose(): void {
    this.disposed = true;
    if (this.timer !== null) clearInterval(this.timer);
    this.timer = null;
    this.listeners.clear();
    this.savedListeners.clear();
  }

  private act(txnId: string, task: () => Promise<SasUpdate | undefined>): Promise<void> {
    return this.serial(async () => {
      this.patch(txnId, { busy: true, error: null });
      try {
        const update = await task();
        if (update) await this.apply(update);
      } catch (error) {
        this.patch(txnId, {
          error: codeOf(error) === SAS_NEEDS_UNLOCK ? "needsUnlock" : "failed",
        });
      } finally {
        this.patch(txnId, { busy: false });
      }
    });
  }

  /** 状態を画面に出し、送るメッセージを送る。検証が終わったら保存を始める。 */
  private async apply(update: SasUpdate): Promise<void> {
    const txnId = update.view.transaction_id;
    const current = this.entries.get(txnId);
    this.entries.set(txnId, {
      view: update.view,
      busy: current?.busy ?? false,
      save: current?.save ?? "idle",
      error: current?.error ?? null,
    });
    this.publish();
    if (update.outgoing.length > 0) {
      try {
        await this.send(update.outgoing);
      } catch {
        this.patch(txnId, { error: "sendFailed" });
      }
    }
    const entry = this.entries.get(txnId);
    if (entry && entry.view.phase === "done" && entry.save === "idle") {
      // 再認証を待つので、Worker の処理の列の外で保存する。
      void this.save(txnId);
    }
  }

  private async send(outgoing: VerificationOutgoing[]): Promise<void> {
    const { worker, client, deviceId } = this.deps;
    let encrypted = await worker.request({ op: "sasEncrypt", outgoing, claimed: null });
    if (Object.keys(encrypted.claim).length > 0) {
      const claimed = await claimKeys(client, encrypted.claim);
      encrypted = await worker.request({ op: "sasEncrypt", outgoing, claimed });
    }
    const undelivered = [...encrypted.missing];
    for (const batch of encrypted.batches) {
      undelivered.push(...(await sendToDevice(client, deviceId, batch)));
    }
    // 鍵を払い出されなかった端末と、サーバーのキューに入れられなかった端末（削除された端末など）。
    if (undelivered.length > 0) throw new Error("送れなかった端末があります");
  }

  private async save(txnId: string): Promise<void> {
    const entry = this.entries.get(txnId);
    if (!entry || entry.view.phase !== "done" || entry.save === "saving") return;
    const theirUserId = entry.view.their_user_id;
    const reauth = this.reauths.get(txnId);
    if (!reauth) {
      this.patch(txnId, { save: "reauth" });
      return;
    }
    this.patch(txnId, { save: "saving" });
    try {
      let token: string | null;
      try {
        token = await reauth;
      } catch {
        this.reauths.delete(txnId);
        this.patch(txnId, { save: "reauth" });
        return;
      }
      const keysQuery = await queryKeys(this.deps.client, [theirUserId]);
      const masterKey = (keysQuery.master_keys as Record<string, JsonObject> | undefined)?.[
        theirUserId
      ];
      if (!masterKey) throw new Error("相手のマスター鍵がありません");
      const accepted = await this.deps.worker.request({ op: "sasSign", txnId, masterKey });
      await saveAcceptedMasterKey(this.deps, accepted, token);
      this.patch(txnId, { save: "saved" });
      for (const listener of this.savedListeners) listener(theirUserId);
    } catch (error) {
      if (error instanceof ReauthRequiredError) {
        this.reauths.delete(txnId);
        this.patch(txnId, { save: "reauth" });
      } else {
        this.patch(txnId, { save: "failed" });
      }
    }
  }

  private patch(txnId: string, patch: Partial<Omit<SasEntry, "view">>): void {
    const entry = this.entries.get(txnId);
    if (!entry) return;
    this.entries.set(txnId, { ...entry, ...patch });
    this.publish();
  }

  private publish(): void {
    if (this.disposed) return;
    this.snapshot = [...this.entries.values()].sort(
      (a, b) => a.view.started_at - b.view.started_at,
    );
    for (const listener of this.listeners) listener();
  }

  private serial<T>(task: () => Promise<T>): Promise<T> {
    const next = this.queue.catch(() => undefined).then(task);
    this.queue = next.catch(() => undefined);
    return next;
  }

  private now(): number {
    return (this.deps.now ?? Date.now)();
  }
}

/** 検証済みか、取りやめた。 */
export function finished(view: SasView): boolean {
  return view.phase === "done" || view.phase === "cancelled";
}
