import type {
  EncryptedMessage,
  IdentityKeys,
  JsonObject,
  KeySlot,
  NewTofuPin,
  OutgoingMessage,
  ResumePlan,
  StartupCheck,
} from "@zoo/crypto-wasm";

import type {
  ClaimedKeys,
  DecryptOutcome,
  RecipientSummary,
  SharePlan,
  ShareResult,
} from "./store/megolm";

import type { PasskeySlotInput, ResetExpectation, UnlockSecret } from "./setupSession";
import type { BackupUploadRequest } from "./store/cryptoStore";

import type {
  DeviceKeysByUser,
  KeyUpload,
  RegistrationRequest,
  ToDeviceResult,
  ToDeviceRow,
} from "./store/cryptoStore";

export type { DeviceKeysByUser, KeyUpload, RegistrationRequest, ToDeviceResult, ToDeviceRow };
export type { ClaimedKeys, DecryptOutcome, RecipientSummary, SharePlan, ShareResult };
export type { KeySlot, PasskeySlotInput, ResetExpectation, ResumePlan, StartupCheck, UnlockSecret };

/**
 * 暗号の Worker（`crypto.worker.ts`）とのやり取りの型。
 *
 * 鍵と Olm、Megolm の状態は Worker の中だけに置き、メインスレッドには結果の JSON だけを返す
 * （docs/e2ee.md の「共有の暗号コア」）。暗号ストアは Worker が 1 つだけ開く（#359）。
 */
export type CryptoOperation =
  | {
      op: "generateDeviceKeys";
      /** 小文字でハイフン区切りの UUID。 */
      userId: string;
      /** UUID v4。 */
      deviceId: string;
    }
  | {
      /** `namespace`（`resolveOfflineNamespace`）の暗号ストアを開く。なければ端末を作る。 */
      op: "open";
      namespace: string;
      userId: string;
    }
  | { op: "registrationRequest" }
  | { op: "markRegistered" }
  | { op: "prepareKeyUpload"; oneTimeKeyCount: number; hasUnusedFallbackKey: boolean }
  | { op: "markKeysPublished" }
  | { op: "processToDevice"; rows: ToDeviceRow[]; deviceKeys: DeviceKeysByUser }
  | { op: "acknowledged"; ids: string[] }
  | { op: "pendingBackupCount" }
  | { op: "close" }
  // 初回設定と解錠（#360）。秘密は Worker のメモリーにだけ置き、要求の本文だけを返す。
  | { op: "setupBegin"; reset: ResetExpectation | null }
  | ({ op: "setupAddPasskeySlot" } & PasskeySlotInput)
  | { op: "setupSecretStorageRequest" }
  | { op: "setupPendingSlots" }
  | { op: "unlock"; response: JsonObject; secret: UnlockSecret }
  | { op: "planResume"; keysQuery: JsonObject; versions: JsonObject }
  | { op: "finishSetup"; secretsRevision: number | null }
  | ({ op: "newPasskeySlot" } & PasskeySlotInput)
  | { op: "startupCheck"; keysQuery: JsonObject; versions: JsonObject }
  | { op: "hasSecrets" }
  | { op: "hasOwnKeys" }
  // 鍵バックアップへのアップロード（#360）。
  | { op: "prepareBackupUpload"; currentVersion: JsonObject | null; limit: number }
  | { op: "backupUploaded" }
  | { op: "backupUploadFailed" }
  | { op: "discardSecrets" }
  | { op: "forgetOwnKeys" }
  // Megolm の送信と復号（#361）。
  | { op: "megolmNeedsSnapshot"; channelId: string; epoch: number }
  | { op: "megolmEvaluate"; channelId: string; pages: JsonObject[] }
  | { op: "megolmSettlePins"; confirmed: NewTofuPin[]; discarded: NewTofuPin[] }
  | { op: "megolmStart"; channelId: string }
  | { op: "megolmShare"; channelId: string; claimed: ClaimedKeys }
  | { op: "megolmMarkShared"; channelId: string; devices: { userId: string; deviceId: string }[] }
  | { op: "megolmEncrypt"; channelId: string; message: OutgoingMessage }
  | { op: "megolmDiscard"; channelId: string }
  | { op: "megolmDecrypt"; rows: EncryptedMessage[] }
  | { op: "megolmRememberEncrypted"; channelId: string }
  | { op: "megolmIsKnownEncrypted"; channelId: string }
  | {
      /** 暗号ストアを消す。開いていなくても、`namespace` のファイルと保存鍵を消す。 */
      op: "destroy";
      namespace: string;
    };

export interface GeneratedDeviceKeys {
  /** 自己署名した端末鍵（`device_keys`）。 */
  deviceKeys: JsonObject;
  identityKeys: IdentityKeys;
  /** 作った署名付きのワンタイム鍵の数。 */
  oneTimeKeyCount: number;
}

export interface OpenedCryptoStore {
  userId: string;
  deviceId: string;
  /** サーバーへの登録が済んでいるか。 */
  registered: boolean;
  /** 保存鍵を開けずに作り直したとき、前の端末の ID（サーバーから消す）。 */
  replacedDeviceId: string | null;
}

export interface CryptoResults {
  generateDeviceKeys: GeneratedDeviceKeys;
  open: OpenedCryptoStore;
  registrationRequest: RegistrationRequest;
  markRegistered: null;
  prepareKeyUpload: KeyUpload | null;
  markKeysPublished: null;
  processToDevice: ToDeviceResult;
  acknowledged: null;
  pendingBackupCount: number;
  close: null;
  destroy: null;
  setupBegin: { recoveryKey: string; keyId: string };
  setupAddPasskeySlot: null;
  setupSecretStorageRequest: JsonObject;
  setupPendingSlots: KeySlot[];
  unlock: { keyId: string; secretsRevision: number | null };
  planResume: ResumePlan;
  finishSetup: null;
  newPasskeySlot: KeySlot;
  startupCheck: StartupCheck | null;
  hasSecrets: boolean;
  hasOwnKeys: boolean;
  prepareBackupUpload: BackupUploadRequest;
  backupUploaded: null;
  backupUploadFailed: null;
  discardSecrets: null;
  forgetOwnKeys: null;
  megolmNeedsSnapshot: boolean;
  megolmEvaluate: RecipientSummary;
  megolmSettlePins: null;
  megolmStart: SharePlan;
  megolmShare: ShareResult;
  megolmMarkShared: null;
  megolmEncrypt: { ciphertext: JsonObject; epoch: number };
  megolmDiscard: null;
  megolmDecrypt: DecryptOutcome[];
  megolmRememberEncrypted: null;
  megolmIsKnownEncrypted: boolean;
}

export type CryptoRequest = CryptoOperation & { id: number };

export interface CryptoFailure {
  name: string;
  message: string;
  /** `ZooCryptoError` の `code`（`zoo_crypto::CryptoError` の列挙子の名前など）。 */
  code?: string;
}

export type CryptoResponse =
  | { id: number; ok: true; result: CryptoResults[CryptoOperation["op"]] }
  | { id: number; ok: false; error: CryptoFailure };
