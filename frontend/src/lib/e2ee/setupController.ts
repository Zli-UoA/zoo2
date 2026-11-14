import type { JsonObject } from "@zoo/crypto-wasm";

import type { ApiClient } from "../api/client";
import type { CryptoWorker } from "./cryptoWorker";
import {
  type CredentialGetter,
  PasskeyError,
  evaluatePrfForNewSlot,
  evaluatePrfForUnlock,
} from "./passkey";
import type { KeySlot, ResumePlan } from "./protocol";
import {
  type SecretStorageResponse,
  addKeySlot,
  createBackupVersion,
  createSecretStorageKey,
  errorCode,
  getBackupKeysPage,
  getCurrentBackupVersion,
  uploadBackupKeys,
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
  /**
   * 解錠（初回設定、初回設定の続き、解錠し直しを含む）を終えたときに呼ぶ。鍵バックアップからの
   * 取り込み（`E2eeContextValue.requestKeyBackupImport`）を始めるのに使う。
   */
  onUnlocked?: () => void;
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

/** 1 回に送るセッションの数（サーバーの上限）。 */
export const BACKUP_UPLOAD_BATCH = 1000;

export type BackupUploadResult =
  | { kind: "done"; uploaded: number }
  /** 今の版を使えない（署名がない、SSSS の鍵と違う、巻き戻りなど）。送っていない。 */
  | { kind: "rejected"; code: string }
  /**
   * サーバーが拒んだ（409 `E2EE_WRONG_BACKUP_VERSION` か `E2EE_SSSS_CONFLICT`）。版が変わったか、
   * 失効で秘密が書き換わったので、解錠し直すまで送らない。
   */
  | { kind: "conflict"; code: string };

/**
 * 鍵バックアップの送信待ちのセッションを、今の版に送る（docs/e2ee.md の「鍵バックアップ」）。
 * 解錠していない端末と、今の版がない場合は何もしない。
 */
export function uploadKeyBackup(deps: E2eeSetupDeps): Promise<BackupUploadResult> {
  return enqueueBackup(deps, () => uploadKeyBackupNow(deps));
}

/**
 * 呼び出し元（解錠、起動時の確認、鍵の受信）にかかわらず、同じ Worker への鍵バックアップの送信と
 * 取り込みは 1 つずつ行う。
 */
function enqueueBackup<T>(deps: E2eeSetupDeps, task: () => Promise<T>): Promise<T> {
  const previous = backupQueues.get(deps.worker) ?? Promise.resolve();
  const run = previous.catch(() => undefined).then(task);
  backupQueues.set(deps.worker, run);
  return run;
}

const backupQueues = new WeakMap<CryptoWorker, Promise<unknown>>();

async function uploadKeyBackupNow(deps: E2eeSetupDeps): Promise<BackupUploadResult> {
  if (!(await deps.worker.request({ op: "hasOwnKeys" }))) return { kind: "done", uploaded: 0 };
  const currentVersion = await getCurrentBackupVersion(deps.client);
  let uploaded = 0;
  for (;;) {
    const request = await deps.worker.request({
      op: "prepareBackupUpload",
      currentVersion,
      limit: BACKUP_UPLOAD_BATCH,
    });
    if (request.kind === "none") return { kind: "done", uploaded };
    if (request.kind === "rejected") return request;
    try {
      await uploadBackupKeys(deps.client, request.version, request.secretsRevision, request.body);
    } catch (error) {
      await deps.worker.request({ op: "backupUploadFailed", uploadId: request.uploadId });
      const code = errorCode(error);
      if (code === "E2EE_WRONG_BACKUP_VERSION" || code === "E2EE_SSSS_CONFLICT") {
        return { kind: "conflict", code };
      }
      throw error;
    }
    await deps.worker.request({ op: "backupUploaded", uploadId: request.uploadId });
    uploaded += request.count;
  }
}

export type BackupImportResult =
  | { kind: "done"; imported: number; dropped: number }
  /** 今の版を使えない（{@link BackupUploadResult} の `rejected` と同じ）。取り込んでいない。 */
  | { kind: "rejected"; code: string }
  /** 取り込みの途中で版が消えた（404）。次の解錠か起動で、今の版から取り込み直す。 */
  | { kind: "gone"; imported: number; dropped: number };

/**
 * 鍵バックアップの今の版から、過去のセッションを取り込む（docs/e2ee.md の「新しい端末での解錠」の
 * 手順 5）。解錠していない端末と、今の版がない場合は何もしない。
 *
 * `GET /api/e2ee/key_backup/keys` を 1 ページ（1000 件）ずつ読み、ページごとに暗号ストアへ保存する。
 * 進み具合も同じ書き込みで保存するので、途中で失敗しても（通信の失敗、タブを閉じた）、次に呼んだ
 * ときに続きのページから取り込む。版を取り込み終えたら、同じ版では読み直さない。
 * アップロードと同じ順番待ちで行う（今の版の検証と送信待ちの記録を、同時に変えないため）。
 */
export function importKeyBackup(
  deps: E2eeSetupDeps,
  /** 1 ページを保存するたびに、取り込んだ数を渡す（復号できなかったメッセージを試し直すのに使う）。 */
  onPage?: (imported: number) => void,
): Promise<BackupImportResult> {
  return enqueueBackup(deps, () => importKeyBackupNow(deps, onPage));
}

async function importKeyBackupNow(
  deps: E2eeSetupDeps,
  onPage?: (imported: number) => void,
): Promise<BackupImportResult> {
  let imported = 0;
  let dropped = 0;
  if (!(await deps.worker.request({ op: "hasOwnKeys" }))) {
    return { kind: "done", imported, dropped };
  }
  const currentVersion = await getCurrentBackupVersion(deps.client);
  const start = await deps.worker.request({ op: "beginBackupImport", currentVersion });
  if (start.kind === "rejected") return start;
  if (start.kind !== "import") return { kind: "done", imported, dropped };
  let after = start.after;
  for (;;) {
    const page = await getBackupKeysPage(deps.client, start.version, after);
    if (!page) return { kind: "gone", imported, dropped };
    const result = await deps.worker.request({
      op: "importBackupPage",
      version: start.version,
      rows: page.sessions,
      next: page.next,
    });
    imported += result.imported;
    dropped += result.dropped;
    onPage?.(result.imported);
    if (!page.next) return { kind: "done", imported, dropped };
    after = page.next;
  }
}

/**
 * 起動時の確認（docs/e2ee.md の「初回設定と解錠の流れ」）。解錠済みの端末だけが、自分の端末への
 * 署名を確かめ、なければ署名し直す。まだ解錠していない端末は、何も取得しない。
 *
 * 暗号ストアの鍵をそのまま使える（解錠済みで、初回設定の途中でも鍵の不一致でもない）ときに `true`。
 * 鍵バックアップへの送信の失敗では `false` にしない（取り込みを始められるように）。
 */
export async function runStartupCheck(deps: E2eeSetupDeps): Promise<boolean> {
  if (!(await deps.worker.request({ op: "hasOwnKeys" }))) return false;
  const [keysQuery, versions] = await Promise.all([
    queryOwnKeys(deps.client, deps.userId),
    listBackupVersions(deps.client),
  ]);
  const check = await deps.worker.request({ op: "startupCheck", keysQuery, versions });
  if (check?.kind === "sign_device") await uploadSignatures(deps.client, [check.signature]);
  if (!check || check.kind === "resume_setup" || check.kind === "unlock") return false;
  // 送信に失敗しても（通信の失敗など）、鍵はそのまま使える。送信待ちは次の受信や起動で送る。
  await uploadKeyBackup(deps).catch(() => undefined);
  return true;
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
  // 再認証を待つ間に別の初回設定を始めていたら、Worker はこの本文を返さない（keyId が違う）。
  const body = await deps.worker.request({ op: "setupSecretStorageRequest", keyId });
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
  await saveRemaining(deps, revision, reauth, keyId);
}

/**
 * SSSS を保存したあと（または解錠したあと）の続きを保存し、自分の鍵を暗号ストアに保存する。
 * 応答をなくして送り直しても、同じ鍵なら成功する（サーバーの冪等の規則）。
 */
export async function saveRemaining(
  deps: E2eeSetupDeps,
  secretsRevision: number | null,
  reauth: string | null,
  /** 開けた（作った）SSSS 鍵の ID。サーバーの既定の鍵がこれでなければ、何も保存しない。 */
  keyId?: string,
): Promise<void> {
  const plan = await planRemaining(deps, keyId);
  // 初回設定とリセットでは、作った SSSS 鍵を前提にする。サーバーが同じトランザクションで既定の
  // 鍵を照合するので、読み直してから送るまでの間に別の端末が SSSS を作り直しても、鍵を差し替えない。
  const precondition: JsonObject = keyId === undefined ? {} : { expected_default_key_id: keyId };
  try {
    await withReauth(async () => {
      if (plan.cross_signing) {
        await uploadCrossSigningKeys(
          deps.client,
          { ...plan.cross_signing, ...precondition },
          reauth,
        );
      }
      if (plan.device_signature) await uploadSignatures(deps.client, [plan.device_signature]);
      if (plan.backup_version) {
        await createBackupVersion(deps.client, { ...plan.backup_version, ...precondition }, reauth);
      }
    });
  } catch (error) {
    if (keyId !== undefined && ssssConflict(error)) {
      await deps.worker.request({ op: "discardSecrets" });
      throw new SetupConflictError();
    }
    throw error;
  }
  await deps.worker.request({ op: "finishSetup", secretsRevision, keyId });
  // 解錠した（または初回設定を終えた）ので、手元のセッションを今の版に送る。失敗しても解錠は済んでいる。
  await uploadKeyBackup(deps).catch(() => undefined);
  // 過去のセッションを鍵バックアップから取り込む。件数が多いと時間がかかるので、待たずに返す
  // （取り込みと試し直しは E2eeProvider が行う）。
  deps.onUnlocked?.();
}

async function planRemaining(deps: E2eeSetupDeps, keyId?: string): Promise<ResumePlan> {
  const [keysQuery, versions, storage] = await Promise.all([
    queryOwnKeys(deps.client, deps.userId),
    listBackupVersions(deps.client),
    keyId === undefined ? null : getSecretStorage(deps.client),
  ]);
  // 別の端末が SSSS を作り直した（同時のリセットなど）。手元の鍵でクロス署名鍵や版を差し替えると、
  // SSSS とサーバーの鍵の世代がずれるので、何も送らない。サーバーの鍵の差し替えは SSSS の世代を
  // 前提にしないので、ここで読み直して確かめる（読んでから送るまでの間の競合は残る）。
  if (storage && storage.default_key_id !== keyId) {
    await deps.worker.request({ op: "discardSecrets" });
    throw new SetupConflictError();
  }
  try {
    return await deps.worker.request({ op: "planResume", keysQuery, versions, keyId });
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
 * `{ needsReauth: true }` を返すので、利用者の操作で再認証してから、返した `keyId` を付けて
 * {@link saveRemaining} を呼ぶ。続きの保存は、開けた SSSS 鍵がサーバーの既定の鍵のままであることを
 * 前提にする（別の端末が SSSS を作り直していたら、サーバーが 409 で拒む）。
 */
export async function unlock(
  deps: E2eeSetupDeps,
  storage: SecretStorageResponse,
  method: UnlockMethod,
): Promise<{ needsReauth: boolean; secretsRevision: number | null; keyId: string }> {
  let secretsRevision: number | null;
  let keyId: string;
  if (method.kind === "passkey") {
    if (!deps.rpId) throw new PasskeyError("prfUnsupported", "パスキーを使えない環境です");
    const result = await evaluatePrfForUnlock(deps.rpId, storage.slots, deps.getCredential);
    ({ secretsRevision, keyId } = await deps.worker.request({
      op: "unlock",
      response: storage as unknown as JsonObject,
      secret: { kind: "passkey", credentialId: result.credentialId, prfOutput: result.prfOutput },
    }));
    void markKeySlotUsed(deps.client, result.slot.id);
  } else {
    ({ secretsRevision, keyId } = await deps.worker.request({
      op: "unlock",
      response: storage as unknown as JsonObject,
      secret: { kind: "recovery", recoveryKey: method.recoveryKey },
    }));
    const recovery = storage.slots.find((slot) => slot.kind === "recovery");
    if (recovery) void markKeySlotUsed(deps.client, recovery.id);
  }
  const plan = await planRemaining(deps, keyId);
  if (plan.cross_signing) return { needsReauth: true, secretsRevision, keyId };
  await saveRemaining(deps, secretsRevision, null, keyId);
  return { needsReauth: false, secretsRevision, keyId };
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

/** リカバリーキーを作り直す（#362）。保存するまでサーバーは変えない。 */
export function newRecoveryKey(
  deps: E2eeSetupDeps,
): Promise<{ slot: KeySlot; recoveryKey: string }> {
  return deps.worker.request({ op: "newRecoverySlot" });
}

/** 作り直したリカバリーキーのスロットを保存する。今のスロットの置き換えには直前の再認証が要る。 */
export async function saveRecoverySlot(
  deps: E2eeSetupDeps,
  storage: SecretStorageResponse,
  slot: KeySlot,
  reauth: string | null,
): Promise<void> {
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
