import { DeviceAccount, decryptAttachment, encryptAttachment } from "@zoo/crypto-wasm";

import { cryptoDatabaseName } from "./identity";
import { RevocationSession, evaluateTrust } from "./revocationSession";
import { SetupSession } from "./setupSession";
import type { CryptoBackend } from "./store/backend";
import { CryptoStore, destroyCryptoStore } from "./store/cryptoStore";
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
  private readonly revocation = new RevocationSession(this.setup, () => this.opened().userId);

  constructor(
    private readonly backends: CryptoBackendFactory,
    private readonly keys: WrappingKeyStore = indexedDbWrappingKeys,
  ) {}

  private lastOpen: { namespace: string; userId: string } | null = null;
  /** 開き直しが済んでいない（開き直しにも失敗した）。次の操作の前に、もう一度開き直す。 */
  private reopenPending = false;

  async run(request: CryptoOperation): Promise<CryptoResults[Op]> {
    const needsStore = ![
      "generateDeviceKeys",
      "open",
      "close",
      "destroy",
      "encryptAttachment",
      "decryptAttachment",
    ].includes(request.op);
    if (this.reopenPending && needsStore) {
      // ストレージが戻っていなければ、ここで失敗する。呼び出し側はあとで試し直す。
      await this.reopen();
    }
    try {
      return await this.dispatch(request);
    } catch (error) {
      // 保存に失敗した操作のあとは、メモリーの状態を捨てて、保存済みの状態から開き直す。
      if (this.store?.isBroken && this.lastOpen) {
        await this.store.close().catch(() => undefined);
        this.store = null;
        this.storeName = null;
        this.reopenPending = true;
        await this.reopen().catch(() => undefined);
      }
      throw error;
    }
  }

  private async reopen(): Promise<void> {
    const reopen = this.lastOpen;
    if (!reopen) {
      this.reopenPending = false;
      return;
    }
    await this.dispatch({ op: "open", ...reopen });
    this.reopenPending = false;
  }

  private async dispatch(request: CryptoOperation): Promise<CryptoResults[Op]> {
    // 失効の保存と段の更新は、実行を始めた失効にだけ当てる（#362）。
    if ("revocationId" in request) this.revocation.check(request.revocationId);
    switch (request.op) {
      case "generateDeviceKeys":
        return generateDeviceKeys(request.userId, request.deviceId);
      case "encryptAttachment": {
        const { ciphertext, file } = encryptAttachment(request.plaintext);
        return { ciphertext, file };
      }
      case "decryptAttachment":
        return decryptAttachment(request.file, request.ciphertext);
      case "open": {
        const name = cryptoDatabaseName(request.namespace);
        if (this.store) {
          if (this.storeName === name && this.store.userId === request.userId) {
            return this.describe(null, false);
          }
          await this.store.close();
          this.store = null;
        }
        const { store, replacedDeviceId, replacedRegistered } = await CryptoStore.open({
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
        this.reopenPending = false;
        return this.describe(replacedDeviceId, replacedRegistered);
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
        return this.opened().processToDevice(request.rows, request.keysQuery);
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
        return this.setup.secretStorageRequest(request.keyId);
      case "setupPendingSlots":
        return this.setup.pendingSlots();
      case "unlock":
        return this.setup.unlock(request.response, request.secret);
      case "planResume":
        return this.setup.plan(this.opened(), request.keysQuery, request.versions, request.keyId);
      case "finishSetup":
        await this.setup.finish(this.opened(), request.secretsRevision, request.keyId);
        return null;
      case "newPasskeySlot":
        return this.setup.newPasskeySlot(request);
      case "newRecoverySlot":
        return this.setup.newRecoverySlot();
      case "startupCheck":
        return this.setup.startupCheck(this.opened(), request.keysQuery, request.versions);
      case "hasSecrets":
        return this.setup.hasSecrets;
      case "hasOwnKeys":
        return this.opened().hasOwnKeys();
      case "prepareBackupUpload":
        return this.opened().prepareBackupUpload(request.currentVersion, request.limit);
      case "beginBackupImport":
        return this.opened().beginBackupImport(request.currentVersion);
      case "importBackupPage":
        return this.opened().importBackupPage(request.version, request.rows, request.next);
      case "backupUploaded":
        await this.opened().backupUploaded(request.uploadId);
        return null;
      case "backupUploadFailed":
        this.opened().discardPendingUpload(request.uploadId);
        return null;
      case "discardSecrets":
        this.revocation.discard();
        this.setup.discard();
        return null;
      case "forgetOwnKeys":
        this.setup.discard();
        await this.opened().forgetOwnKeys();
        return null;
      case "revocationStatus":
        return this.revocation.status(this.opened());
      case "revocationBegin": {
        const { op: _op, ...input } = request;
        return this.revocation.begin(this.opened(), input);
      }
      case "revocationResume":
        return this.revocation.resume(this.opened(), request.secretStorage, request.expectedId);
      case "revocationNewRecoveryKey":
        return this.revocation.newRecoveryKey();
      case "revocationAddPasskeySlot":
        this.revocation.addPasskeySlot(request);
        return null;
      case "revocationSecretStorageRequest":
        return this.revocation.secretStorageRequest();
      case "revocationSecretStorageSaved":
        await this.revocation.secretStorageSaved(this.opened(), request.secretsRevision);
        return null;
      case "revocationCrossSigningRequest":
        return this.revocation.crossSigningRequest();
      case "revocationCrossSigningSaved":
        await this.revocation.crossSigningSaved(this.opened(), request.secretsRevision);
        return null;
      case "revocationSignatures":
        return this.revocation.signatures(this.opened());
      case "revocationDropStaleMasterKeys":
        return this.revocation.dropStaleMasterKeys(this.opened(), request.masterKeys);
      case "revocationAdvance":
        await this.revocation.advance(this.opened(), request.stage);
        return null;
      case "revocationBackupVersionRequest":
        return this.revocation.backupVersionRequest();
      case "revocationBackupCreated":
        await this.revocation.backupCreated(this.opened(), request.version);
        return null;
      case "revocationVersionsToMigrate":
        return this.revocation.versionsToMigrate(request.versions);
      case "revocationReencrypt":
        return this.revocation.reencrypt(this.opened(), request.version, request.rows);
      case "revocationVerify":
        await this.revocation.verify(
          this.opened(),
          request.secretStorage,
          request.keysQuery,
          request.currentVersion,
        );
        return null;
      case "revocationAbandon":
        await this.revocation.abandon(this.opened(), request.expectedId);
        return null;
      case "trustRecordedUsers":
        return this.opened().trustStore.recordedUsers();
      case "trustEvaluate":
        return evaluateTrust(this.opened(), request.keysQuery, request.userIds);
      case "trustPinResult":
        if (request.registered) this.opened().trustStore.confirmTofuPin(request.pin);
        else this.opened().trustStore.discardTofuPin(request.pin);
        await this.opened().saveTrustStore();
        return null;
      case "acceptMasterKey":
        return this.opened().acceptMasterKey(request.targetUserId, request.masterKey);
      case "confirmAcceptedMasterKey":
        await this.opened().confirmAcceptedMasterKey(request.targetUserId, request.masterKey);
        return null;
      case "sasRequest":
        return this.opened().verification.request(
          request.targetUserId,
          request.keysQuery,
          request.now,
        );
      case "sasReceive":
        return this.opened().verification.receive(request.message, request.keysQuery, request.now);
      case "sasAccept":
        return this.opened().verification.accept(request.txnId);
      case "sasConfirm":
        return this.opened().verification.confirm(request.txnId);
      case "sasMismatch":
        return this.opened().verification.mismatch(request.txnId);
      case "sasCancel":
        return this.opened().verification.cancel(request.txnId);
      case "sasExpire":
        return this.opened().verification.expire(request.now);
      case "sasForget":
        this.opened().verification.forget(request.txnId);
        return null;
      case "sasSign":
        return this.opened().verification.sign(request.txnId, request.masterKey);
      case "sasEncrypt":
        return this.opened().verification.encrypt(request.outgoing, request.claimed);
      case "megolmNeedsSnapshot":
        return this.opened().megolm.needsSnapshot(request.channelId, request.epoch);
      case "megolmEvaluate":
        return this.opened().megolm.evaluate(request.channelId, request.pages);
      case "megolmSettlePins":
        await this.opened().megolm.settlePins(request.confirmed, request.discarded);
        return null;
      case "megolmStart":
        return this.opened().megolm.start(request.channelId);
      case "megolmShare":
        return this.opened().megolm.share(request.channelId, request.claimed);
      case "megolmMarkShared":
        this.opened().megolm.markShared(request.channelId, request.devices);
        return null;
      case "megolmEncrypt":
        return this.opened().megolm.encrypt(request.channelId, request.message);
      case "megolmDiscard":
        this.opened().megolm.discard(request.channelId);
        return null;
      case "megolmDecrypt":
        return this.opened().megolm.decrypt(request.rows);
      case "megolmRememberEncrypted":
        await this.opened().megolm.rememberEncrypted(request.channelId);
        return null;
      case "megolmIsKnownEncrypted":
        return this.opened().megolm.isKnownEncrypted(request.channelId);
      case "megolmNoteRecipients":
        return this.opened().megolm.noteRecipients(request.channelId, request.userIds);
      case "close":
        this.revocation.discard();
        this.setup.discard();
        this.lastOpen = null;
        this.reopenPending = false;
        await this.store?.close();
        this.store = null;
        this.storeName = null;
        return null;
      case "destroy": {
        this.revocation.discard();
        this.setup.discard();
        const name = cryptoDatabaseName(request.namespace);
        try {
          if (this.store && this.storeName === name) {
            await this.store.destroy();
          } else {
            await destroyCryptoStore({
              removeFiles: () => this.backends.remove(name),
              deleteWrappingKey: () => this.keys.delete(name),
            });
          }
        } finally {
          // 消し切れなくても（ファイルの削除だけが失敗したなど）、このストアはもう使わない。保存鍵を
          // 包む鍵は先に消しているので、開き直すと新しい端末として作り直してしまう。
          if (this.storeName === name) {
            this.store = null;
            this.storeName = null;
          }
          // 開き直しに失敗したあと（storeName が null）でも、消したストアを開き直さない。
          if (this.lastOpen?.namespace === request.namespace) {
            this.lastOpen = null;
            this.reopenPending = false;
          }
        }
        return null;
      }
    }
  }

  private opened(): CryptoStore {
    if (!this.store) throw new Error("暗号ストアが開いていません");
    return this.store;
  }

  private describe(replacedDeviceId: string | null, replacedRegistered: boolean) {
    const store = this.opened();
    return {
      userId: store.userId,
      deviceId: store.deviceId,
      registered: store.isRegistered,
      replacedDeviceId,
      replacedRegistered,
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
