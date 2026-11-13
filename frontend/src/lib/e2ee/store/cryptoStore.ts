import {
  DeviceAccount,
  InboundSessionStore,
  OlmSessions,
  OwnKeys,
  Revocation,
  TrustStore,
  type OwnSecrets,
  type RevocationSummary,
  type UnlockedSecretStorage,
  type JsonObject,
  type SessionRef,
} from "@zoo/crypto-wasm";

import { inboundSessionKey, type CryptoBackend, type CryptoStoreWrite } from "./backend";
import {
  StorageKeyUnavailableError,
  WRAPPED_STORAGE_KEY_META,
  indexedDbWrappingKeys,
  loadStorageKey,
  type WrappingKeyStore,
} from "./storageKey";

const META_USER_ID = "user_id";
const META_DEVICE_ID = "device_id";
/** サーバーへの登録が済んだか（`"1"`）。応答をなくしたら、同じ鍵で登録し直す（冪等）。 */
const META_REGISTERED = "registered";
/**
 * 解錠した自分の鍵（自己署名鍵、ユーザー署名鍵、バックアップ鍵。#360）。値は `OwnKeys.pickle` の
 * 封筒（保存鍵で暗号化したもの）で、平文の秘密は入らない。
 */
const META_OWN_KEYS = "own_keys";
/** 最後に解錠したときの SSSS の `secrets_revision`（鍵バックアップへのアップロードに付ける）。 */
const META_SECRETS_REVISION = "secrets_revision";
/**
 * 終わっていない失効の進み具合（#362）。値は `Revocation.pickle` の封筒（保存鍵で暗号化したもの）。
 * 新しい自己署名鍵、ユーザー署名鍵、バックアップ鍵と古いバックアップ鍵を含み、マスター鍵と SSSS 鍵は
 * 含まない。
 */
const META_REVOCATION = "revocation";

/** to-device の Shape の行のうち、処理に使う列。 */
export interface ToDeviceRow {
  id: string;
  sender_user_id: string;
  sender_device_id: string;
  type: string;
  content: JsonObject;
}

/** `keys/query` の応答の `device_keys`（ユーザー、端末の順）。 */
export type DeviceKeysByUser = Record<string, Record<string, JsonObject>>;

export interface RegistrationRequest {
  device_keys: JsonObject;
  one_time_keys: Record<string, JsonObject>;
  fallback_keys: Record<string, JsonObject>;
}

export interface KeyUpload {
  one_time_keys: Record<string, JsonObject>;
  fallback_keys: Record<string, JsonObject>;
}

export interface ToDeviceResult {
  /** 処理を終えた（保存した、または捨てた）ID。ack を送ってよい。 */
  processed: string[];
  /** 保存した Megolm セッション鍵の数。 */
  roomKeys: number;
  /** 復号や検証に失敗して捨てた数。 */
  dropped: number;
  /** 新しく作った Olm セッションの数（ワンタイム鍵を使ったので、補充する）。 */
  newOlmSessions: number;
}

export interface OpenedStore {
  store: CryptoStore;
  /** 保存鍵を開けずに暗号ストアを作り直したとき、前の端末の ID（サーバーから消す）。 */
  replacedDeviceId: string | null;
}

/**
 * 1 つのユーザーの暗号ストア（端末アカウント、Olm セッション、Megolm の受信用セッション、
 * 配布先の判定の記録）。docs/e2ee.md の「暗号ストア」。
 *
 * Worker の中で、Web Locks を取れたタブのものだけを使う。状態を変える操作は、変えた値を
 * 1 つのトランザクションで保存してから返す。
 */
export class CryptoStore {
  private readonly olmSessions = new OlmSessions();
  private readonly inbound = new InboundSessionStore();
  private readonly loadedInbound = new Set<string>();
  private readonly processedToDevice: Set<string>;
  private closed = false;
  private broken = false;

  private constructor(
    private readonly backend: CryptoBackend,
    private readonly keys: WrappingKeyStore,
    private readonly name: string,
    private readonly storageKey: Uint8Array,
    private readonly account: DeviceAccount,
    private trust: TrustStore,
    private registered: boolean,
    processedToDevice: string[],
  ) {
    this.processedToDevice = new Set(processedToDevice);
  }

  /**
   * 暗号ストアを開く。端末アカウントがなければ作る（新しい `device_id` は UUID v4）。
   *
   * `reset` は、保存鍵を開けないときに保存先を消して作り直す関数（新しい空の保存先を返す）。
   */
  static async open(options: {
    backend: CryptoBackend;
    name: string;
    userId: string;
    reset: () => Promise<CryptoBackend>;
    keys?: WrappingKeyStore;
  }): Promise<OpenedStore> {
    let backend = options.backend;
    const keys = options.keys ?? indexedDbWrappingKeys;
    let snapshot = await backend.read();
    let replacedDeviceId: string | null = null;
    if (snapshot.meta[META_USER_ID] && snapshot.meta[META_USER_ID] !== options.userId) {
      throw new Error("暗号ストアが別のユーザーのものです");
    }
    let storageKey: Uint8Array;
    try {
      storageKey = await loadStorageKey({
        name: options.name,
        wrapped: snapshot.meta[WRAPPED_STORAGE_KEY_META] ?? null,
        saveWrapped: (wrapped) => backend.write({ meta: { [WRAPPED_STORAGE_KEY_META]: wrapped } }),
        keys,
      });
    } catch (error) {
      if (!(error instanceof StorageKeyUnavailableError)) throw error;
      // 保存鍵がなければ、どの値も読めない。作り直して、新しい端末として登録し直す。
      replacedDeviceId = snapshot.meta[META_DEVICE_ID] ?? null;
      // 開いたままのファイルは消せないので、先に閉じる。
      await backend.close().catch(() => undefined);
      backend = await options.reset();
      snapshot = await backend.read();
      storageKey = await loadStorageKey({
        name: options.name,
        wrapped: null,
        saveWrapped: (wrapped) => backend.write({ meta: { [WRAPPED_STORAGE_KEY_META]: wrapped } }),
        keys,
      });
    }

    const deviceId = snapshot.meta[META_DEVICE_ID];
    if (snapshot.account && deviceId) {
      const account = DeviceAccount.fromPickle(
        snapshot.account,
        storageKey,
        options.userId,
        deviceId,
      );
      const trust = snapshot.trustStore
        ? TrustStore.fromPickle(snapshot.trustStore, account, storageKey)
        : new TrustStore(options.userId);
      const store = new CryptoStore(
        backend,
        keys,
        options.name,
        storageKey,
        account,
        trust,
        snapshot.meta[META_REGISTERED] === "1",
        snapshot.processedToDevice,
      );
      for (const session of snapshot.olmSessions) {
        store.olmSessions.pushPickle(session.pickle, account, storageKey);
      }
      return { store, replacedDeviceId };
    }

    const account = new DeviceAccount(options.userId, crypto.randomUUID());
    account.generateOneTimeKeys(account.maxNumberOfOneTimeKeys);
    account.generateFallbackKey();
    const trust = new TrustStore(options.userId);
    const store = new CryptoStore(
      backend,
      keys,
      options.name,
      storageKey,
      account,
      trust,
      false,
      [],
    );
    await backend.write({
      meta: {
        [META_USER_ID]: options.userId,
        [META_DEVICE_ID]: account.deviceId,
        [META_REGISTERED]: "0",
      },
      account: account.pickle(storageKey),
      trustStore: trust.pickle(account, storageKey),
    });
    return { store, replacedDeviceId };
  }

  get userId(): string {
    return this.account.userId;
  }

  get deviceId(): string {
    return this.account.deviceId;
  }

  get isRegistered(): boolean {
    return this.registered;
  }

  /** 次に保存する配布先の判定の記録（#361 以降が使う）。 */
  get trustStore(): TrustStore {
    return this.trust;
  }

  /** `POST /api/e2ee/devices` の本文（`display_name` を除く）。応答をなくしても同じ鍵で送り直す。 */
  registrationRequest(): RegistrationRequest {
    return {
      device_keys: this.account.deviceKeys(),
      one_time_keys: this.account.oneTimeKeys(),
      fallback_keys: this.account.fallbackKeys(),
    };
  }

  async markRegistered(): Promise<void> {
    this.account.markKeysAsPublished();
    await this.write({
      meta: { [META_REGISTERED]: "1" },
      account: this.account.pickle(this.storageKey),
    });
    this.registered = true;
  }

  /**
   * サーバーのワンタイム鍵を最大数（50 個）に戻し、使われたフォールバック鍵を作り直す。
   * 送るものがなければ `null`。まだ公開していない鍵（前の送信の応答をなくしたもの）も入れる。
   */
  async prepareKeyUpload(counts: {
    oneTimeKeyCount: number;
    hasUnusedFallbackKey: boolean;
  }): Promise<KeyUpload | null> {
    const unpublished = Object.keys(this.account.oneTimeKeys()).length;
    const missing = this.account.maxNumberOfOneTimeKeys - counts.oneTimeKeyCount - unpublished;
    let changed = false;
    if (missing > 0) {
      this.account.generateOneTimeKeys(missing);
      changed = true;
    }
    if (!counts.hasUnusedFallbackKey && Object.keys(this.account.fallbackKeys()).length === 0) {
      this.account.generateFallbackKey();
      changed = true;
    }
    if (changed) {
      await this.write({ account: this.account.pickle(this.storageKey) });
    }
    const upload = {
      one_time_keys: this.account.oneTimeKeys(),
      fallback_keys: this.account.fallbackKeys(),
    };
    if (
      Object.keys(upload.one_time_keys).length === 0 &&
      Object.keys(upload.fallback_keys).length === 0
    ) {
      return null;
    }
    return upload;
  }

  async markKeysPublished(): Promise<void> {
    this.account.markKeysAsPublished();
    await this.write({ account: this.account.pickle(this.storageKey) });
  }

  /**
   * to-device メッセージを処理する（docs/e2ee.md の「受信時の検証」の `zoo.room_key`）。
   *
   * 1 件ごとに、変わった状態（端末アカウント、Olm セッション、受信用セッション）と処理済みの印を
   * 1 つのトランザクションで保存する。ack が届く前に終了しても、次の起動で同じメッセージを
   * 処理し直さない（Olm のメッセージは 2 回は復号できないため）。
   *
   * `deviceKeys` は送信ユーザーの `keys/query` の応答。送信端末の検証済みかの判定（手順 4）は、
   * クロス署名を扱う #360 以降で足す。それまでは「未検証の送信者」として保存する。
   */
  async processToDevice(
    rows: ToDeviceRow[],
    deviceKeys: DeviceKeysByUser,
  ): Promise<ToDeviceResult> {
    const result: ToDeviceResult = { processed: [], roomKeys: 0, dropped: 0, newOlmSessions: 0 };
    for (const row of rows) {
      if (this.processedToDevice.has(row.id)) {
        result.processed.push(row.id);
        continue;
      }
      const changes: CryptoStoreWrite = { addProcessedToDevice: [row.id] };
      try {
        if (row.type === "m.room.encrypted") {
          await this.processEncrypted(row, deviceKeys, changes, result);
        } else {
          result.dropped += 1;
        }
        await this.write(changes);
      } catch (error) {
        // 復号で進んだメモリーの状態を保存できていない。書き込みに限らず（保存済みのセッションの
        // 読み取りなど）途中で失敗したら、開き直すまで使わない。
        this.broken = true;
        throw error;
      }
      this.processedToDevice.add(row.id);
      result.processed.push(row.id);
    }
    return result;
  }

  private async processEncrypted(
    row: ToDeviceRow,
    deviceKeys: DeviceKeysByUser,
    changes: CryptoStoreWrite,
    result: ToDeviceResult,
  ): Promise<void> {
    const before = this.olmSessions.length;
    let decrypted;
    try {
      decrypted = this.account.decryptToDevice(this.olmSessions, row.content);
    } catch {
      // 復号できないメッセージは何度試しても読めない。捨てて ack する。
      result.dropped += 1;
      return;
    }
    try {
      // 復号できたら、セッションの状態が進んでいる。検証の結果に関わらず保存する。
      result.newOlmSessions += this.olmSessions.length - before;
      changes.account = this.account.pickle(this.storageKey);
      changes.olmSessions = this.olmSessions
        .pickleSessions(this.account, this.storageKey, decrypted.senderKey)
        .map((session) => ({
          sessionId: session.session_id,
          theirIdentityKey: session.their_identity_key,
          pickle: session.pickle,
        }));
      const sender = decrypted.sender;
      const senderDevice = decrypted.senderDevice;
      const listed = sender && senderDevice ? deviceKeys[sender]?.[senderDevice] : undefined;
      let roomKey;
      try {
        roomKey = decrypted.verifyRoomKey(this.account, listed ?? null, null);
      } catch {
        result.dropped += 1;
        return;
      }
      const ref: SessionRef = {
        channel_id: roomKey.channelId,
        sender_key: roomKey.senderKey,
        session_id: roomKey.sessionId,
      };
      await this.loadInboundSession(ref);
      const outcome = this.inbound.addRoomKey(roomKey);
      if (outcome !== "kept") {
        const pickle = this.inbound.pickleSession(ref, this.account, this.storageKey);
        if (pickle) changes.inboundSessions = [{ key: inboundSessionKey(ref), pickle }];
        result.roomKeys += 1;
      }
    } finally {
      decrypted.free();
    }
  }

  /** 保存済みの同じキーのセッションを、比べるためにメモリーへ読む。 */
  private async loadInboundSession(ref: SessionRef): Promise<void> {
    const key = inboundSessionKey(ref);
    if (this.loadedInbound.has(key)) return;
    const pickle = await this.backend.readInboundSession(key);
    if (pickle) this.inbound.restoreSession(pickle, this.account, this.storageKey);
    this.loadedInbound.add(key);
  }

  /**
   * 保存できなかったら、メモリーの状態（端末アカウント、Olm セッションなど）は保存より先に
   * 進んでいる。そのまま次の操作を続けると、保存されていない状態で処理し直すことになる
   * （消費済みの Olm メッセージを捨ててしまうなど）。{@link isBroken} にして、呼び出し側に
   * 保存先から開き直させる。
   */
  private async write(changes: CryptoStoreWrite): Promise<void> {
    try {
      await this.backend.write(changes);
    } catch (error) {
      this.broken = true;
      throw error;
    }
  }

  /** 保存に失敗し、メモリーの状態が保存先とずれている。閉じて開き直す。 */
  get isBroken(): boolean {
    return this.broken;
  }

  /** ack を送れた ID を、処理済みの印から外す。 */
  async acknowledged(ids: string[]): Promise<void> {
    if (ids.length === 0) return;
    await this.write({ removeProcessedToDevice: ids });
    for (const id of ids) this.processedToDevice.delete(id);
  }

  /**
   * 解錠した（または初回設定で作った）自分の鍵を保存する（#360）。マスター鍵の秘密鍵は保存せず、
   * 自分のマスター鍵の公開鍵とユーザー署名鍵を、配布先の判定の記録に入れる。
   */
  async saveOwnKeys(secrets: OwnSecrets, secretsRevision: number | null): Promise<void> {
    const keys = secrets.ownKeys();
    const crossSigning = secrets.crossSigning;
    try {
      this.trust.trustOwnMasterKey(crossSigning.master);
      this.trust.recordOwnUserSigningKey(crossSigning.userSigning);
      await this.backend.write({
        meta: {
          [META_OWN_KEYS]: keys.pickle(this.account, this.storageKey),
          [META_SECRETS_REVISION]: secretsRevision === null ? null : String(secretsRevision),
        },
        trustStore: this.trust.pickle(this.account, this.storageKey),
      });
    } finally {
      keys.free();
      crossSigning.free();
    }
  }

  /** 保存した自分の鍵。まだ解錠していなければ `null`。使い終わったら `free()` する。 */
  async ownKeys(): Promise<{ keys: OwnKeys; secretsRevision: number | null } | null> {
    const { meta } = await this.backend.read();
    const sealed = meta[META_OWN_KEYS];
    if (!sealed) return null;
    const revision = meta[META_SECRETS_REVISION];
    return {
      keys: OwnKeys.fromPickle(sealed, this.account, this.storageKey),
      secretsRevision: revision ? Number(revision) : null,
    };
  }

  /** 自分の鍵を保存してあるか（解錠済みの端末か）。 */
  async hasOwnKeys(): Promise<boolean> {
    const { meta } = await this.backend.read();
    return Boolean(meta[META_OWN_KEYS]);
  }

  /** 自分の鍵を消す（E2EE のリセットや、SSSS を開けなくなったとき）。 */
  async forgetOwnKeys(): Promise<void> {
    await this.backend.write({ meta: { [META_OWN_KEYS]: null, [META_SECRETS_REVISION]: null } });
  }

  /** 今の端末の署名付きの端末鍵。 */
  deviceKeys(): JsonObject {
    return this.account.deviceKeys();
  }

  /** 配布先の判定の記録を保存し直す（#361 以降が判定のたびに呼ぶ）。 */
  async saveTrustStore(): Promise<void> {
    await this.write({ trustStore: this.trust.pickle(this.account, this.storageKey) });
  }

  /**
   * 利用者が相手の今のマスター鍵を受け入れた（再検証。#362）。記録を今のユーザー署名鍵で確認したものに
   * 置き換えて保存し、`replace: true` で送るピンと、マスター鍵への署名を返す。解錠していなければ投げる。
   */
  async acceptMasterKey(targetUserId: string, masterKey: JsonObject) {
    const own = await this.ownKeys();
    if (!own) throw new Error("解錠していません");
    const userSigning = own.keys.userSigning;
    try {
      const accepted = this.trust.acceptMasterKey(userSigning, targetUserId, masterKey);
      await this.saveTrustStore();
      return accepted;
    } finally {
      userSigning.free();
      own.keys.free();
    }
  }

  /** 終わっていない失効の進み具合（秘密鍵を除く）。なければ `null`。 */
  async revocationSummary(): Promise<RevocationSummary | null> {
    const { meta } = await this.backend.read();
    const sealed = meta[META_REVOCATION];
    return sealed ? Revocation.peek(sealed, this.account, this.storageKey) : null;
  }

  /** 失効の進み具合を保存する（保存が 1 つ済むたびに呼ぶ）。 */
  async saveRevocation(revocation: Revocation): Promise<void> {
    await this.write({
      meta: { [META_REVOCATION]: revocation.pickle(this.account, this.storageKey) },
    });
  }

  /** 保存した失効を、開け直した SSSS で続ける。 */
  async resumeRevocation(unlocked: UnlockedSecretStorage): Promise<Revocation | null> {
    const { meta } = await this.backend.read();
    const sealed = meta[META_REVOCATION];
    return sealed ? Revocation.resume(sealed, this.account, this.storageKey, unlocked) : null;
  }

  /** 失効の進み具合を消す（終えたとき、やり直すとき）。 */
  async clearRevocation(): Promise<void> {
    await this.write({ meta: { [META_REVOCATION]: null } });
  }

  /**
   * 失効を行う端末が、クロス署名鍵を保存したあとに、記録を新しい鍵にそろえる。新しい鍵の
   * `secrets_revision` も記録する（次の起動で SSSS が書き換えられたと誤って判定しないため）。
   */
  async applyRevocation(revocation: Revocation, secretsRevision: number | null): Promise<void> {
    revocation.applyToTrust(this.trust);
    const secrets = revocation.ownSecrets();
    const keys = secrets.ownKeys();
    try {
      await this.write({
        meta: {
          [META_OWN_KEYS]: keys.pickle(this.account, this.storageKey),
          [META_SECRETS_REVISION]: secretsRevision === null ? null : String(secretsRevision),
          [META_REVOCATION]: revocation.pickle(this.account, this.storageKey),
        },
        trustStore: this.trust.pickle(this.account, this.storageKey),
      });
    } finally {
      keys.free();
      secrets.free();
    }
  }

  /** 鍵バックアップにまだ送っていない受信用セッションの数（ログアウトの確認に使う）。 */
  pendingBackupCount(): Promise<number> {
    return this.backend.pendingBackupCount();
  }

  async close(): Promise<void> {
    if (this.closed) return;
    this.closed = true;
    this.free();
    await this.backend.close();
  }

  /** 暗号ストアを消す（ログアウト）。保存鍵を包む鍵も消す。 */
  async destroy(): Promise<void> {
    if (!this.closed) {
      this.closed = true;
      this.free();
    }
    await this.backend.destroy();
    await this.keys.delete(this.name);
  }

  private free(): void {
    // 秘密鍵を持つ wasm 側のメモリーをすぐに返す。
    this.account.free();
    this.olmSessions.free();
    this.inbound.free();
    this.trust.free();
  }
}
