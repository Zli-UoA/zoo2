import {
  type BackupDecryptionKey,
  type Distribution,
  KeySnapshot,
  RecoveryKey,
  Revocation,
  type JsonObject,
  type KeySlot,
  type RevocationScope,
  type RevocationSignatures,
  type RevocationStage,
  type RevocationSummary,
  newPasskeySlot,
  newRecoverySlot,
} from "@zoo/crypto-wasm";

import { REVOCATION_CHANGED, REVOCATION_PENDING } from "./protocol";
import type { PasskeySlotInput, SetupSession } from "./setupSession";
import type { CryptoStore } from "./store/cryptoStore";

/** `revocationBegin` に渡すサーバーの値。 */
export interface RevocationBeginInput {
  scope: RevocationScope;
  /** `GET /api/e2ee/secret_storage`（始める前に開け直したもの）。 */
  secretStorage: JsonObject;
  /** 自分だけを問い合わせた `keys/query`。 */
  keysQuery: JsonObject;
  /** `GET /api/e2ee/key_backup/versions`。 */
  versions: JsonObject;
  /** `GET /api/e2ee/tofu_pins` のすべてのピン。 */
  pins: JsonObject[];
  /** ピンか記録がある相手の `keys/query`。 */
  targetsQuery: JsonObject;
  /** 手順 1 で削除した端末。 */
  deletedDevices: string[];
}

function coded(code: string, message: string): Error {
  return Object.assign(new Error(message), { code });
}

/** `trustEvaluate` の結果（端末の一覧は返さない。送信は #361 の配布で行う）。 */
export type TrustEvaluation = Omit<Distribution, "devices">;

/**
 * 失効の手順（docs/e2ee.md の「失効の手順」。#362）のうち、Worker の中で行うもの。
 *
 * マスター鍵の秘密鍵と SSSS 鍵は Worker のメモリーにだけ置く。保存が 1 つ済むたびに、進み具合
 * （`Revocation.pickle`。新しい自己署名鍵、ユーザー署名鍵、バックアップ鍵を含む）を暗号ストアに
 * 保存し、ページを閉じても SSSS を開け直して続けられるようにする。
 */
export class RevocationSession {
  private revocation: Revocation | null = null;
  /** 2〜5 で作る新しい SSSS 鍵のスロット。 */
  private slots: KeySlot[] = [];
  /** 移行する古い版の鍵（版の番号ごと）。 */
  private migration = new Map<string, BackupDecryptionKey>();

  constructor(
    private readonly setup: SetupSession,
    private readonly userId: () => string,
  ) {}

  /** 保存した失効の進み具合。なければ `null`。 */
  status(store: CryptoStore): Promise<RevocationSummary | null> {
    return store.revocationSummary();
  }

  /** 失効を始める。SSSS を開けてあること（`unlock`）が前提。 */
  async begin(store: CryptoStore, input: RevocationBeginInput): Promise<RevocationSummary> {
    // 終わっていない失効を黙って捨てない（実行中の続きが、新しい失効に保存を当ててしまうため）。
    // やり直すときは、先に `abandon` するか、`resume` で続ける。
    if (await store.revocationSummary()) {
      throw coded(REVOCATION_PENDING, "終わっていない失効があります");
    }
    this.discard();
    const unlocked = this.setup.unlockedStorage(input.secretStorage);
    const own = await store.ownKeys();
    try {
      this.revocation = Revocation.begin(
        {
          scope: input.scope,
          keys_query: input.keysQuery,
          versions: input.versions,
          pins: input.pins,
          targets_query: input.targetsQuery,
          deleted_devices: input.deletedDevices,
        },
        unlocked,
        store.trustStore,
        own?.keys.backupKey,
        own?.keys.userSigning,
      );
    } finally {
      unlocked.free();
      own?.keys.free();
    }
    await store.saveRevocation(this.revocation);
    return this.revocation.summary();
  }

  /** 保存した失効を、開け直した SSSS で続ける。2〜5 を SSSS の保存の前に止めたら `RevocationRestart`。 */
  async resume(store: CryptoStore, secretStorage: JsonObject): Promise<RevocationSummary> {
    this.discard();
    const unlocked = this.setup.unlockedStorage(secretStorage);
    try {
      this.revocation = await store.resumeRevocation(unlocked);
    } finally {
      unlocked.free();
    }
    if (!this.revocation) throw new Error("続ける失効がありません");
    return this.revocation.summary();
  }

  /** 2〜5：新しい SSSS 鍵のリカバリーキーを作る。返したリカバリーキーを利用者に控えてもらう。 */
  newRecoveryKey(): string {
    const revocation = this.require();
    const recovery = RecoveryKey.generate();
    const ssss = revocation.ssssKey();
    try {
      const slot = { ...newRecoverySlot(this.userId(), ssss, recovery), label: null };
      this.slots = [...this.slots.filter((s) => s.kind !== "recovery"), slot];
      return recovery.toDisplay();
    } finally {
      recovery.free();
      ssss.free();
    }
  }

  /** 2〜5：新しい SSSS 鍵のパスキーのスロットを作る（新しい `prf_salt`）。 */
  addPasskeySlot(input: PasskeySlotInput): void {
    const revocation = this.require();
    const ssss = revocation.ssssKey();
    try {
      const slot = {
        ...newPasskeySlot(this.userId(), ssss, input.credentialId, input.prfSalt, input.prfOutput),
        label: input.label,
      };
      this.slots = [
        ...this.slots.filter((existing) => existing.credential_id !== input.credentialId),
        slot,
      ];
    } finally {
      ssss.free();
    }
  }

  /** 新しい SSSS 鍵の ID（2〜5 の 409 のあとで、自分の保存が済んでいたかを見る）。 */
  keyId(): string {
    return this.require().summary().key_id;
  }

  secretStorageRequest(): JsonObject {
    return this.require().secretStorageRequest(this.slots);
  }

  async secretStorageSaved(store: CryptoStore, secretsRevision: number | null): Promise<void> {
    const revocation = this.require();
    revocation.secretStorageSaved(secretsRevision);
    await store.saveRevocation(revocation);
    // 1〜5 では既定の SSSS 鍵が新しい鍵に変わった。解錠で持っている古い SSSS 鍵で、「暗号化」タブから
    // スロットを作ると、新しい鍵を開けないスロットで既存のスロットを置き換えてしまう。捨てて、
    // 開け直してもらう。
    if (revocation.summary().scope === "full") this.setup.discard();
  }

  crossSigningRequest(): JsonObject {
    return this.require().crossSigningRequest();
  }

  /** クロス署名鍵を保存した。記録と自分の鍵を新しい鍵にそろえる。 */
  async crossSigningSaved(store: CryptoStore, secretsRevision: number | null): Promise<void> {
    const revocation = this.require();
    revocation.advance("cross_signing_saved");
    await store.applyRevocation(revocation, secretsRevision);
  }

  signatures(store: CryptoStore): RevocationSignatures {
    return this.require().signatures(store.deviceKeys());
  }

  async advance(store: CryptoStore, stage: RevocationStage): Promise<void> {
    const revocation = this.require();
    revocation.advance(stage);
    await store.saveRevocation(revocation);
  }

  backupVersionRequest(): JsonObject {
    return this.require().backupVersionRequest();
  }

  async backupCreated(store: CryptoStore, version: string): Promise<void> {
    const revocation = this.require();
    revocation.backupCreated(version);
    await store.saveRevocation(revocation);
  }

  /** 新しい版に移す古い版（`GET /api/e2ee/key_backup/versions` を読み直したもの）。 */
  versionsToMigrate(versions: JsonObject): string[] {
    this.freeMigration();
    for (const { version, key } of this.require().versionsToMigrate(versions)) {
      this.migration.set(version, key);
    }
    return [...this.migration.keys()];
  }

  /**
   * 古い版の行を、新しい版の公開鍵で暗号化し直す。捨てた行の数を記録して保存する（古い版を消したあとで
   * 止まっても、再開した完了の画面に出せるように）。
   */
  async reencrypt(
    store: CryptoStore,
    version: string,
    rows: JsonObject[],
  ): Promise<{ rows: JsonObject[]; dropped: number }> {
    const key = this.migration.get(version);
    if (!key) throw new Error(`版 ${version} の鍵がありません`);
    const revocation = this.require();
    const result = revocation.reencryptRows(key, rows);
    revocation.recordDropped(version, result.dropped);
    await store.saveRevocation(revocation);
    return result;
  }

  /**
   * 保存のあとに読み直した値を、作った鍵と比べる。一致したら失効の記録を消し、メモリーの秘密を捨てる。
   * 一致しなければ `ServerKeyMismatch`（SSSS を開け直してやり直す）。
   */
  async verify(
    store: CryptoStore,
    secretStorage: JsonObject,
    keysQuery: JsonObject,
    currentVersion: JsonObject,
  ): Promise<void> {
    this.require().verify(secretStorage, keysQuery, currentVersion);
    await store.clearRevocation();
    this.discard();
    this.setup.discard();
  }

  /** 失効の記録を消す（やり直すとき）。 */
  async abandon(store: CryptoStore): Promise<void> {
    this.discard();
    await store.clearRevocation();
  }

  summary(): RevocationSummary {
    return this.require().summary();
  }

  /**
   * 保存と段の更新の要求が、今持つ失効へのものかを確かめる。画面を離れて別の失効を始めたあとに、
   * 前の実行の応答が、新しい失効の段を進めないようにするため。違えば何も変えずに `RevocationChanged`。
   */
  check(revocationId: string): void {
    if (this.revocation?.summary().revocation_id !== revocationId) {
      throw coded(REVOCATION_CHANGED, "実行を始めた失効が、別の失効に置き換わりました");
    }
  }

  discard(): void {
    this.freeMigration();
    this.revocation?.free();
    this.revocation = null;
    this.slots = [];
  }

  private freeMigration(): void {
    for (const key of this.migration.values()) key.free();
    this.migration.clear();
  }

  private require(): Revocation {
    if (!this.revocation) throw new Error("失効を始めていません");
    return this.revocation;
  }
}

/** 配布先の判定を行い、記録を保存する（他のユーザーの検証状態の表示。#362）。 */
export async function evaluateTrust(
  store: CryptoStore,
  keysQuery: JsonObject,
  userIds: string[],
): Promise<TrustEvaluation> {
  const snapshot = KeySnapshot.fromJson({ ...keysQuery, user_ids: [...userIds, store.userId] });
  const own = await store.ownKeys();
  const userSigning = own?.keys.userSigning ?? null;
  try {
    const result = userSigning
      ? store.trustStore.evaluateWithUserSigning(snapshot, store.deviceId, userSigning)
      : store.trustStore.evaluate(snapshot, store.deviceId);
    await store.saveTrustStore();
    for (const device of result.devices) device.free();
    return { users: result.users, new_pins: result.new_pins, warnings: result.warnings };
  } finally {
    snapshot.free();
    userSigning?.free();
    own?.keys.free();
  }
}
