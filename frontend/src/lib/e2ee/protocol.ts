import type { IdentityKeys, JsonObject } from "@zoo/crypto-wasm";

import type {
  DeviceKeysByUser,
  KeyUpload,
  RegistrationRequest,
  ToDeviceResult,
  ToDeviceRow,
} from "./store/cryptoStore";

export type { DeviceKeysByUser, KeyUpload, RegistrationRequest, ToDeviceResult, ToDeviceRow };

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
