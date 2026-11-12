import { DeviceAccount } from "@zoo/crypto-wasm";

import type { CryptoFailure, CryptoOperation, CryptoResults } from "./protocol";

/**
 * Worker の中で 1 つの操作を実行する。wasm の初期化（`init` か `initSync`）を済ませてから呼ぶ。
 *
 * Worker の外（Node のテスト）からも同じ処理を確かめられるよう、メッセージの受け渡しと分けてある。
 */
export function runCryptoOperation(
  request: CryptoOperation,
): CryptoResults[CryptoOperation["op"]] {
  switch (request.op) {
    case "generateDeviceKeys":
      return generateDeviceKeys(request.userId, request.deviceId);
  }
}

function generateDeviceKeys(userId: string, deviceId: string) {
  const account = new DeviceAccount(userId, deviceId);
  try {
    account.generateOneTimeKeys(account.maxNumberOfOneTimeKeys);
    return {
      deviceKeys: account.deviceKeys(),
      identityKeys: account.identityKeys(),
      oneTimeKeyCount: Object.keys(account.oneTimeKeys()).length,
    };
  } finally {
    // 秘密鍵を持つ wasm 側のメモリーをすぐに返す。
    account.free();
  }
}

/** 投げられた値を、postMessage で送れる形にする。 */
export function toCryptoFailure(error: unknown): CryptoFailure {
  if (error instanceof Error) {
    const code = (error as Error & { code?: unknown }).code;
    return {
      name: error.name,
      message: error.message,
      ...(typeof code === "string" ? { code } : {}),
    };
  }
  return { name: "Error", message: String(error) };
}
