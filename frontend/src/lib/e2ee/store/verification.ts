import {
  DeviceKeys,
  KeySnapshot,
  OlmSession,
  SasVerification,
  type AcceptedMasterKey,
  type DeviceAccount,
  type JsonObject,
  type OwnKeys,
  type SasView,
  type TrustStore,
  type VerificationMessage,
  type VerificationOutgoing,
} from "@zoo/crypto-wasm";

import type { CryptoStoreWrite } from "./backend";
import {
  OUTBOUND_OLM_META_PREFIX,
  type ClaimRequest,
  type ClaimedKeys,
  type ToDeviceMessages,
} from "./megolm";

/** 検証の今の状態と、送るメッセージ（画面のスレッドが暗号化して送る）。 */
export interface SasUpdate {
  view: SasView;
  outgoing: VerificationOutgoing[];
}

/** 検証のメッセージを暗号化した結果。 */
export interface SasEncrypted {
  /**
   * 送る to-device の `messages`。1 つの要求には 1 つの端末あてに 1 件しか入れられないので、
   * メッセージごとに分け、この順に送る。
   */
  batches: ToDeviceMessages[];
  /** 送信用の Olm セッションがない端末（`keys/claim` で払い出してから、もう一度呼ぶ）。 */
  claim: ClaimRequest;
  /** ワンタイム鍵もフォールバック鍵も払い出されず、送れなかった端末。 */
  missing: { userId: string; deviceId: string }[];
}

/** 検証の処理が使う、暗号ストアの中身（`CryptoStore` が渡す）。 */
export interface SasContext {
  account: DeviceAccount;
  trust(): TrustStore;
  storageKey: Uint8Array;
  write(changes: CryptoStoreWrite): Promise<void>;
  readMeta(): Promise<Record<string, string>>;
  ownKeys(): Promise<{ keys: OwnKeys } | null>;
  saveTrustStore(): Promise<void>;
}

/** 解錠していないので、検証を受け入れられない（`CryptoFailure.code`）。 */
export const SAS_NEEDS_UNLOCK = "SasNeedsUnlock";
/** その `transaction_id` の検証がない（閉じたか、Worker を開き直した）。 */
export const SAS_UNKNOWN = "SasUnknown";

export class SasError extends Error {
  constructor(
    readonly code: string,
    message: string,
  ) {
    super(message);
    this.name = "SasError";
  }
}

const REQUEST_TYPE = "m.key.verification.request";

/**
 * 他のユーザーの絵文字 SAS 検証（#368。docs/e2ee.md の「絵文字 SAS 検証」）。
 *
 * vodozemac の SAS の状態は保存できないので、検証は Worker のメモリーにだけ置く。Worker を開き直したら
 * 続けられない（相手には、10 分で `m.timeout` の取りやめになる）。
 */
export class SasVerifications {
  private readonly active = new Map<string, SasVerification>();

  constructor(private readonly ctx: SasContext) {}

  /** `targetUserId` に検証を求める。`keysQuery` は相手の `keys/query` の応答。 */
  async request(targetUserId: string, keysQuery: JsonObject, now: number): Promise<SasUpdate> {
    await this.requireUnlocked();
    const snapshot = KeySnapshot.fromJson(keysQuery);
    try {
      const { verification, outgoing } = SasVerification.request(
        this.ctx.account,
        this.ctx.trust(),
        snapshot,
        targetUserId,
        now,
      );
      const view = verification.view();
      this.active.set(view.transaction_id, verification);
      return { view, outgoing };
    } finally {
      snapshot.free();
    }
  }

  /**
   * 届いたメッセージで検証を進める。`keysQuery` は送信ユーザーの `keys/query` の応答。知らない
   * `transaction_id` のメッセージ（要求を除く）と、受け付けない要求（古い、たどれない端末など）は
   * 捨てて `null` を返す。
   */
  receive(message: VerificationMessage, keysQuery: JsonObject, now: number): SasUpdate | null {
    const txnId = message.content.transaction_id;
    if (typeof txnId !== "string") return null;
    const snapshot = KeySnapshot.fromJson(keysQuery);
    try {
      const current = this.active.get(txnId);
      if (!current) {
        if (message.type !== REQUEST_TYPE) return null;
        let verification: SasVerification;
        try {
          verification = SasVerification.fromRequest(
            this.ctx.account,
            this.ctx.trust(),
            message,
            snapshot,
            now,
          );
        } catch {
          return null;
        }
        this.active.set(txnId, verification);
        return { view: verification.view(), outgoing: [] };
      }
      const outgoing = current.receive(message, snapshot);
      return { view: current.view(), outgoing };
    } finally {
      snapshot.free();
    }
  }

  async accept(txnId: string): Promise<SasUpdate> {
    await this.requireUnlocked();
    const verification = this.get(txnId);
    return { outgoing: verification.accept(), view: verification.view() };
  }

  confirm(txnId: string): SasUpdate {
    const verification = this.get(txnId);
    return { outgoing: verification.confirm(), view: verification.view() };
  }

  mismatch(txnId: string): SasUpdate {
    const verification = this.get(txnId);
    return { outgoing: verification.mismatch(), view: verification.view() };
  }

  cancel(txnId: string): SasUpdate | null {
    const verification = this.active.get(txnId);
    if (!verification) return null;
    return { outgoing: verification.cancel(), view: verification.view() };
  }

  /** 10 分を過ぎた検証を取りやめる。変わった検証だけを返す。 */
  expire(now: number): SasUpdate[] {
    const updates: SasUpdate[] = [];
    for (const verification of this.active.values()) {
      const outgoing = verification.expire(now);
      if (outgoing.length > 0) updates.push({ view: verification.view(), outgoing });
    }
    return updates;
  }

  /** 画面を閉じた検証を忘れる。 */
  forget(txnId: string): void {
    this.active.get(txnId)?.free();
    this.active.delete(txnId);
  }

  /**
   * 検証できた相手のマスター鍵に、自分のユーザー署名鍵で署名する。記録は登録待ちで保存し、
   * `replace: true` のピンと署名を返す（保存できたら `confirmAccepted`）。
   */
  async sign(txnId: string, masterKey: JsonObject): Promise<AcceptedMasterKey> {
    const verification = this.get(txnId);
    const own = await this.ctx.ownKeys();
    if (!own) throw new SasError(SAS_NEEDS_UNLOCK, "解錠していません");
    const userSigning = own.keys.userSigning;
    try {
      const accepted = verification.signMasterKey(this.ctx.trust(), userSigning, masterKey);
      await this.ctx.saveTrustStore();
      return accepted;
    } finally {
      userSigning.free();
      own.keys.free();
    }
  }

  /**
   * 検証のメッセージを、宛先の端末ごとに Olm で暗号化する。送信用の Olm セッションは Megolm の
   * 鍵の配布と同じもの（`outbound_olm:<識別鍵>`）を使い回す。セッションがなく、`claimed` にも
   * 鍵がない端末は `claim` に入れて返す（`claimed` が `null` なら、何も暗号化しない）。
   */
  async encrypt(
    outgoing: VerificationOutgoing[],
    claimed: ClaimedKeys | null,
  ): Promise<SasEncrypted> {
    const { account, storageKey } = this.ctx;
    const meta = await this.ctx.readMeta();
    const claim: ClaimRequest = {};
    const missing: SasEncrypted["missing"] = [];
    for (const out of outgoing) {
      for (const device of out.devices) {
        if (meta[OUTBOUND_OLM_META_PREFIX + device.curve25519]) continue;
        if (claimed?.[device.user_id]?.[device.device_id]) continue;
        (claim[device.user_id] ??= {})[device.device_id] = "signed_curve25519";
      }
    }
    if (claimed === null && Object.keys(claim).length > 0) {
      return { batches: [], claim, missing };
    }
    const sessions: Record<string, string> = {};
    const batches: ToDeviceMessages[] = [];
    for (const out of outgoing) {
      const batch: ToDeviceMessages = {};
      for (const target of out.devices) {
        const metaKey = OUTBOUND_OLM_META_PREFIX + target.curve25519;
        const stored = sessions[metaKey] ?? meta[metaKey];
        const device = DeviceKeys.fromVerificationTarget(target);
        let olm: OlmSession | null = null;
        try {
          if (stored) {
            olm = OlmSession.fromPickle(stored, account, storageKey);
          } else {
            const oneTimeKey = Object.values(
              claimed?.[target.user_id]?.[target.device_id] ?? {},
            )[0];
            if (!oneTimeKey) {
              missing.push({ userId: target.user_id, deviceId: target.device_id });
              continue;
            }
            olm = account.createOutboundOlmSession(device, oneTimeKey);
          }
          (batch[target.user_id] ??= {})[target.device_id] = account.encryptVerification(
            olm,
            device,
            out.type,
            out.content,
          );
          sessions[metaKey] = olm.pickle(account, storageKey);
        } finally {
          olm?.free();
          device.free();
        }
      }
      if (Object.keys(batch).length > 0) batches.push(batch);
    }
    if (Object.keys(sessions).length > 0) await this.ctx.write({ meta: sessions });
    return { batches, claim: {}, missing };
  }

  close(): void {
    for (const verification of this.active.values()) verification.free();
    this.active.clear();
  }

  private get(txnId: string): SasVerification {
    const verification = this.active.get(txnId);
    if (!verification) throw new SasError(SAS_UNKNOWN, "検証がありません");
    return verification;
  }

  private async requireUnlocked(): Promise<void> {
    const own = await this.ctx.ownKeys();
    if (!own) throw new SasError(SAS_NEEDS_UNLOCK, "解錠していません");
    own.keys.free();
  }
}
