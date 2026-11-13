import { DeviceAccount } from "@zoo/crypto-wasm";

import { cryptoDatabaseName } from "./identity";
import { SetupSession } from "./setupSession";
import type { CryptoBackend } from "./store/backend";
import { CryptoStore } from "./store/cryptoStore";
import { indexedDbWrappingKeys, type WrappingKeyStore } from "./store/storageKey";
import type { CryptoFailure, CryptoOperation, CryptoResults } from "./protocol";

type Op = CryptoOperation["op"];

/** 暗号ストアの保存先を開く関数と、消す関数（ブラウザーでは OPFS の SQLite）。 */
export interface CryptoBackendFactory {
  open(databaseName: string): Promise<CryptoBackend>;
  remove(databaseName: string): Promise<void>;
}

/**
 * Worker の中で操作を実行する。wasm の初期化（`init` か `initSync`）を済ませてから呼ぶ。
 *
 * Worker の外（Node のテスト）からも同じ処理を確かめられるよう、メッセージの受け渡しと分けてある。
 * 暗号ストアは 1 つだけ開き、同時に 1 つの操作だけを行う（呼び出し側が直列にする）。
 */
export class CryptoOperations {
  private store: CryptoStore | null = null;
  private storeName: string | null = null;
  private readonly setup = new SetupSession(() => this.opened().userId);

  constructor(
    private readonly backends: CryptoBackendFactory,
    private readonly keys: WrappingKeyStore = indexedDbWrappingKeys,
  ) {}

  private lastOpen: { namespace: string; userId: string } | null = null;

  async run(request: CryptoOperation): Promise<CryptoResults[Op]> {
    try {
      return await this.dispatch(request);
    } catch (error) {
      // 保存に失敗した操作のあとは、メモリーの状態を捨てて、保存済みの状態から開き直す。
      if (this.store?.isBroken && this.lastOpen) {
        const reopen = this.lastOpen;
        await this.store.close().catch(() => undefined);
        this.store = null;
        this.storeName = null;
        await this.dispatch({ op: "open", ...reopen }).catch(() => undefined);
      }
      throw error;
    }
  }

  private async dispatch(request: CryptoOperation): Promise<CryptoResults[Op]> {
    switch (request.op) {
      case "generateDeviceKeys":
        return generateDeviceKeys(request.userId, request.deviceId);
      case "open": {
        const name = cryptoDatabaseName(request.namespace);
        if (this.store) {
          if (this.storeName === name && this.store.userId === request.userId) {
            return this.describe(null);
          }
          await this.store.close();
          this.store = null;
        }
        const { store, replacedDeviceId } = await CryptoStore.open({
          backend: await this.backends.open(name),
          name,
          userId: request.userId,
          keys: this.keys,
          reset: async () => {
            await this.backends.remove(name);
            return this.backends.open(name);
          },
        });
        this.store = store;
        this.storeName = name;
        this.lastOpen = { namespace: request.namespace, userId: request.userId };
        return this.describe(replacedDeviceId);
      }
      case "registrationRequest":
        return this.opened().registrationRequest();
      case "markRegistered":
        await this.opened().markRegistered();
        return null;
      case "prepareKeyUpload":
        return this.opened().prepareKeyUpload(request);
      case "markKeysPublished":
        await this.opened().markKeysPublished();
        return null;
      case "processToDevice":
        return this.opened().processToDevice(request.rows, request.deviceKeys);
      case "acknowledged":
        await this.opened().acknowledged(request.ids);
        return null;
      case "pendingBackupCount":
        return this.opened().pendingBackupCount();
      case "setupBegin":
        return this.setup.begin(request.reset);
      case "setupAddPasskeySlot":
        this.setup.addPasskeySlot(request);
        return null;
      case "setupSecretStorageRequest":
        return this.setup.secretStorageRequest();
      case "setupPendingSlots":
        return this.setup.pendingSlots();
      case "unlock":
        return this.setup.unlock(request.response, request.secret);
      case "planResume":
        return this.setup.plan(this.opened(), request.keysQuery, request.versions);
      case "finishSetup":
        await this.setup.finish(this.opened(), request.secretsRevision);
        return null;
      case "newPasskeySlot":
        return this.setup.newPasskeySlot(request);
      case "startupCheck":
        return this.setup.startupCheck(this.opened(), request.keysQuery, request.versions);
      case "hasSecrets":
        return this.setup.hasSecrets;
      case "hasOwnKeys":
        return this.opened().hasOwnKeys();
      case "discardSecrets":
        this.setup.discard();
        return null;
      case "forgetOwnKeys":
        this.setup.discard();
        await this.opened().forgetOwnKeys();
        return null;
      case "close":
        this.setup.discard();
        this.lastOpen = null;
        await this.store?.close();
        this.store = null;
        this.storeName = null;
        return null;
      case "destroy": {
        this.setup.discard();
        const name = cryptoDatabaseName(request.namespace);
        if (this.store && this.storeName === name) {
          await this.store.destroy();
        } else {
          await this.backends.remove(name);
          await this.keys.delete(name);
        }
        if (this.storeName === name) {
          this.store = null;
          this.storeName = null;
          this.lastOpen = null;
        }
        return null;
      }
    }
  }

  private opened(): CryptoStore {
    if (!this.store) throw new Error("暗号ストアが開いていません");
    return this.store;
  }

  private describe(replacedDeviceId: string | null) {
    const store = this.opened();
    return {
      userId: store.userId,
      deviceId: store.deviceId,
      registered: store.isRegistered,
      replacedDeviceId,
    };
  }
}

/** 状態を持たない操作（鍵の生成）。Node のテストから直接呼ぶ。 */
export function runCryptoOperation(
  request: Extract<CryptoOperation, { op: "generateDeviceKeys" }>,
): CryptoResults["generateDeviceKeys"] {
  return generateDeviceKeys(request.userId, request.deviceId);
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
