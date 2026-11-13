import {
  BackupKeyList,
  OwnSecrets,
  RecoveryKey,
  SecretStorage,
  SsssKey,
  newPasskeySlot,
  newRecoverySlot,
  type JsonObject,
  type KeySlot,
  type ResumePlan,
  type StartupCheck,
  type UnlockedSecretStorage,
} from "@zoo/crypto-wasm";

import type { CryptoStore } from "./store/cryptoStore";

/** `POST /api/e2ee/secret_storage/keys` の前提の値（E2EE のリセットのとき）。 */
export interface ResetExpectation {
  expectedDefaultKeyId: string;
  expectedSecretsRevision: number | null;
}

export type UnlockSecret =
  | { kind: "recovery"; recoveryKey: string }
  | { kind: "passkey"; credentialId: string; prfOutput: Uint8Array };

export interface PasskeySlotInput {
  /** base64url（パディングなし）の credential ID（`credential.rawId`）。 */
  credentialId: string;
  prfSalt: Uint8Array;
  prfOutput: Uint8Array;
  label: string | null;
}

interface Pending {
  secrets: OwnSecrets;
  ssss: SsssKey;
  /** 初回設定とリセットで作ったスロット。解錠では空。 */
  slots: KeySlot[];
  reset: ResetExpectation | null;
  /** 初回設定かリセットで作った鍵か（解錠で取り出した鍵ではない）。 */
  created: boolean;
}

/**
 * 初回設定と解錠のあいだ、Worker のメモリーにだけ置く秘密（マスター鍵の秘密鍵と SSSS 鍵）。
 * docs/e2ee.md の「初回設定と解錠の流れ」。
 *
 * マスター鍵の秘密鍵と SSSS 鍵は暗号ストアに保存しない。ページを閉じたら消え、続きは SSSS を
 * 開け直して行う（初回設定の再開）。解錠のあとも、鍵スロットの追加に使えるよう、ページを
 * 閉じるか {@link discard} するまで持つ。
 */
export class SetupSession {
  private pending: Pending | null = null;

  constructor(private readonly userId: () => string) {}

  get hasSecrets(): boolean {
    return this.pending !== null;
  }

  /**
   * 初回設定（`reset` が `null`）か E2EE のリセットを始める。クロス署名鍵、バックアップ鍵、SSSS 鍵、
   * リカバリーキーを作り、リカバリーキーのスロットを作る。返したリカバリーキーを利用者に控えてもらう。
   */
  begin(reset: ResetExpectation | null): { recoveryKey: string; keyId: string } {
    this.discard();
    const userId = this.userId();
    const secrets = OwnSecrets.generate(userId);
    const ssss = SsssKey.generate();
    const recovery = RecoveryKey.generate();
    try {
      const slot = { ...newRecoverySlot(userId, ssss, recovery), label: null };
      this.pending = { secrets, ssss, slots: [slot], reset, created: true };
      return { recoveryKey: recovery.toDisplay(), keyId: ssss.keyId };
    } finally {
      recovery.free();
    }
  }

  /** 初回設定にパスキーのスロットを足す。同じパスキーのスロットがあれば置き換える。 */
  addPasskeySlot(input: PasskeySlotInput): void {
    const pending = this.created();
    const slot = this.newPasskeySlot(input);
    pending.slots = [
      ...pending.slots.filter((existing) => existing.credential_id !== input.credentialId),
      slot,
    ];
  }

  /** 解錠したあとに、既定の SSSS 鍵のパスキーのスロットを作る（`POST /api/e2ee/secret_storage/slots`）。 */
  newPasskeySlot(input: PasskeySlotInput): KeySlot {
    const pending = this.require();
    return {
      ...newPasskeySlot(
        this.userId(),
        pending.ssss,
        input.credentialId,
        input.prfSalt,
        input.prfOutput,
      ),
      label: input.label,
    };
  }

  /** 初回設定で作ったスロット（保存の再試行で、サーバーにないものを足す）。 */
  pendingSlots(): KeySlot[] {
    return [...this.created().slots];
  }

  /** `POST /api/e2ee/secret_storage/keys` の本文。 */
  secretStorageRequest(): JsonObject {
    const pending = this.created();
    const crossSigning = pending.secrets.crossSigning;
    const backupKey = pending.secrets.backupKey;
    const previous = new BackupKeyList();
    try {
      return pending.ssss.keysRequest(
        {
          expected_default_key_id: pending.reset?.expectedDefaultKeyId ?? null,
          expected_secrets_revision: pending.reset?.expectedSecretsRevision ?? null,
        },
        crossSigning,
        backupKey,
        previous,
        pending.slots,
      );
    } finally {
      crossSigning.free();
      backupKey.free();
      previous.free();
    }
  }

  /** `GET /api/e2ee/secret_storage` の応答と、パスキーかリカバリーキーで SSSS を開ける。 */
  unlock(
    response: JsonObject,
    secret: UnlockSecret,
  ): { keyId: string; secretsRevision: number | null } {
    this.discard();
    const storage = SecretStorage.fromJson(this.userId(), response);
    if (!storage) throw new SetupError("NoSecretStorage", "SSSS がまだありません");
    try {
      const unlocked =
        secret.kind === "recovery"
          ? unlockWithRecoveryKey(storage, secret.recoveryKey)
          : storage.unlockWithPasskey(secret.credentialId, secret.prfOutput);
      try {
        const secrets = OwnSecrets.fromUnlocked(unlocked);
        const ssss = unlocked.key();
        this.pending = { secrets, ssss, slots: [], reset: null, created: false };
        return { keyId: ssss.keyId, secretsRevision: unlocked.secretsRevision ?? null };
      } finally {
        unlocked.free();
      }
    } finally {
      storage.free();
    }
  }

  /**
   * サーバーの状態と比べ、続きに送る要求を決める。リセットなら差し替える要求を作る。
   * 鍵が違えば `code` が `ServerKeyMismatch` のエラーになる。
   */
  plan(store: CryptoStore, keysQuery: JsonObject, versions: JsonObject): ResumePlan {
    const pending = this.require();
    const deviceKeys = store.deviceKeys();
    return pending.reset
      ? pending.secrets.planReset(keysQuery, versions, deviceKeys)
      : pending.secrets.planResume(keysQuery, versions, deviceKeys);
  }

  /** 続きの保存が終わったら、自分の鍵を暗号ストアに保存する。リセットの前提はここで外す。 */
  async finish(store: CryptoStore, secretsRevision: number | null): Promise<void> {
    const pending = this.require();
    await store.saveOwnKeys(pending.secrets, secretsRevision);
    pending.reset = null;
    pending.slots = [];
  }

  /** 暗号ストアの自分の鍵とサーバーの状態から、起動時にすることを決める。まだ解錠していなければ `null`。 */
  async startupCheck(
    store: CryptoStore,
    keysQuery: JsonObject,
    versions: JsonObject,
  ): Promise<StartupCheck | null> {
    const own = await store.ownKeys();
    if (!own) return null;
    try {
      return own.keys.startupCheck(keysQuery, versions, store.deviceKeys());
    } finally {
      own.keys.free();
    }
  }

  /**
   * 開けた SSSS 鍵で、読み直した `GET /api/e2ee/secret_storage` の応答を開ける（失効の始めと再開。
   * #362）。既定の鍵が開けた鍵と違えば `SsssKeyMismatch`。使い終わったら `free()` する。
   */
  unlockedStorage(response: JsonObject): UnlockedSecretStorage {
    const pending = this.require();
    const storage = SecretStorage.fromJson(this.userId(), response);
    if (!storage) throw new SetupError("NoSecretStorage", "SSSS がまだありません");
    try {
      return storage.unlockWithKey(pending.ssss);
    } finally {
      storage.free();
    }
  }

  /** メモリーの秘密を捨てる。 */
  discard(): void {
    if (!this.pending) return;
    this.pending.secrets.free();
    this.pending.ssss.free();
    this.pending = null;
  }

  private require(): Pending {
    if (!this.pending) throw new SetupError("NoSecrets", "SSSS を開けていません");
    return this.pending;
  }

  private created(): Pending {
    const pending = this.require();
    if (!pending.created) throw new SetupError("NoSecrets", "初回設定を始めていません");
    return pending;
  }
}

/** Worker の中の初回設定と解錠の誤り（`code` で分岐する）。 */
export class SetupError extends Error {
  constructor(
    readonly code: "NoSecretStorage" | "NoSecrets",
    message: string,
  ) {
    super(message);
    this.name = "SetupError";
  }
}

function unlockWithRecoveryKey(storage: SecretStorage, input: string) {
  const recovery = RecoveryKey.parse(input);
  try {
    return storage.unlockWithRecoveryKey(recovery);
  } finally {
    recovery.free();
  }
}
