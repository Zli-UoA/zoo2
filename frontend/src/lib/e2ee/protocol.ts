import type { IdentityKeys, JsonObject } from "@zoo/crypto-wasm";

/**
 * 暗号の Worker（`crypto.worker.ts`）とのやり取りの型。
 *
 * 鍵と Olm、Megolm の状態は Worker の中だけに置き、メインスレッドには結果の JSON だけを返す
 * （docs/e2ee.md の「共有の暗号コア」）。操作は #359 以降（暗号ストア、端末の登録）で足していく。
 */
export type CryptoOperation = {
  op: "generateDeviceKeys";
  /** 小文字でハイフン区切りの UUID。 */
  userId: string;
  /** UUID v4。 */
  deviceId: string;
};

export interface GeneratedDeviceKeys {
  /** 自己署名した端末鍵（`device_keys`）。 */
  deviceKeys: JsonObject;
  identityKeys: IdentityKeys;
  /** 作った署名付きのワンタイム鍵の数。 */
  oneTimeKeyCount: number;
}

export interface CryptoResults {
  generateDeviceKeys: GeneratedDeviceKeys;
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
