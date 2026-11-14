import type {
  EncryptedFile,
  AcceptedMasterKey,
  EncryptedMessage,
  IdentityKeys,
  JsonObject,
  KeySlot,
  NewTofuPin,
  OutgoingMessage,
  ResumePlan,
  RevocationSignatures,
  RevocationStage,
  RevocationSummary,
  StartupCheck,
  TrustRecord,
  VerificationMessage,
  VerificationOutgoing,
} from "@zoo/crypto-wasm";

import type { RevocationBeginInput, TrustEvaluation } from "./revocationSession";
import type {
  ClaimedKeys,
  DecryptOutcome,
  RecipientSummary,
  SharePlan,
  ShareResult,
} from "./store/megolm";

import type { PasskeySlotInput, ResetExpectation, UnlockSecret } from "./setupSession";
import type { BackupImportStart, BackupUploadRequest } from "./store/cryptoStore";
import type { SasEncrypted, SasUpdate } from "./store/verification";

import type {
  DeviceKeysByUser,
  KeyUpload,
  KeysQueryResponse,
  RegistrationRequest,
  ToDeviceResult,
  ToDeviceRow,
} from "./store/cryptoStore";

export type {
  DeviceKeysByUser,
  KeyUpload,
  KeysQueryResponse,
  RegistrationRequest,
  ToDeviceResult,
  ToDeviceRow,
};
export type { ClaimedKeys, DecryptOutcome, RecipientSummary, SharePlan, ShareResult };
export type { KeySlot, PasskeySlotInput, ResetExpectation, ResumePlan, StartupCheck, UnlockSecret };
export type { RevocationBeginInput, TrustEvaluation };
export type { SasEncrypted, SasUpdate };

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
  | { op: "processToDevice"; rows: ToDeviceRow[]; keysQuery: KeysQueryResponse }
  | { op: "acknowledged"; ids: string[] }
  | { op: "pendingBackupCount" }
  | { op: "close" }
  // 初回設定と解錠（#360）。秘密は Worker のメモリーにだけ置き、要求の本文だけを返す。
  | { op: "setupBegin"; reset: ResetExpectation | null }
  | ({ op: "setupAddPasskeySlot" } & PasskeySlotInput)
  | { op: "setupSecretStorageRequest"; keyId: string }
  | { op: "setupPendingSlots" }
  | { op: "unlock"; response: JsonObject; secret: UnlockSecret }
  /** `keyId` を渡すと、メモリーの SSSS 鍵がその鍵のときだけ行う（初回設定とリセット）。 */
  | { op: "planResume"; keysQuery: JsonObject; versions: JsonObject; keyId?: string }
  | { op: "finishSetup"; secretsRevision: number | null; keyId?: string }
  | ({ op: "newPasskeySlot" } & PasskeySlotInput)
  | { op: "newRecoverySlot" }
  | { op: "startupCheck"; keysQuery: JsonObject; versions: JsonObject }
  | { op: "hasSecrets" }
  | { op: "hasOwnKeys" }
  // 鍵バックアップへのアップロード（#360）。
  | { op: "prepareBackupUpload"; currentVersion: JsonObject | null; limit: number }
  | { op: "backupUploaded"; uploadId: number }
  | { op: "backupUploadFailed"; uploadId: number }
  | { op: "beginBackupImport"; currentVersion: JsonObject | null }
  | { op: "importBackupPage"; version: string; rows: JsonObject[]; next: string | null }
  | { op: "discardSecrets" }
  | { op: "forgetOwnKeys" }
  // 失効の手順と、他のユーザーの検証（#362）。
  | { op: "revocationStatus" }
  | ({ op: "revocationBegin" } & RevocationBeginInput)
  | { op: "revocationResume"; secretStorage: JsonObject; expectedId?: string }
  | { op: "revocationNewRecoveryKey" }
  | ({ op: "revocationAddPasskeySlot" } & PasskeySlotInput)
  | { op: "revocationSecretStorageRequest"; revocationId: string }
  | { op: "revocationSecretStorageSaved"; revocationId: string; secretsRevision: number | null }
  | { op: "revocationCrossSigningRequest"; revocationId: string }
  | { op: "revocationCrossSigningSaved"; revocationId: string; secretsRevision: number | null }
  | { op: "revocationSignatures"; revocationId: string }
  | { op: "revocationDropStaleMasterKeys"; revocationId: string; masterKeys: JsonObject }
  | { op: "revocationAdvance"; revocationId: string; stage: RevocationStage }
  | { op: "revocationBackupVersionRequest"; revocationId: string }
  | { op: "revocationBackupCreated"; revocationId: string; version: string }
  | { op: "revocationVersionsToMigrate"; revocationId: string; versions: JsonObject }
  | { op: "revocationReencrypt"; revocationId: string; version: string; rows: JsonObject[] }
  | {
      op: "revocationVerify";
      revocationId: string;
      secretStorage: JsonObject;
      keysQuery: JsonObject;
      currentVersion: JsonObject;
    }
  | { op: "revocationAbandon"; expectedId?: string }
  | { op: "trustRecordedUsers" }
  | { op: "trustEvaluate"; keysQuery: JsonObject; userIds: string[] }
  | { op: "trustPinResult"; pin: NewTofuPin; registered: boolean }
  | {
      op: "acceptMasterKey";
      targetUserId: string;
      masterKey: JsonObject;
      /** 確認の画面で見せた公開鍵。`masterKey` がこの鍵でなければ署名しない。 */
      expectedMasterKey: string;
    }
  /** 相手の記録（前に信頼したマスター鍵）。再検証の確認の画面に出す。 */
  | { op: "trustRecord"; targetUserId: string }
  /** 解錠して信頼した自分のマスター鍵の公開鍵（相手が再検証で指紋を比べるために出す）。 */
  | { op: "ownMasterKey" }
  | { op: "confirmAcceptedMasterKey"; targetUserId: string; masterKey: string }
  // 他のユーザーの絵文字 SAS 検証（#368）。検証は Worker のメモリーにだけ置く。
  | { op: "sasRequest"; targetUserId: string; keysQuery: JsonObject; now: number }
  | { op: "sasReceive"; message: VerificationMessage; keysQuery: JsonObject; now: number }
  | { op: "sasAccept"; txnId: string }
  | { op: "sasConfirm"; txnId: string }
  | { op: "sasMismatch"; txnId: string }
  | { op: "sasCancel"; txnId: string }
  | { op: "sasExpire"; now: number }
  | { op: "sasForget"; txnId: string }
  | { op: "sasSign"; txnId: string; masterKey: JsonObject }
  | { op: "sasEncrypt"; outgoing: VerificationOutgoing[]; claimed: ClaimedKeys | null }
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
  | { op: "megolmNoteRecipients"; channelId: string; userIds: string[] }
  | {
      /** 暗号ストアを消す。開いていなくても、`namespace` のファイルと保存鍵を消す。 */
      op: "destroy";
      namespace: string;
    }
  | {
      /**
       * 端末が別の端末から削除されていた（確かめたあと）。暗号ストアを消さずに、端末アカウントだけを
       * 作り直して開き直す（`CryptoStore.replaceDevice`）。開いている `namespace` の暗号ストアに当てる。
       */
      op: "replaceDevice";
      namespace: string;
      userId: string;
    }
  // 暗号化した添付（#364）。暗号ストアを使わない。
  | { op: "encryptAttachment"; plaintext: Uint8Array }
  | { op: "decryptAttachment"; file: EncryptedFile; ciphertext: Uint8Array };

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
  /** 前の端末の登録が済んでいたか（`replacedDeviceId` がないときは `false`）。 */
  replacedRegistered: boolean;
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
  replaceDevice: OpenedCryptoStore;
  setupBegin: { recoveryKey: string; keyId: string };
  setupAddPasskeySlot: null;
  setupSecretStorageRequest: JsonObject;
  setupPendingSlots: KeySlot[];
  unlock: { keyId: string; secretsRevision: number | null };
  planResume: ResumePlan;
  finishSetup: null;
  newPasskeySlot: KeySlot;
  newRecoverySlot: { slot: KeySlot; recoveryKey: string };
  startupCheck: StartupCheck | null;
  hasSecrets: boolean;
  hasOwnKeys: boolean;
  prepareBackupUpload: BackupUploadRequest;
  backupUploaded: null;
  backupUploadFailed: null;
  beginBackupImport: BackupImportStart;
  importBackupPage: { imported: number; dropped: number };
  discardSecrets: null;
  forgetOwnKeys: null;
  revocationStatus: RevocationSummary | null;
  revocationBegin: RevocationSummary;
  revocationResume: RevocationSummary;
  revocationNewRecoveryKey: string;
  revocationAddPasskeySlot: null;
  revocationSecretStorageRequest: JsonObject;
  revocationSecretStorageSaved: null;
  revocationCrossSigningRequest: JsonObject;
  revocationCrossSigningSaved: null;
  revocationSignatures: RevocationSignatures;
  revocationAdvance: null;
  revocationBackupVersionRequest: JsonObject;
  revocationBackupCreated: null;
  revocationVersionsToMigrate: string[];
  revocationDropStaleMasterKeys: string[];
  revocationReencrypt: { rows: JsonObject[]; dropped: number };
  revocationVerify: null;
  revocationAbandon: null;
  trustRecordedUsers: string[];
  trustEvaluate: TrustEvaluation;
  trustPinResult: null;
  acceptMasterKey: AcceptedMasterKey;
  trustRecord: TrustRecord | null;
  ownMasterKey: string | null;
  confirmAcceptedMasterKey: null;
  sasRequest: SasUpdate;
  sasReceive: SasUpdate | null;
  sasAccept: SasUpdate;
  sasConfirm: SasUpdate;
  sasMismatch: SasUpdate;
  sasCancel: SasUpdate | null;
  sasExpire: SasUpdate[];
  sasForget: null;
  sasSign: AcceptedMasterKey;
  sasEncrypt: SasEncrypted;
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
  encryptAttachment: { ciphertext: Uint8Array; file: EncryptedFile };
  decryptAttachment: Uint8Array;
  megolmNoteRecipients: string[];
}

export type CryptoRequest = CryptoOperation & { id: number };

/** 失効の要求の `revocationId` が、Worker が今持つ失効と違う（`CryptoFailure.code`）。何も保存していない。 */
export const REVOCATION_CHANGED = "RevocationChanged";
/** 終わっていない失効の記録があるのに、新しい失効を始めようとした（`CryptoFailure.code`）。 */
export const REVOCATION_PENDING = "RevocationPending";

export interface CryptoFailure {
  name: string;
  message: string;
  /** `ZooCryptoError` の `code`（`zoo_crypto::CryptoError` の列挙子の名前など）。 */
  code?: string;
}

export type CryptoResponse =
  | { id: number; ok: true; result: CryptoResults[CryptoOperation["op"]] }
  | { id: number; ok: false; error: CryptoFailure };
