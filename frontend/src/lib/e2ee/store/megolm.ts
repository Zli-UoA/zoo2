import {
  KeySnapshot,
  OlmSession,
  OutboundSession,
  type DeviceAccount,
  type DeviceKeys,
  type EncryptedMessage,
  type InboundSessionStore,
  type JsonObject,
  type NewTofuPin,
  type OutgoingMessage,
  type OwnKeys,
  type RevisionStatus,
  type SessionRef,
  type TrustStore,
  type UserState,
} from "@zoo/crypto-wasm";

import { inboundSessionKey, type CryptoStoreWrite } from "./backend";

/**
 * 送信用の Olm セッションを置く `meta` のキーの接頭辞（後ろは相手の端末の識別鍵）。
 *
 * 受信で作った Olm セッション（`olmSessions`）とは分けて置く。送信用のセッションを受信用の一覧に
 * 混ぜると、暗号化で進めた状態と一覧の中の古い写しがずれるため。相手の端末ごとに 1 つを使い回し、
 * `keys/claim` のワンタイム鍵を使い切らないようにする。
 */
export const OUTBOUND_OLM_META_PREFIX = "outbound_olm:";

/** 配れない相手の理由（送信画面に出す。docs/e2ee.md の「鍵の配布先の判定」）。 */
export type UndeliverableReason =
  | "notSetUp"
  | "invalidKeys"
  | "needsReverification"
  | "pinPending"
  | "blocked";

export interface RecipientSummary {
  /** 配布先のスナップショットの `membership_epoch`。送信用セッションの epoch になる。 */
  epoch: number;
  /** 初めて見た相手の TOFU のピン（`POST /api/e2ee/tofu_pins` で置く）。 */
  newPins: NewTofuPin[];
  /** 配れないユーザーと理由。 */
  undeliverable: { userId: string; reason: UndeliverableReason }[];
  /** 配る端末の数（今の端末を除く）。 */
  deviceCount: number;
  /** 配るユーザーの数（自分を含む）。 */
  userCount: number;
  /** スナップショットのメンバー（`user_id` の昇順）。前回の送信から増えた人を表示するのに使う。 */
  userIds: string[];
}

/** `keys/claim` の要求の `one_time_keys`（`{user_id: {device_id: "signed_curve25519"}}`）。 */
export type ClaimRequest = Record<string, Record<string, "signed_curve25519">>;
/** `keys/claim` の応答の `one_time_keys`。 */
export type ClaimedKeys = Record<string, Record<string, Record<string, JsonObject>>>;
/** to-device で送る `messages`（`{user_id: {device_id: content}}`）。 */
export type ToDeviceMessages = Record<string, Record<string, JsonObject>>;

export interface SharePlan {
  /** 鍵を配る端末のうち、送信用の Olm セッションがまだない端末（`keys/claim` で払い出す）。 */
  claim: ClaimRequest;
  /** 鍵を配る端末の数。0 なら配るものはない。 */
  pending: number;
}

export interface ShareResult {
  messages: ToDeviceMessages;
  /** ワンタイム鍵もフォールバック鍵も払い出されず、配れなかった端末。 */
  missing: { userId: string; deviceId: string }[];
}

export type DecryptOutcome =
  | {
      status: "decrypted";
      body: string;
      kind: "message" | "emote";
      sender: "verified" | "unverified" | "unauthenticated";
      revision: RevisionStatus;
    }
  | { status: "undecryptable"; reason: "keyMissing" | "invalid" | "failed" };

/** Megolm の処理が使う、暗号ストアの中身（`CryptoStore` が渡す）。 */
export interface MegolmContext {
  account: DeviceAccount;
  inbound: InboundSessionStore;
  trust(): TrustStore;
  storageKey: Uint8Array;
  loadInboundSession(ref: SessionRef): Promise<void>;
  write(changes: CryptoStoreWrite): Promise<void>;
  readMeta(): Promise<Record<string, string>>;
  ownKeys(): Promise<{ keys: OwnKeys } | null>;
}

interface Outbound {
  session: OutboundSession;
  /** 鍵を配った端末（`user_id|device_id`）。 */
  shared: Set<string>;
}

interface PendingDistribution {
  epoch: number;
  devices: DeviceKeys[];
}

/** `code` 付きのエラー（Worker の外に `code` のまま渡る）。 */
export class MegolmError extends Error {
  constructor(
    readonly code: "NeedsUnlock" | "NoSnapshot" | "NoOutboundSession",
    message: string,
  ) {
    super(message);
    this.name = "MegolmError";
  }
}

function deviceKey(userId: string, deviceId: string): string {
  return `${userId}|${deviceId}`;
}

function undeliverableReason(state: UserState): UndeliverableReason | null {
  switch (state.state) {
    case "trusted":
    case "needs_unlock":
      return null;
    case "no_cross_signing_keys":
      return "notSetUp";
    case "invalid_cross_signing_keys":
      return "invalidKeys";
    case "needs_reverification":
      return "needsReverification";
    case "pin_pending":
      return "pinPending";
    case "blocked":
      return "blocked";
  }
}

/**
 * チャンネルごとの Megolm の送信と、メッセージの復号（docs/e2ee.md の「membership_epoch と送信の順序」
 * 「鍵の配布先の判定」「受信時の検証」）。Worker の中で、暗号ストアを開いているあいだだけ使う。
 *
 * 【決定】（#361）Web の送信用 Megolm セッションは暗号ストアに保存せず、Worker のメモリーにだけ置く。
 * Worker を起動し直したら（タブを開き直したら）、次の送信で新しいセッションを作って配り直す。
 * 自分のメッセージは、送信用セッションを受信用セッションとしても保存するので、開き直しても読める。
 */
export class MegolmChannels {
  private readonly outbound = new Map<string, Outbound>();
  private readonly pending = new Map<string, PendingDistribution>();

  constructor(private readonly ctx: MegolmContext) {}

  /**
   * 配布先のスナップショットを取り直す必要があるか。送信用セッションがない、期間やメッセージ数を
   * 超えた、`membership_epoch` が変わった、のどれかなら、古いセッションを捨てて `true` を返す。
   */
  needsSnapshot(channelId: string, epoch: number): boolean {
    const current = this.outbound.get(channelId);
    if (!current) return true;
    if (current.session.needsRotation(Date.now(), epoch)) {
      this.discard(channelId);
      return true;
    }
    return false;
  }

  /**
   * `recipients` の応答（全ページ）から配布先を判定する。自分の鍵（ユーザー署名鍵）が要るので、
   * 解錠していなければ `NeedsUnlock` で失敗する。判定の記録は保存し直す。
   */
  async evaluate(channelId: string, pages: JsonObject[]): Promise<RecipientSummary> {
    const [first, ...rest] = pages;
    if (!first) throw new MegolmError("NoSnapshot", "配布先のスナップショットがありません");
    const epoch = Number(first.membership_epoch);
    const own = await this.ctx.ownKeys();
    if (!own) throw new MegolmError("NeedsUnlock", "この端末はまだ解錠していません");
    const snapshot = KeySnapshot.fromJson(first);
    const userSigning = own.keys.userSigning;
    let distribution;
    try {
      for (const page of rest) snapshot.extend(KeySnapshot.fromJson(page));
      distribution = this.ctx
        .trust()
        .evaluateWithUserSigning(snapshot, this.ctx.account.deviceId, userSigning);
    } finally {
      userSigning.free();
      own.keys.free();
      snapshot.free();
    }
    await this.saveTrust();
    this.setDistribution(channelId, epoch, distribution.devices);
    const undeliverable: RecipientSummary["undeliverable"] = [];
    for (const [userId, user] of Object.entries(distribution.users)) {
      const reason = undeliverableReason(user.state);
      if (reason) undeliverable.push({ userId, reason });
    }
    return {
      epoch,
      newPins: distribution.new_pins,
      undeliverable: undeliverable.sort((a, b) => a.userId.localeCompare(b.userId)),
      deviceCount: distribution.devices.length,
      userCount: Object.keys(distribution.users).length - undeliverable.length,
      userIds: Object.keys(distribution.users).sort(),
    };
  }

  /**
   * 判定済みの配布先を置く（{@link evaluate} が使う）。`devices` は検証済みの端末鍵で、この後は
   * このクラスが持つ（`free` する）。テストでは、クロス署名の判定を通さずに配布先を置くのに使う。
   */
  setDistribution(channelId: string, epoch: number, devices: DeviceKeys[]): void {
    this.freePending(channelId);
    this.pending.set(channelId, { epoch, devices });
  }

  /** TOFU のピンを置けたもの（`confirmed`）と、409 で置けなかったもの（`discarded`）を記録する。 */
  async settlePins(confirmed: NewTofuPin[], discarded: NewTofuPin[]): Promise<void> {
    const trust = this.ctx.trust();
    for (const pin of confirmed) trust.confirmTofuPin(pin);
    for (const pin of discarded) trust.discardTofuPin(pin);
    await this.saveTrust();
  }

  /**
   * 判定した配布先で送信用セッションを用意する。epoch が違えば作り直す。まだ鍵を配っていない端末と、
   * そのうち送信用の Olm セッションがない端末（`keys/claim` で払い出す）を返す。
   */
  async start(channelId: string): Promise<SharePlan> {
    const pending = this.pending.get(channelId);
    if (!pending) throw new MegolmError("NoSnapshot", "配布先を判定していません");
    let current = this.outbound.get(channelId);
    if (current && current.session.epoch !== pending.epoch) {
      this.discard(channelId);
      current = undefined;
    }
    if (!current) {
      const session = new OutboundSession(channelId, pending.epoch, Date.now());
      current = { session, shared: new Set() };
      this.outbound.set(channelId, current);
      // 自分のメッセージを、この端末でも（開き直したあとも）読めるようにする。
      this.ctx.inbound.addOutbound(this.ctx.account, session);
      const ref: SessionRef = {
        channel_id: channelId,
        sender_key: this.ctx.account.identityKeys().curve25519,
        session_id: session.sessionId,
      };
      const pickle = this.ctx.inbound.pickleSession(ref, this.ctx.account, this.ctx.storageKey);
      if (pickle)
        await this.ctx.write({ inboundSessions: [{ key: inboundSessionKey(ref), pickle }] });
    }
    const meta = await this.ctx.readMeta();
    const claim: ClaimRequest = {};
    let count = 0;
    for (const device of pending.devices) {
      if (current.shared.has(deviceKey(device.userId, device.deviceId))) continue;
      count += 1;
      if (meta[OUTBOUND_OLM_META_PREFIX + device.curve25519]) continue;
      (claim[device.userId] ??= {})[device.deviceId] = "signed_curve25519";
    }
    return { claim, pending: count };
  }

  /**
   * まだ配っていない端末あての `zoo.room_key` を Olm で暗号化する。送信用の Olm セッションがなければ、
   * `keys/claim` で払い出された鍵で作る。進めた Olm セッションは保存する。配れたかは、to-device を
   * 送れたあとに {@link markShared} で記録する。
   */
  async share(channelId: string, claimed: ClaimedKeys): Promise<ShareResult> {
    const pending = this.pending.get(channelId);
    const current = this.outbound.get(channelId);
    if (!pending || !current) {
      throw new MegolmError("NoOutboundSession", "送信用セッションがありません");
    }
    const { account, storageKey } = this.ctx;
    const meta = await this.ctx.readMeta();
    const roomKey = current.session.roomKey();
    const messages: ToDeviceMessages = {};
    const missing: ShareResult["missing"] = [];
    const sessions: Record<string, string> = {};
    try {
      for (const device of pending.devices) {
        if (current.shared.has(deviceKey(device.userId, device.deviceId))) continue;
        const metaKey = OUTBOUND_OLM_META_PREFIX + device.curve25519;
        const stored = sessions[metaKey] ?? meta[metaKey];
        let olm: OlmSession;
        if (stored) {
          olm = OlmSession.fromPickle(stored, account, storageKey);
        } else {
          const oneTimeKey = Object.values(claimed[device.userId]?.[device.deviceId] ?? {})[0];
          if (!oneTimeKey) {
            missing.push({ userId: device.userId, deviceId: device.deviceId });
            continue;
          }
          olm = account.createOutboundOlmSession(device, oneTimeKey);
        }
        try {
          (messages[device.userId] ??= {})[device.deviceId] = account.encryptRoomKey(
            olm,
            device,
            roomKey,
          );
          sessions[metaKey] = olm.pickle(account, storageKey);
        } finally {
          olm.free();
        }
      }
    } finally {
      roomKey.free();
    }
    if (Object.keys(sessions).length > 0) await this.ctx.write({ meta: sessions });
    return { messages, missing };
  }

  /** to-device を送れた端末を、鍵を配った端末として記録する。 */
  markShared(channelId: string, devices: { userId: string; deviceId: string }[]): void {
    const current = this.outbound.get(channelId);
    if (!current) return;
    for (const device of devices) current.shared.add(deviceKey(device.userId, device.deviceId));
  }

  /** メッセージを暗号化する。`ciphertext` と、送信の要求に入れる `epoch` を返す。 */
  encrypt(channelId: string, message: OutgoingMessage): { ciphertext: JsonObject; epoch: number } {
    const current = this.outbound.get(channelId);
    if (!current) throw new MegolmError("NoOutboundSession", "送信用セッションがありません");
    return {
      ciphertext: current.session.encrypt(this.ctx.account, message),
      epoch: current.session.epoch,
    };
  }

  /** 送信用セッションを捨てる（409 `E2EE_STALE_EPOCH` を受けたときなど）。 */
  discard(channelId: string): void {
    this.outbound.get(channelId)?.session.free();
    this.outbound.delete(channelId);
  }

  /** `messages` の行を復号し、行と照合する。行ごとの結果を同じ順で返す。 */
  async decrypt(rows: EncryptedMessage[]): Promise<DecryptOutcome[]> {
    const results: DecryptOutcome[] = [];
    for (const row of rows) {
      const ciphertext = row.ciphertext as { sender_key?: unknown; session_id?: unknown };
      if (typeof ciphertext.sender_key !== "string" || typeof ciphertext.session_id !== "string") {
        results.push({ status: "undecryptable", reason: "invalid" });
        continue;
      }
      await this.ctx.loadInboundSession({
        channel_id: row.channel_id,
        sender_key: ciphertext.sender_key,
        session_id: ciphertext.session_id,
      });
      try {
        const decrypted = this.ctx.inbound.decrypt(row);
        const kind = decrypted.payload.content.kind === "emote" ? "emote" : "message";
        results.push({
          status: "decrypted",
          body: decrypted.payload.content.body,
          kind,
          sender:
            decrypted.origin === "backup"
              ? "unauthenticated"
              : decrypted.sender_verified
                ? "verified"
                : "unverified",
          revision: decrypted.revision,
        });
      } catch (error) {
        const code = (error as { code?: unknown }).code;
        results.push({
          status: "undecryptable",
          reason:
            code === "MissingRoomKey" ? "keyMissing" : code === "Rejected" ? "invalid" : "failed",
        });
      }
    }
    return results;
  }

  free(): void {
    // Map は、たどっている途中で消しても次の要素に進める。
    for (const channelId of this.outbound.keys()) this.discard(channelId);
    for (const channelId of this.pending.keys()) this.freePending(channelId);
  }

  private freePending(channelId: string): void {
    for (const device of this.pending.get(channelId)?.devices ?? []) device.free();
    this.pending.delete(channelId);
  }

  private async saveTrust(): Promise<void> {
    await this.ctx.write({
      trustStore: this.ctx.trust().pickle(this.ctx.account, this.ctx.storageKey),
    });
  }
}
