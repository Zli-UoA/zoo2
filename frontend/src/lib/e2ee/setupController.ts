import type { JsonObject } from "@zoo/crypto-wasm";

import type { ApiClient } from "../api/client";
import type { CryptoWorker } from "./cryptoWorker";
import {
  type CredentialGetter,
  PasskeyError,
  evaluatePrfForNewSlot,
  evaluatePrfForUnlock,
} from "./passkey";
import type { ResumePlan } from "./protocol";
import {
  type SecretStorageResponse,
  addKeySlot,
  createBackupVersion,
  createSecretStorageKey,
  deleteKeySlot,
  getSecretStorage,
  hasSecretStorage,
  isReauthRequired,
  listBackupVersions,
  markKeySlotUsed,
  queryOwnKeys,
  ssssConflict,
  uploadCrossSigningKeys,
  uploadSignatures,
} from "./secretStorageApi";

/**
 * 初回設定と解錠の手順（docs/e2ee.md の「初回設定と解錠の流れ」。#360）。
 *
 * 秘密（マスター鍵の秘密鍵、SSSS 鍵）は Worker の中だけに置き、ここでは Worker が作った要求の
 * 本文をサーバーに送る。画面（`EncryptionSettings`）はここの関数を順に呼ぶ。
 */
export interface E2eeSetupDeps {
  worker: CryptoWorker;
  client: ApiClient;
  userId: string;
  /** パスキーの RP ID（issuer のホスト名）。OIDC を設定していなければ `null`（リカバリーキーだけ）。 */
  rpId: string | null;
  getCredential?: CredentialGetter;
}

export type E2eeStatus =
  /** SSSS がまだない。初回設定を始める。 */
  | { kind: "notSetUp" }
  /**
   * SSSS はあるが、この端末は解錠していない。`resume` は、解錠済みの端末が初回設定の途中で
   * 止まったのを見つけたとき（SSSS を開け直して続ける）。`mismatch` は、暗号ストアの鍵とサーバーの
   * 鍵が違うとき（失効のあとか、改ざん）。
   */
  | { kind: "locked"; storage: SecretStorageResponse; reason: "new" | "resume" | "mismatch" }
  /** 解錠済み。 */
  | { kind: "unlocked"; storage: SecretStorageResponse };

/** 保存が 409 `E2EE_SSSS_CONFLICT` になり、別の端末が先に初回設定を始めていた。 */
export class SetupConflictError extends Error {
  constructor() {
    super("別の端末が先に初回設定を始めました");
    this.name = "SetupConflictError";
  }
}

/** 直前の再認証が要る（403 `E2EE_REAUTH_REQUIRED`）。同じ鍵のまま、再認証してやり直す。 */
export class ReauthRequiredError extends Error {
  constructor() {
    super("再認証が必要です");
    this.name = "ReauthRequiredError";
  }
}

/** サーバーの鍵が SSSS の鍵と違う（失効が途中か、SSSS が書き換えられた）。 */
export class ServerKeyMismatchError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "ServerKeyMismatchError";
  }
}

function isServerKeyMismatch(error: unknown): boolean {
  return (error as { code?: unknown } | null)?.code === "ServerKeyMismatch";
}

async function withReauth<T>(task: () => Promise<T>): Promise<T> {
  try {
    return await task();
  } catch (error) {
    if (isReauthRequired(error)) throw new ReauthRequiredError();
    throw error;
  }
}

/** 今の状態を読む。解錠済みの端末は、起動時の確認（署名し直し、止まった初回設定の発見）も行う。 */
export async function loadE2eeStatus(deps: E2eeSetupDeps): Promise<E2eeStatus> {
  const storage = await getSecretStorage(deps.client);
  if (!hasSecretStorage(storage)) return { kind: "notSetUp" };
  const [keysQuery, versions] = await Promise.all([
    queryOwnKeys(deps.client, deps.userId),
    listBackupVersions(deps.client),
  ]);
  const check = await deps.worker.request({ op: "startupCheck", keysQuery, versions });
  if (!check) return { kind: "locked", storage, reason: "new" };
  switch (check.kind) {
    case "ready":
      return { kind: "unlocked", storage };
    case "sign_device":
      await uploadSignatures(deps.client, [check.signature]);
      return { kind: "unlocked", storage };
    case "resume_setup":
      return { kind: "locked", storage, reason: "resume" };
    case "unlock":
      return { kind: "locked", storage, reason: "mismatch" };
  }
}

/**
 * 起動時の確認（docs/e2ee.md の「初回設定と解錠の流れ」）。解錠済みの端末だけが、自分の端末への
 * 署名を確かめ、なければ署名し直す。まだ解錠していない端末は、何も取得しない。
 */
export async function runStartupCheck(deps: E2eeSetupDeps): Promise<void> {
  if (!(await deps.worker.request({ op: "hasOwnKeys" }))) return;
  const [keysQuery, versions] = await Promise.all([
    queryOwnKeys(deps.client, deps.userId),
    listBackupVersions(deps.client),
  ]);
  const check = await deps.worker.request({ op: "startupCheck", keysQuery, versions });
  if (check?.kind === "sign_device") await uploadSignatures(deps.client, [check.signature]);
}

/**
 * 初回設定（`storage` が `null`）か、E2EE のリセットを始める。リカバリーキーを返すので、利用者に
 * 控えてもらう。リセットは、`GET /api/e2ee/secret_storage` で読んだ鍵と `secrets_revision` を前提にする。
 */
export async function beginSetup(
  deps: E2eeSetupDeps,
  resetFrom: SecretStorageResponse | null,
): Promise<{ recoveryKey: string; keyId: string }> {
  const reset =
    resetFrom && hasSecretStorage(resetFrom)
      ? {
          expectedDefaultKeyId: resetFrom.default_key_id as string,
          expectedSecretsRevision: resetFrom.secrets_revision,
        }
      : null;
  return deps.worker.request({ op: "setupBegin", reset });
}

/** 初回設定に、パスキー（PRF）のスロットを足す。 */
export async function addSetupPasskey(deps: E2eeSetupDeps, label: string | null): Promise<void> {
  if (!deps.rpId) throw new PasskeyError("prfUnsupported", "パスキーを使えない環境です");
  const prfSalt = crypto.getRandomValues(new Uint8Array(32));
  const result = await evaluatePrfForNewSlot(deps.rpId, prfSalt, deps.getCredential);
  await deps.worker.request({ op: "setupAddPasskeySlot", ...result, prfSalt, label });
}

/**
 * 初回設定（またはリセット）を保存する。SSSS、クロス署名鍵、端末への署名、バックアップの版の順。
 *
 * - 403 `E2EE_REAUTH_REQUIRED` なら {@link ReauthRequiredError}。同じ鍵のまま再認証してやり直す。
 * - SSSS の保存が 409 `E2EE_SSSS_CONFLICT` なら、読み直して、自分の鍵が保存済みなら続ける。
 *   別の鍵なら {@link SetupConflictError}（手元の鍵を捨てる。表示したリカバリーキーは使えない）。
 */
export async function completeSetup(
  deps: E2eeSetupDeps,
  keyId: string,
  reauth: string | null,
): Promise<void> {
  const body = await deps.worker.request({ op: "setupSecretStorageRequest" });
  let revision: number | null;
  try {
    revision = await withReauth(() => createSecretStorageKey(deps.client, body, reauth));
  } catch (error) {
    if (!ssssConflict(error)) throw error;
    // 応答をなくして送り直したか、別の端末が先に保存した。読み直して判定する。
    const current = await getSecretStorage(deps.client);
    if (current.default_key_id !== keyId) {
      await deps.worker.request({ op: "discardSecrets" });
      throw new SetupConflictError();
    }
    revision = current.secrets_revision;
    // 前の試行で SSSS を保存したあとに足したパスキーのスロットは、まだサーバーにない。
    // 足すだけなので、再認証は要らない（同じ ID の再送は何もしない）。
    const saved = new Set(current.slots.map((slot) => slot.id));
    const pending = await deps.worker.request({ op: "setupPendingSlots" });
    for (const slot of pending) {
      if (!saved.has(slot.id)) await addKeySlot(deps.client, keyId, slot, reauth);
    }
  }
  await saveRemaining(deps, revision, reauth);
}

/**
 * SSSS を保存したあと（または解錠したあと）の続きを保存し、自分の鍵を暗号ストアに保存する。
 * 応答をなくして送り直しても、同じ鍵なら成功する（サーバーの冪等の規則）。
 */
export async function saveRemaining(
  deps: E2eeSetupDeps,
  secretsRevision: number | null,
  reauth: string | null,
): Promise<void> {
  const plan = await planRemaining(deps);
  await withReauth(async () => {
    if (plan.cross_signing) await uploadCrossSigningKeys(deps.client, plan.cross_signing, reauth);
    if (plan.device_signature) await uploadSignatures(deps.client, [plan.device_signature]);
    if (plan.backup_version) await createBackupVersion(deps.client, plan.backup_version, reauth);
  });
  await deps.worker.request({ op: "finishSetup", secretsRevision });
}

async function planRemaining(deps: E2eeSetupDeps): Promise<ResumePlan> {
  const [keysQuery, versions] = await Promise.all([
    queryOwnKeys(deps.client, deps.userId),
    listBackupVersions(deps.client),
  ]);
  try {
    return await deps.worker.request({ op: "planResume", keysQuery, versions });
  } catch (error) {
    if (isServerKeyMismatch(error)) {
      await deps.worker.request({ op: "discardSecrets" });
      throw new ServerKeyMismatchError((error as Error).message);
    }
    throw error;
  }
}

export type UnlockMethod = { kind: "recovery"; recoveryKey: string } | { kind: "passkey" };

/**
 * パスキーかリカバリーキーで SSSS を開ける。続きの保存に再認証が要らなければ、そのまま最後まで
 * 保存して `{ needsReauth: false }` を返す。要るとき（初回設定の再開でクロス署名鍵がない）は
 * `{ needsReauth: true }` を返すので、利用者の操作で再認証してから {@link saveRemaining} を呼ぶ。
 */
export async function unlock(
  deps: E2eeSetupDeps,
  storage: SecretStorageResponse,
  method: UnlockMethod,
): Promise<{ needsReauth: boolean; secretsRevision: number | null }> {
  let secretsRevision: number | null;
  if (method.kind === "passkey") {
    if (!deps.rpId) throw new PasskeyError("prfUnsupported", "パスキーを使えない環境です");
    const result = await evaluatePrfForUnlock(deps.rpId, storage.slots, deps.getCredential);
    ({ secretsRevision } = await deps.worker.request({
      op: "unlock",
      response: storage as unknown as JsonObject,
      secret: { kind: "passkey", credentialId: result.credentialId, prfOutput: result.prfOutput },
    }));
    void markKeySlotUsed(deps.client, result.slot.id);
  } else {
    ({ secretsRevision } = await deps.worker.request({
      op: "unlock",
      response: storage as unknown as JsonObject,
      secret: { kind: "recovery", recoveryKey: method.recoveryKey },
    }));
    const recovery = storage.slots.find((slot) => slot.kind === "recovery");
    if (recovery) void markKeySlotUsed(deps.client, recovery.id);
  }
  const plan = await planRemaining(deps);
  if (plan.cross_signing) return { needsReauth: true, secretsRevision };
  await saveRemaining(deps, secretsRevision, null);
  return { needsReauth: false, secretsRevision };
}

/**
 * 解錠済みの端末で、パスキーのスロットを足す（同じパスキーのスロットは置き換える）。
 * 置き換えには直前の再認証が要る。SSSS 鍵がメモリーになければ、先に解錠し直す。
 */
export async function addPasskeySlot(
  deps: E2eeSetupDeps,
  storage: SecretStorageResponse,
  label: string | null,
  reauth: string | null,
): Promise<void> {
  if (!deps.rpId) throw new PasskeyError("prfUnsupported", "パスキーを使えない環境です");
  const prfSalt = crypto.getRandomValues(new Uint8Array(32));
  const result = await evaluatePrfForNewSlot(deps.rpId, prfSalt, deps.getCredential);
  const slot = await deps.worker.request({ op: "newPasskeySlot", ...result, prfSalt, label });
  await withReauth(() => addKeySlot(deps.client, storage.default_key_id as string, slot, reauth));
}

/** パスキーのスロットを消す（AuthZ でパスキーを消しても、ここで消すまでは解錠できる）。 */
export async function removeKeySlot(
  deps: E2eeSetupDeps,
  slotId: string,
  reauth: string | null,
): Promise<void> {
  await withReauth(() => deleteKeySlot(deps.client, slotId, reauth));
}

/** SSSS 鍵がメモリーにあるか（スロットを足せるか）。 */
export function hasUnlockedSecrets(deps: E2eeSetupDeps): Promise<boolean> {
  return deps.worker.request({ op: "hasSecrets" });
}
