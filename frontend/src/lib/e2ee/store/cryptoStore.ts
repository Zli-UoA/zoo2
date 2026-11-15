import {
  BackupUploadState,
  DeviceAccount,
  type BackupRow,
  type BackupUpload,
  InboundSessionStore,
  KeySnapshot,
  OlmSessions,
  OwnKeys,
  Revocation,
  TrustStore,
  type OwnSecrets,
  type RevocationSummary,
  type UnlockedSecretStorage,
  type JsonObject,
  type SenderTrust,
  type SessionRef,
  type VerificationMessage,
} from "@zoo/crypto-wasm";

import {
  inboundSessionKey,
  type CryptoBackend,
  type CryptoStoreWrite,
  type InboundSessionKey,
} from "./backend";
import { BACKUP_IMPORT_MAX_PAGES, BACKUP_IMPORT_PAGE_SIZE } from "../backupImportLimits";
import { masterPublicKey } from "../masterKey";
import { ENCRYPTED_CHANNEL_META_PREFIX, MegolmChannels, RECIPIENTS_META_PREFIX } from "./megolm";
import { SasVerifications } from "./verification";
import {
  StorageKeyUnavailableError,
  WRAPPED_STORAGE_KEY_META,
  indexedDbWrappingKeys,
  loadStorageKey,
  type WrappingKeyStore,
} from "./storageKey";

export const META_USER_ID = "user_id";
export const META_DEVICE_ID = "device_id";
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

/**
 * 鍵バックアップのアップロードの状態（`BackupUploadState.toJson`。見た版の最大を持ち、版の
 * 巻き戻りに気付くため）。秘密は入らない。
 */
const META_BACKUP_UPLOAD = "backup_upload";

/**
 * 鍵バックアップからの取り込みの進み具合（{@link BackupImportProgress} の JSON）。秘密は入らない。
 * 途中で止まったら、同じ版なら続きから取り込み、版が変わったら始めから取り込む。
 */
const META_BACKUP_IMPORT = "backup_import";

/**
 * 端末を作り直すとき（{@link CryptoStore.replaceDevice}）に引き継ぐ `meta`。ほか（`own_keys`、
 * `secrets_revision`、`revocation`、`backup_import`、`outbound_olm:*` など）は捨てる。前置きで
 * 引き継ぐもの（`recipients:*`、`encrypted_channel:*`）は `replaceDevice` で見る。
 */
const CARRIED_META = new Set([WRAPPED_STORAGE_KEY_META, META_USER_ID, META_BACKUP_UPLOAD]);

interface BackupImportProgress {
  /** 取り込んでいる（取り込んだ）版。 */
  version: string;
  /** 次に読むページのカーソル（`GET /api/e2ee/key_backup/keys` の `after`）。 */
  after: string | null;
  /**
   * 最後のページまで取り込んだ。1 行も読まなかったときは済みにしない（サーバーが一時的に、または
   * わざと空を返しても、その版を読み直せるように。空の版の読み直しは 1 回の要求で済む）。
   */
  done: boolean;
  /** この版で読んだ行の数。 */
  rows: number;
  /** この版で読んだページの数（{@link BACKUP_IMPORT_MAX_PAGES} を超えて読まない）。 */
  pages: number;
}

/** {@link CryptoStore.beginBackupImport} の結果。 */
export type BackupImportStart =
  /** `after` のカーソル（`null` なら最初）から読んで取り込む。`pages` はこの版で読んだページの数。 */
  | { kind: "import"; version: string; after: string | null; pages: number }
  /** この版は取り込み終えている。 */
  | { kind: "done"; version: string }
  /** 解錠していない、または今の版がない。 */
  | { kind: "none" }
  /** 今の版を使えない（アップロードの `rejected` と同じ）。取り込まない。 */
  | { kind: "rejected"; code: string };

/** 鍵バックアップへの 1 回のアップロード（`POST /api/e2ee/key_backup/keys`）。 */
export type BackupUploadRequest =
  | {
      kind: "upload";
      /** この要求の ID。応答を受けたら、同じ ID で完了か失敗を知らせる。 */
      uploadId: number;
      version: string;
      secretsRevision: number;
      body: JsonObject;
      count: number;
    }
  /** 送るものがない（未解錠、今の版がない、送信待ちがない）。 */
  | { kind: "none" }
  /**
   * 今の版を使えない（`code` は zoo-crypto の誤り。マスター鍵の署名がない `Signature` や
   * `MissingSignature`、公開鍵が SSSS と違う `BackupKeyMismatch`、版の巻き戻り
   * `BackupVersionRollback` など）。送らない。
   */
  | { kind: "rejected"; code: string };

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

/**
 * `keys/query` の応答（50 人ずつ問い合わせてまとめたもの）。`KeySnapshot.fromJson` にそのまま渡せる。
 */
export type KeysQueryResponse = {
  device_keys: DeviceKeysByUser;
  master_keys: Record<string, JsonObject>;
  self_signing_keys: Record<string, JsonObject>;
  user_signing_keys: Record<string, JsonObject>;
  tofu_pins: Record<string, JsonObject>;
  signatures: JsonObject[];
};

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
  /**
   * 受信時の検証（手順 1〜3）を通った、絵文字 SAS の検証のメッセージ（#368）。画面のスレッドが
   * 送信ユーザーの `keys/query` を取ってから、`sasReceive` で検証を進める。
   */
  verifications: VerificationMessage[];
}

/** 絵文字 SAS の検証のメッセージの `type` の接頭辞。 */
const VERIFICATION_TYPE_PREFIX = "m.key.verification.";

export interface OpenedStore {
  store: CryptoStore;
  /** 保存鍵を開けずに暗号ストアを作り直したとき、前の端末の ID（サーバーから消す）。 */
  replacedDeviceId: string | null;
  /**
   * 前の端末の登録が済んでいたか。済んでいなければ、削除 API の 404 でもすぐには消えたものと
   * しない（登録の応答をなくしただけかもしれない）。
   */
  replacedRegistered: boolean;
}

interface OpenOptions {
  backend: CryptoBackend;
  name: string;
  userId: string;
  reset: () => Promise<CryptoBackend>;
  keys?: WrappingKeyStore;
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
  private backupState: BackupUploadState | null = null;
  /**
   * 応答を待っているアップロード（ID ごと）。要求が重なっても、応答を受けた要求に入れた
   * セッションだけを送信済みにする。
   */
  private readonly pendingUploads = new Map<
    number,
    { upload: BackupUpload; sessions: { key: string; pickle: string }[] }
  >();
  private nextUploadId = 0;
  private broken = false;
  private megolmChannels: MegolmChannels | null = null;
  private sasVerifications: SasVerifications | null = null;

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
  static async open(options: OpenOptions): Promise<OpenedStore> {
    const current = { backend: options.backend };
    try {
      return await CryptoStore.openWith(options, current);
    } catch (error) {
      // 途中で失敗したら、開いた保存先を閉じる（呼び出し側は store を受け取らないので閉じられない）。
      await current.backend.close().catch(() => undefined);
      throw error;
    }
  }

  private static async openWith(
    options: OpenOptions,
    current: { backend: CryptoBackend },
  ): Promise<OpenedStore> {
    let backend = current.backend;
    const keys = options.keys ?? indexedDbWrappingKeys;
    let snapshot = await backend.read();
    let replacedDeviceId: string | null = null;
    let replacedRegistered = false;
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
      replacedRegistered = snapshot.meta[META_REGISTERED] === "1";
      // 開いたままのファイルは消せないので、先に閉じる。
      await backend.close().catch(() => undefined);
      backend = await options.reset();
      current.backend = backend;
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
      let trust: TrustStore;
      try {
        trust = snapshot.trustStore
          ? TrustStore.fromPickle(snapshot.trustStore, account, storageKey)
          : new TrustStore(options.userId);
      } catch (error) {
        // 秘密鍵を持つ wasm 側のメモリーを、失敗してもすぐに返す。
        account.free();
        throw error;
      }
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
      try {
        for (const session of snapshot.olmSessions) {
          store.olmSessions.pushPickle(session.pickle, account, storageKey);
        }
      } catch (error) {
        store.free();
        throw error;
      }
      return { store, replacedDeviceId, replacedRegistered };
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
    try {
      await backend.write({
        meta: {
          [META_USER_ID]: options.userId,
          [META_DEVICE_ID]: account.deviceId,
          [META_REGISTERED]: "0",
        },
        account: account.pickle(storageKey),
        trustStore: trust.pickle(account, storageKey),
      });
    } catch (error) {
      store.free();
      throw error;
    }
    return { store, replacedDeviceId, replacedRegistered };
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

  /** Megolm の送信と復号（#361）。 */
  get megolm(): MegolmChannels {
    this.megolmChannels ??= new MegolmChannels({
      account: this.account,
      inbound: this.inbound,
      trust: () => this.trust,
      storageKey: this.storageKey,
      loadInboundSession: (ref) => this.loadInboundSession(ref),
      write: (changes) => this.write(changes),
      readMeta: async () => (await this.backend.read()).meta,
      ownKeys: () => this.ownKeys(),
    });
    return this.megolmChannels;
  }

  /** 他のユーザーの絵文字 SAS 検証（#368）。検証はメモリーにだけ置く。 */
  get verification(): SasVerifications {
    this.sasVerifications ??= new SasVerifications({
      account: this.account,
      trust: () => this.trust,
      storageKey: this.storageKey,
      write: (changes) => this.write(changes),
      readMeta: async () => (await this.backend.read()).meta,
      ownKeys: () => this.ownKeys(),
      saveTrustStore: () => this.saveTrustStore(),
    });
    return this.sasVerifications;
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
   * `keysQuery` は送信ユーザーと自分の `keys/query` の応答。端末鍵（手順 1）と、送信端末を
   * 検証済みにするかの判定（手順 4。送信ユーザーの信頼したマスター鍵と自己署名鍵）に使う。
   */
  async processToDevice(
    rows: ToDeviceRow[],
    keysQuery: KeysQueryResponse,
  ): Promise<ToDeviceResult> {
    const result: ToDeviceResult = {
      processed: [],
      roomKeys: 0,
      dropped: 0,
      newOlmSessions: 0,
      verifications: [],
    };
    // 自分の別の端末は、暗号ストアの自分の自己署名鍵（解錠して取り出した今の鍵）と、サーバーの
    // 自己署名鍵が同じときだけ検証済みにする（失効で置き換えた古い鍵への巻き戻しを受け入れない）。
    const own = rows.length > 0 ? await this.ownKeys() : null;
    let ownSelfSigningKey: string | null = null;
    if (own) {
      const selfSigning = own.keys.selfSigning;
      ownSelfSigningKey = selfSigning.publicKey;
      selfSigning.free();
      own.keys.free();
    }
    const senders = new SenderTrustCache(this.trust, keysQuery, ownSelfSigningKey);
    try {
      for (const row of rows) {
        if (this.processedToDevice.has(row.id)) {
          result.processed.push(row.id);
          continue;
        }
        const changes: CryptoStoreWrite = { addProcessedToDevice: [row.id] };
        try {
          if (row.type === "m.room.encrypted") {
            await this.processEncrypted(row, keysQuery.device_keys, senders, changes, result);
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
    } finally {
      senders.free();
    }
    return result;
  }

  private async processEncrypted(
    row: ToDeviceRow,
    deviceKeys: DeviceKeysByUser,
    senders: SenderTrustCache,
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
      const type = decrypted.plaintext.type;
      if (typeof type === "string" && type.startsWith(VERIFICATION_TYPE_PREFIX)) {
        // 絵文字 SAS の検証（#368）。Olm のメッセージは 2 回は復号できないので、ここで受信時の検証を
        // 通したものを返し、検証を進めるのは画面のスレッドに任せる。
        try {
          result.verifications.push(decrypted.verifyVerification(this.account, listed ?? null));
        } catch {
          result.dropped += 1;
        }
        return;
      }
      // 手順 4：送信ユーザーの、信頼したマスター鍵で署名された自己署名鍵。信頼できなければ `null`
      // （未解錠、再検証待ち、初めて見た相手など）で、セッションは「未検証の送信者」になる。
      const trust = sender && senderDevice ? senders.get(sender, senderDevice) : null;
      if (senders.takeTrustChanged()) {
        changes.trustStore = this.trust.pickle(this.account, this.storageKey);
      }
      let roomKey;
      try {
        roomKey = decrypted.verifyRoomKey(this.account, listed ?? null, trust);
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
      // 失敗したら、メモリーの記録が保存と食い違うので、開き直させる（`write` が印を付ける）。
      await this.write({
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

  /**
   * 鍵バックアップへ送る次の要求を作る（docs/e2ee.md の「鍵バックアップ」。#360）。
   *
   * `currentVersion` は `GET /api/e2ee/key_backup/versions/current` の応答（なければ `null`）。
   * 今の版の `auth_data` に自分のマスター鍵の署名があり、公開鍵が SSSS のバックアップ鍵と同じで、
   * 前に見た版より古くないときだけ送る。版が変わったら、すべてのセッションを送り直す。
   */
  async prepareBackupUpload(
    currentVersion: JsonObject | null,
    limit: number,
  ): Promise<BackupUploadRequest> {
    const accepted = await this.acceptBackupVersion(currentVersion);
    if (accepted.kind !== "accepted") return accepted;
    const state = accepted.state;

    const sessions = await this.backend.pendingBackupSessions(limit);
    if (sessions.length === 0) return { kind: "none" };
    for (const session of sessions) {
      const ref = JSON.parse(session.key) as [string, string, string];
      const sessionRef = { channel_id: ref[0], sender_key: ref[1], session_id: ref[2] };
      await this.loadInboundSession(sessionRef);
      state.enqueue(sessionRef);
    }
    const upload = state.nextUpload(this.inbound, limit);
    if (!upload) return { kind: "none" };
    const sent = new Set(upload.sessions.map((ref) => inboundSessionKey(ref)));
    const uploadId = ++this.nextUploadId;
    this.pendingUploads.set(uploadId, {
      upload,
      sessions: sessions.filter((session) => sent.has(session.key)),
    });
    return {
      kind: "upload",
      uploadId,
      version: upload.version,
      secretsRevision: upload.secretsRevision,
      body: upload.body,
      count: sent.size,
    };
  }

  /**
   * 今の版を検証して受け入れる（アップロードと取り込みの前に行う。docs/e2ee.md の「鍵バックアップ」）。
   *
   * `auth_data` に自分のマスター鍵の署名があり、公開鍵が SSSS のバックアップ鍵と同じで、前に見た版より
   * 古くないときだけ受け入れる。版が変わったら、すべての受信用セッションを送信待ちに戻す。
   */
  private async acceptBackupVersion(
    currentVersion: JsonObject | null,
  ): Promise<
    | { kind: "accepted"; state: BackupUploadState; version: string }
    | { kind: "none" }
    | { kind: "rejected"; code: string }
  > {
    const own = await this.ownKeys();
    if (!own) return { kind: "none" };
    const state = await this.loadBackupState();
    try {
      state.setSecretsRevision(own.secretsRevision ?? undefined);
      if (!currentVersion) {
        state.suspend();
        return { kind: "none" };
      }
      let change: string;
      const backupKey = own.keys.backupKey;
      try {
        change = state.acceptCurrentVersion(currentVersion, own.keys.masterPublicKey, backupKey);
      } catch (error) {
        const code = (error as { code?: unknown }).code;
        return { kind: "rejected", code: typeof code === "string" ? code : "Unknown" };
      } finally {
        backupKey.free();
      }
      if (change === "changed") {
        try {
          await this.backend.resetBackedUp();
        } catch (error) {
          // 新しい版を見たことは、送信待ちに戻せたあとでだけ記録する。次の呼び出しで、保存した
          // 状態から読み直して、もう一度戻す。
          this.backupState?.free();
          this.backupState = null;
          throw error;
        }
      }
      await this.saveBackupState(state);
    } finally {
      own.keys.free();
    }
    const verified = state.currentVersion();
    if (!verified) return { kind: "none" };
    return { kind: "accepted", state, version: verified.version };
  }

  /**
   * 鍵バックアップからの取り込みを始める（docs/e2ee.md の「新しい端末での解錠」の手順 5）。
   *
   * `currentVersion` は `GET /api/e2ee/key_backup/versions/current` の応答。アップロードと同じ検証を
   * 通った今の版だけから取り込む。途中で止まった取り込みは、同じ版なら続きのカーソルから始める。
   * その版を取り込み終えていれば `done`。
   */
  async beginBackupImport(currentVersion: JsonObject | null): Promise<BackupImportStart> {
    const accepted = await this.acceptBackupVersion(currentVersion);
    if (accepted.kind !== "accepted") return accepted;
    const progress = await this.backupImportProgress();
    if (progress?.version !== accepted.version) {
      return { kind: "import", version: accepted.version, after: null, pages: 0 };
    }
    if (progress.done) return { kind: "done", version: accepted.version };
    return {
      kind: "import",
      version: accepted.version,
      after: progress.after,
      pages: progress.pages,
    };
  }

  /**
   * `GET /api/e2ee/key_backup/keys` の 1 ページを取り込む。`next` は応答の `next`（最後のページなら
   * `null`）で、取り込んだセッションと一緒に進み具合として保存する。
   *
   * 取り込んだセッションは「バックアップ由来」として保存し、Olm 由来のセッションを置き換えない
   * （`InboundSessionStore.importBackupSession`）。復号できない行、平文が行と合わない行、`session_key`
   * から導いた `session_id` が合わない行は捨てる。同じ版には上げ直さない（送信済みとして保存する）。
   */
  async importBackupPage(
    version: string,
    rows: JsonObject[],
    next: string | null,
  ): Promise<{ imported: number; dropped: number }> {
    // 受け入れた今の版からだけ取り込む（取り込みの途中で版が変わったら、始め直させる）。
    const accepted = this.backupState?.currentVersion();
    if (!accepted || accepted.version !== version) {
      throw new Error("鍵バックアップの今の版が変わりました");
    }
    if (rows.length > BACKUP_IMPORT_PAGE_SIZE) {
      throw new Error("鍵バックアップの 1 ページの行が多すぎます");
    }
    const previous = await this.backupImportProgress();
    const counted = previous?.version === version ? previous : { rows: 0, pages: 0 };
    if (counted.pages >= BACKUP_IMPORT_MAX_PAGES) {
      throw new Error("鍵バックアップのページが多すぎます");
    }
    const own = await this.ownKeys();
    if (!own) throw new Error("解錠していません");
    const backupKey = own.keys.backupKey;
    const sessions = new Map<InboundSessionKey, string>();
    let imported = 0;
    let dropped = 0;
    try {
      for (const row of rows) {
        let session;
        try {
          session = backupKey.decryptRow(row as unknown as BackupRow);
        } catch {
          dropped += 1;
          continue;
        }
        const ref: SessionRef = {
          channel_id: session.channelId,
          sender_key: session.senderKey,
          session_id: session.sessionId,
        };
        try {
          await this.loadInboundSession(ref);
        } catch (error) {
          session.free();
          throw error;
        }
        let outcome;
        try {
          // `session` は渡すと wasm 側に移る（ここで解放される）。
          outcome = this.inbound.importBackupSession(session);
        } catch {
          dropped += 1;
          continue;
        }
        if (outcome === "kept") continue;
        const pickle = this.inbound.pickleSession(ref, this.account, this.storageKey);
        if (pickle) sessions.set(inboundSessionKey(ref), pickle);
        imported += 1;
      }
    } catch (error) {
      // 取り込んだ行がメモリーに残っている。同じストアで試し直すと、その行は「保存済み」として
      // 扱われ（`kept`）、保存されないまま進み具合だけが進むので、開き直すまで使わない。
      this.broken = true;
      throw error;
    } finally {
      backupKey.free();
      own.keys.free();
    }
    const total = counted.rows + rows.length;
    // 1 行も読まずに最後のページまで来たら、次は始めから読み直すので、ページの数も 0 に戻す
    // （空の版を何度読み直しても、ページの上限に届かないように）。
    const emptyVersion = next === null && total === 0;
    const progress: BackupImportProgress = {
      version,
      after: next,
      done: next === null && total > 0,
      rows: total,
      pages: emptyVersion ? 0 : counted.pages + 1,
    };
    await this.write({
      meta: { [META_BACKUP_IMPORT]: JSON.stringify(progress) },
      inboundSessions: [...sessions].map(([key, pickle]) => ({ key, pickle, fromBackup: true })),
    });
    return { imported, dropped };
  }

  private async backupImportProgress(): Promise<BackupImportProgress | null> {
    const { meta } = await this.backend.read();
    const saved = meta[META_BACKUP_IMPORT];
    if (!saved) return null;
    try {
      const value = JSON.parse(saved) as Partial<BackupImportProgress>;
      if (typeof value.version !== "string" || typeof value.done !== "boolean") return null;
      const count = (n: unknown) =>
        typeof n === "number" && Number.isSafeInteger(n) && n >= 0 ? n : 0;
      return {
        version: value.version,
        after: typeof value.after === "string" ? value.after : null,
        done: value.done,
        rows: count(value.rows),
        pages: count(value.pages),
      };
    } catch {
      return null;
    }
  }

  /**
   * {@link prepareBackupUpload} の `uploadId` の要求が成功した。その要求に入れたセッションだけを
   * 送信済みにする（送ったあとに書き換わったものは除く）。
   */
  async backupUploaded(uploadId: number): Promise<void> {
    const pending = this.pendingUploads.get(uploadId);
    if (!pending) return;
    this.pendingUploads.delete(uploadId);
    try {
      this.backupState?.uploadSucceeded(pending.upload);
      await this.backend.markBackedUp(pending.sessions);
      if (this.backupState) await this.saveBackupState(this.backupState);
    } finally {
      pending.upload.free();
    }
  }

  /** `uploadId` の要求が失敗した。入れたセッションは送信待ちのまま残す。 */
  discardPendingUpload(uploadId?: number): void {
    const ids = uploadId === undefined ? [...this.pendingUploads.keys()] : [uploadId];
    for (const id of ids) {
      this.pendingUploads.get(id)?.upload.free();
      this.pendingUploads.delete(id);
    }
  }

  private async loadBackupState(): Promise<BackupUploadState> {
    if (this.backupState) return this.backupState;
    const { meta } = await this.backend.read();
    const saved = meta[META_BACKUP_UPLOAD];
    let state: BackupUploadState;
    try {
      state = saved
        ? BackupUploadState.fromJson(saved, this.userId)
        : new BackupUploadState(this.userId);
    } catch {
      state = new BackupUploadState(this.userId);
    }
    this.backupState = state;
    return state;
  }

  private async saveBackupState(state: BackupUploadState): Promise<void> {
    await this.backend.write({ meta: { [META_BACKUP_UPLOAD]: state.toJson() } });
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
   * 置き換えて（ピンを保存できるまでは登録待ちで）保存し、`replace: true` で送るピンと、マスター鍵への
   * 署名を返す。保存できたら {@link confirmAcceptedMasterKey} を呼ぶ。解錠していなければ投げる。
   */
  /**
   * 受け入れ（#362）か SAS（#368）のピンと署名をサーバーに保存できた。記録の登録待ちを外して保存する
   * （ピンを置けるまでは、記録だけで配らない）。
   */
  async confirmAcceptedMasterKey(targetUserId: string, masterKey: string): Promise<void> {
    this.trust.confirmAcceptedMasterKey(targetUserId, masterKey);
    await this.saveTrustStore();
  }

  async acceptMasterKey(targetUserId: string, masterKey: JsonObject, expectedMasterKey: string) {
    // 利用者に指紋を見せた鍵にだけ署名する（再検証の確認。docs/e2ee.md の「他のユーザーの再検証」）。
    if (masterPublicKey(masterKey) !== expectedMasterKey) {
      throw new Error("確認した鍵と、署名するマスター鍵が違います");
    }
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

  /**
   * 端末が別の端末から削除されていた（docs/e2ee.md の「端末が削除されていたとき」。呼び出し側が
   * `GET /api/e2ee/devices` で確かめてから呼ぶ）。暗号ストアを消さずに、端末アカウントだけを作り直す。
   *
   * サーバーの応答だけで信頼の記録と送っていないセッションを失わないよう、ユーザーに属する記録は
   * 新しい端末アカウントで封をし直して引き継ぐ（封の AAD に `device_id` が入るため）。
   *
   * - 引き継ぐ：`TrustStore`、受信用セッション（鍵バックアップに送ったかどうかも）、鍵バックアップの
   *   アップロードの状態（見た版の最大）、配布先とチャンネルの記録（`recipients:*`、`encrypted_channel:*`）
   * - 捨てる：Olm セッション（送信用を含む）、処理済みの to-device、解錠した自分の鍵と
   *   `secrets_revision`（利用者は解錠し直す）、終わっていない失効、鍵バックアップからの取り込みの進み具合
   * - 開けない受信用セッションは捨てる。
   *
   * 1 つのトランザクションで書く。呼んだあとは、この store を閉じて開き直す。
   */
  async replaceDevice(): Promise<{ deviceId: string; carried: number; dropped: number }> {
    const { meta } = await this.backend.read();
    const rows = await this.backend.readAllInboundSessions();
    const account = new DeviceAccount(this.userId, crypto.randomUUID());
    try {
      account.generateOneTimeKeys(account.maxNumberOfOneTimeKeys);
      account.generateFallbackKey();
      const inboundSessions: { key: string; pickle: string }[] = [];
      const droppedInboundSessions: string[] = [];
      for (const row of rows) {
        // 1 行ずつ開き直す（すべてをメモリーに載せない）。
        const sessions = new InboundSessionStore();
        try {
          const ref = sessions.restoreSession(row.pickle, this.account, this.storageKey);
          const pickle = sessions.pickleSession(ref, account, this.storageKey);
          if (pickle) inboundSessions.push({ key: row.key, pickle });
          else droppedInboundSessions.push(row.key);
        } catch {
          droppedInboundSessions.push(row.key);
        } finally {
          sessions.free();
        }
      }
      const carriedMeta: Record<string, string> = {};
      for (const [key, value] of Object.entries(meta)) {
        if (
          CARRIED_META.has(key) ||
          key.startsWith(RECIPIENTS_META_PREFIX) ||
          key.startsWith(ENCRYPTED_CHANNEL_META_PREFIX)
        ) {
          carriedMeta[key] = value;
        }
      }
      carriedMeta[META_USER_ID] = this.userId;
      carriedMeta[META_DEVICE_ID] = account.deviceId;
      carriedMeta[META_REGISTERED] = "0";
      await this.backend.replaceDevice({
        meta: carriedMeta,
        account: account.pickle(this.storageKey),
        trustStore: this.trust.pickle(account, this.storageKey),
        inboundSessions,
        droppedInboundSessions,
      });
      return {
        deviceId: account.deviceId,
        carried: inboundSessions.length,
        dropped: droppedInboundSessions.length,
      };
    } finally {
      account.free();
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

  /** 暗号ストアを消す（ログアウト）。保存鍵を包む鍵も消す（{@link destroyCryptoStore}）。 */
  async destroy(): Promise<void> {
    if (!this.closed) {
      this.closed = true;
      this.free();
    }
    await destroyCryptoStore({
      removeFiles: () => this.backend.destroy(),
      deleteWrappingKey: () => this.keys.delete(this.name),
    });
  }

  private free(): void {
    // 秘密鍵を持つ wasm 側のメモリーをすぐに返す。
    this.megolmChannels?.free();
    this.sasVerifications?.close();
    this.megolmChannels = null;
    this.account.free();
    this.olmSessions.free();
    this.inbound.free();
    this.trust.free();
    this.discardPendingUpload();
    this.backupState?.free();
    this.backupState = null;
  }
}

/**
 * 1 回の to-device の処理で使う、送信ユーザーごとの {@link TrustStore.senderTrust} の結果。
 *
 * 判定（`senderTrust`）は記録を変えることがある（規則 3 の署名や共有ピンの採用）。変わったかは
 * 保存した文字列と比べて調べ、変わったら同じ行の書き込みで記録も保存する。
 */
class SenderTrustCache {
  private snapshot: KeySnapshot | null = null;
  private readonly results = new Map<string, SenderTrust | null>();
  private saved: string;

  constructor(
    private readonly trust: TrustStore,
    private readonly keysQuery: KeysQueryResponse,
    /** 暗号ストアの自分の自己署名鍵の公開鍵（未解錠なら `null`）。 */
    private readonly ownSelfSigningKey: string | null,
  ) {
    this.saved = trust.toJson();
  }

  get(sender: string, senderDevice: string): SenderTrust | null {
    const key = JSON.stringify([sender, senderDevice]);
    if (!this.results.has(key)) {
      let trust: SenderTrust | null = null;
      try {
        this.snapshot ??= KeySnapshot.fromJson(this.keysQuery as unknown as JsonObject);
        trust = this.trust.senderTrust(this.snapshot, sender, senderDevice, this.ownSelfSigningKey);
      } catch {
        // 応答を読めなければ、確かめられないものとして扱う（「未検証の送信者」）。
        trust = null;
      }
      this.results.set(key, trust);
    }
    return this.results.get(key) ?? null;
  }

  /** 前に呼んでから記録が変わったか。 */
  takeTrustChanged(): boolean {
    const now = this.trust.toJson();
    if (now === this.saved) return false;
    this.saved = now;
    return true;
  }

  free(): void {
    this.snapshot?.free();
    this.snapshot = null;
  }
}

/**
 * 暗号ストアを消す。保存鍵を包む鍵を先に消し、失敗してもファイルの削除を試す。
 *
 * ファイルの削除だけが失敗して残っても、包む鍵がなければ保存鍵を開けないので、どの値も読めない。
 * Push を復号する Service Worker（`lib/e2ee/push/`）は、ログアウトの削除待ち（localStorage）を
 * 読めないので、残ったファイルからログアウトしたユーザーの本文を復号しないよう、この順にする。
 * 次に同じユーザーで開くと、保存鍵を開けない暗号ストアとして作り直す（`CryptoStore.open`）。
 */
export async function destroyCryptoStore(steps: {
  removeFiles: () => Promise<void>;
  deleteWrappingKey: () => Promise<void>;
}): Promise<void> {
  let failure: unknown = null;
  try {
    await steps.deleteWrappingKey();
  } catch (error) {
    failure = error;
  }
  try {
    await steps.removeFiles();
  } catch (error) {
    failure ??= error;
  }
  if (failure !== null) throw failure;
}
