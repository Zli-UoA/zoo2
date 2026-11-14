import type {
  AcceptedMasterKey,
  JsonObject,
  RevocationScope,
  RevocationStage,
  RevocationSummary,
  TrustWarning,
  UserState,
} from "@zoo/crypto-wasm";

import { ApiError } from "../api/client";
import { deleteDevice } from "./api";
import { REVOCATION_CHANGED, REVOCATION_PENDING } from "./protocol";
import { PasskeyError, evaluatePrfForNewSlot, evaluatePrfForUnlock } from "./passkey";
import {
  type OwnDevice,
  currentBackupVersion,
  deleteBackupVersion,
  listAllPins,
  listBackupSessions,
  listDevices,
  pinConflict,
  putSecrets,
  queryKeys,
  uploadBackupSessions,
  uploadPins,
  uploadSignaturesWithReauth,
} from "./revocationApi";
import {
  type SecretStorageResponse,
  createBackupVersion,
  createSecretStorageKey,
  errorCode,
  getSecretStorage,
  isReauthRequired,
  listBackupVersions,
  markKeySlotUsed,
  queryOwnKeys,
  uploadCrossSigningKeys,
} from "./secretStorageApi";
import {
  type E2eeSetupDeps,
  ReauthRequiredError,
  ServerKeyMismatchError,
  type UnlockMethod,
} from "./setupController";

/**
 * 端末一覧、失効の手順、他のユーザーの検証（docs/e2ee.md の「失効と復旧」「クロス署名」。#362）。
 *
 * 秘密（マスター鍵の秘密鍵、SSSS 鍵、新しい鍵）は Worker の中だけに置き、ここでは Worker が作った
 * 要求の本文をサーバーに送る。画面（`DevicesSettings`）はここの関数を順に呼ぶ。
 */

/** 端末一覧の 1 行。 */
export interface DeviceView {
  deviceId: string;
  /** 端末名。攻撃者も付けられるので、端末を見分ける根拠にしない（登録日時と最後に使った日時を並べる）。 */
  displayName: string | null;
  createdAt: string;
  lastSeenAt: string | null;
  current: boolean;
  /** 今の自己署名鍵の署名があるか（なければ、解錠し直すまで鍵が届かない）。 */
  signed: boolean;
}

/** 署名の行（`keys/query` の `signatures`）。 */
interface SignatureRow {
  signer_user_id: string;
  signer_key_id: string;
  target_user_id: string;
  target_key_id: string;
}

function crossSigningPublicKey(keysQuery: JsonObject, field: string, userId: string) {
  const key = (keysQuery[field] as Record<string, { keys?: Record<string, string> }> | undefined)?.[
    userId
  ];
  const values = Object.values(key?.keys ?? {});
  return values.length === 1 ? values[0] : null;
}

/**
 * 自分の端末の一覧。今の自己署名鍵の署名があるかは、表示のために署名の行があるかだけを見る
 * （配布先の判定では、署名を検証する）。
 */
export async function loadDevices(deps: E2eeSetupDeps, currentDeviceId: string | null) {
  const [devices, keysQuery] = await Promise.all([
    listDevices(deps.client),
    queryOwnKeys(deps.client, deps.userId),
  ]);
  const selfSigning = crossSigningPublicKey(keysQuery, "self_signing_keys", deps.userId);
  const rows = (keysQuery.signatures as unknown as SignatureRow[] | undefined) ?? [];
  const signed = new Set(
    rows
      .filter(
        (row) =>
          row.signer_user_id === deps.userId &&
          row.target_user_id === deps.userId &&
          selfSigning !== null &&
          row.signer_key_id === `ed25519:${selfSigning}`,
      )
      .map((row) => row.target_key_id),
  );
  return devices.map((device: OwnDevice): DeviceView => ({
    deviceId: device.device_id,
    displayName: device.display_name,
    createdAt: device.created_at,
    lastSeenAt: device.last_seen_at,
    current: device.device_id === currentDeviceId,
    signed: signed.has(device.device_id),
  }));
}

/** 保存した失効の進み具合（終わっていない失効がなければ `null`）。 */
export function revocationStatus(deps: E2eeSetupDeps): Promise<RevocationSummary | null> {
  return deps.worker.request({ op: "revocationStatus" });
}

/** 始める前に SSSS を開け直す（パスキーかリカバリーキー）。続きの保存はしない。 */
export async function openSecretStorage(
  deps: E2eeSetupDeps,
  method: UnlockMethod,
): Promise<SecretStorageResponse> {
  const storage = await getSecretStorage(deps.client);
  if (method.kind === "passkey") {
    if (!deps.rpId) throw new PasskeyError("prfUnsupported", "パスキーを使えない環境です");
    const result = await evaluatePrfForUnlock(deps.rpId, storage.slots, deps.getCredential);
    await deps.worker.request({
      op: "unlock",
      response: storage as unknown as JsonObject,
      secret: { kind: "passkey", credentialId: result.credentialId, prfOutput: result.prfOutput },
    });
    void markKeySlotUsed(deps.client, result.slot.id);
  } else {
    await deps.worker.request({
      op: "unlock",
      response: storage as unknown as JsonObject,
      secret: { kind: "recovery", recoveryKey: method.recoveryKey },
    });
    const recovery = storage.slots.find((slot) => slot.kind === "recovery");
    if (recovery) void markKeySlotUsed(deps.client, recovery.id);
  }
  return storage;
}

/** 失効の手順の 1：選んだ端末を削除する（端末の削除は失効ではない。続けて 2 以降を行う）。 */
export async function deleteSelectedDevices(deps: E2eeSetupDeps, deviceIds: string[]) {
  for (const deviceId of deviceIds) await deleteDevice(deps.client, deviceId);
}

/**
 * 失効を始める。署名し直す一覧を集めて暗号ストアに保存する（サーバーにはまだ何も保存しない）。
 * SSSS を開けてあること（{@link openSecretStorage}）が前提。
 */
export async function beginRevocation(
  deps: E2eeSetupDeps,
  scope: RevocationScope,
  deletedDevices: string[],
): Promise<RevocationSummary> {
  const [secretStorage, keysQuery, versions, pins] = await Promise.all([
    getSecretStorage(deps.client),
    queryOwnKeys(deps.client, deps.userId),
    listBackupVersions(deps.client),
    listAllPins(deps.client),
  ]);
  // 署名し直す候補：ピンか記録がある相手（記録の相手は Worker から受け取らないので、ピンの相手と、
  // 自分を署名者とする署名がある相手を `keys/query` で取る。記録だけの相手のピンは記録から作る）。
  const targets = new Set<string>();
  for (const pin of pins) {
    if (typeof pin.target_user_id === "string") targets.add(pin.target_user_id);
  }
  for (const user of await recordedUsers(deps)) targets.add(user);
  targets.delete(deps.userId);
  const targetsQuery = targets.size > 0 ? await queryKeys(deps.client, [...targets]) : {};
  try {
    return await deps.worker.request({
      op: "revocationBegin",
      scope,
      secretStorage: secretStorage as unknown as JsonObject,
      keysQuery,
      versions,
      pins,
      targetsQuery,
      deletedDevices,
    });
  } catch (error) {
    if ((error as { code?: unknown } | null)?.code === "ServerKeyMismatch") {
      throw new ServerKeyMismatchError((error as Error).message);
    }
    if (codeOf(error) === REVOCATION_PENDING) throw new RevocationPendingError();
    throw error;
  }
}

function codeOf(error: unknown): unknown {
  return (error as { code?: unknown } | null)?.code;
}

/** 暗号化チャンネルを共有する相手として記録した相手（失効の候補を集めるため）。 */
async function recordedUsers(deps: E2eeSetupDeps): Promise<string[]> {
  return deps.worker.request({ op: "trustRecordedUsers" });
}

/** {@link resumeRevocation} の結果。 */
export type ResumeResult =
  /** 記録した段から続ける。 */
  | { kind: "resumed"; summary: RevocationSummary }
  /**
   * 2〜5 を SSSS の保存の前に止めたので、最初からやり直す。記録は捨てたが、範囲と削除した端末は
   * 引き継ぐ（範囲を黙って端末だけに縮めないため）。
   */
  | { kind: "restart"; scope: RevocationScope; deletedDevices: string[] }
  /**
   * 別の端末が SSSS を作り直したので続けられない。記録は残る。「やり直す」では、この失効
   * （`revocationId`）の記録だけを捨ててから、同じ範囲と端末で始め直す。
   */
  | {
      kind: "conflict";
      revocationId: string;
      scope: RevocationScope;
      deletedDevices: string[];
    };

/** 止まった失効を、開け直した SSSS で続ける。 */
export async function resumeRevocation(deps: E2eeSetupDeps): Promise<ResumeResult> {
  // やり直しになったら記録を捨てるので、その前に範囲と削除した端末を読んでおく。
  const saved = await deps.worker.request({ op: "revocationStatus" });
  if (!saved) throw new Error("続ける失効がありません");
  // 読んだ失効だけを続ける（読んでから続けるまでに、別の画面で別の失効に置き換わっていたら止める）。
  const revocationId = saved.revocation_id;
  const secretStorage = await getSecretStorage(deps.client);
  try {
    const summary = await deps.worker.request({
      op: "revocationResume",
      secretStorage: secretStorage as unknown as JsonObject,
      expectedId: revocationId,
    });
    return { kind: "resumed", summary };
  } catch (error) {
    const code = codeOf(error);
    if (code === "RevocationRestart") {
      await deps.worker.request({ op: "revocationAbandon", expectedId: revocationId });
      return { kind: "restart", scope: saved.scope, deletedDevices: saved.deleted_devices };
    }
    if (code === "ServerKeyMismatch") {
      return {
        kind: "conflict",
        revocationId,
        scope: saved.scope,
        deletedDevices: saved.deleted_devices,
      };
    }
    if (code === REVOCATION_CHANGED) throw new RevocationChangedError();
    throw error;
  }
}

/** 2〜5：新しい SSSS 鍵のリカバリーキーを作る。利用者に控えてもらう。 */
export function newRevocationRecoveryKey(deps: E2eeSetupDeps): Promise<string> {
  return deps.worker.request({ op: "revocationNewRecoveryKey" });
}

/** 2〜5：新しい SSSS 鍵に、この端末のパスキーのスロットを作る（新しい `prf_salt`）。 */
export async function addRevocationPasskey(deps: E2eeSetupDeps, label: string | null) {
  if (!deps.rpId) throw new PasskeyError("prfUnsupported", "パスキーを使えない環境です");
  const prfSalt = crypto.getRandomValues(new Uint8Array(32));
  const result = await evaluatePrfForNewSlot(deps.rpId, prfSalt, deps.getCredential);
  await deps.worker.request({ op: "revocationAddPasskeySlot", ...result, prfSalt, label });
}

/** 別の端末の失効と入り混じった（409）。SSSS を開け直して、失効をやり直す。 */
/**
 * 実行を始めた失効と、Worker が今持つ失効が違う（再認証のあいだに、画面を離れて別の失効を始めた）。
 * 何も保存していない。今の失効は捨てずに、画面を読み直す。
 */
export class RevocationChangedError extends Error {
  constructor() {
    super("実行を始めた失効が、別の失効に置き換わりました");
    this.name = "RevocationChangedError";
  }
}

/** 終わっていない失効があるので、新しい失効を始めない。「失効を続ける」から続けるか、捨ててから始める。 */
export class RevocationPendingError extends Error {
  constructor() {
    super("終わっていない失効があります");
    this.name = "RevocationPendingError";
  }
}

export class RevocationConflictError extends Error {
  constructor(readonly code: string) {
    super(`別の端末の失効と入り混じりました（${code}）`);
    this.name = "RevocationConflictError";
  }
}

/** 失効の結果。 */
export interface RevocationResult {
  summary: RevocationSummary;
  /** 新しい自己署名鍵で署名されていない自分の端末（解錠し直すまで鍵が届かない）。 */
  unsignedDevices: DeviceView[];
  /** 新しい版に移せなかった行の数。 */
  droppedSessions: number;
}

function stageAtLeast(summary: RevocationSummary, stage: RevocationStage): boolean {
  const order: RevocationStage[] = [
    "prepared",
    "secret_storage_saved",
    "cross_signing_saved",
    "signatures_saved",
    "backup_created",
    "migrated",
    "verified",
  ];
  return order.indexOf(summary.stage) >= order.indexOf(stage);
}

async function guarded<T>(task: () => Promise<T>): Promise<T> {
  try {
    return await task();
  } catch (error) {
    if (isReauthRequired(error)) throw new ReauthRequiredError();
    if (error instanceof ApiError && error.status === 409) {
      throw new RevocationConflictError(errorCode(error) ?? "CONFLICT");
    }
    throw error;
  }
}

/**
 * 記録した段の続きから、最後まで保存する（SSSS、クロス署名鍵、署名とピン、鍵バックアップの版、古い版の
 * 移行と削除、読み直しの確認）。`reauth` は 5 分以内の再認証の ID トークン。
 *
 * - 403 `E2EE_REAUTH_REQUIRED` なら {@link ReauthRequiredError}。再認証してから、同じ関数を呼び直す。
 * - 409（別の端末の失効と入り混じった）なら {@link RevocationConflictError}。SSSS を開け直して
 *   {@link beginRevocation} からやり直す。
 * - 読み直しで一致しなければ {@link ServerKeyMismatchError}。同じくやり直す。
 */
export async function continueRevocation(
  deps: E2eeSetupDeps,
  reauth: string | null,
  onProgress?: (summary: RevocationSummary) => void,
  /** 実行を始めたときの失効（`revocation_id`）。違えば何も保存せずに {@link RevocationChangedError}。 */
  expectedId?: string,
): Promise<RevocationResult> {
  try {
    return await runRevocation(deps, reauth, onProgress, expectedId);
  } catch (error) {
    // Worker は、保存と段の更新の要求ごとに失効 ID を照合する。途中で別の失効に置き換わったら止まる。
    if (codeOf(error) === REVOCATION_CHANGED) throw new RevocationChangedError();
    throw error;
  }
}

async function runRevocation(
  deps: E2eeSetupDeps,
  reauth: string | null,
  onProgress: ((summary: RevocationSummary) => void) | undefined,
  expectedId: string | undefined,
): Promise<RevocationResult> {
  const worker = deps.worker;
  let summary = await worker.request({ op: "revocationStatus" });
  if (!summary) throw new Error("続ける失効がありません");
  if (expectedId !== undefined && summary.revocation_id !== expectedId) {
    throw new RevocationChangedError();
  }
  // 以降の Worker への保存と段の更新には、この失効の ID を付ける（Worker が照合する）。
  const revocationId = summary.revocation_id;
  const progress = async () => {
    const next = await worker.request({ op: "revocationStatus" });
    if (next && next.revocation_id !== revocationId) throw new RevocationChangedError();
    summary = next ?? summary;
    if (summary) onProgress?.(summary);
  };
  let secretsRevision: number | null = null;

  if (!stageAtLeast(summary, "secret_storage_saved")) {
    const body = await worker.request({ op: "revocationSecretStorageRequest", revocationId });
    try {
      secretsRevision = await guarded(() =>
        summary?.scope === "full"
          ? createSecretStorageKey(deps.client, body, reauth)
          : putSecrets(deps.client, body, reauth),
      );
    } catch (error) {
      // 応答をなくして送り直したか、別の端末が先に書いた。読み直して、自分の書き込みが済んでいれば続ける。
      if (!(error instanceof RevocationConflictError) || !ssssConflictCode(error)) throw error;
      const current = await getSecretStorage(deps.client);
      if (summary.scope !== "full" || current.default_key_id !== summary.key_id) throw error;
      secretsRevision = current.secrets_revision;
    }
    await worker.request({ op: "revocationSecretStorageSaved", revocationId, secretsRevision });
    await progress();
  }
  if (secretsRevision === null) {
    secretsRevision = (await getSecretStorage(deps.client)).secrets_revision;
  }

  if (!stageAtLeast(summary, "cross_signing_saved")) {
    const body = await worker.request({ op: "revocationCrossSigningRequest", revocationId });
    await guarded(() => uploadCrossSigningKeys(deps.client, body, reauth));
    await worker.request({ op: "revocationCrossSigningSaved", revocationId, secretsRevision });
    await progress();
  }

  if (!stageAtLeast(summary, "signatures_saved")) {
    // 今の端末への自己署名は、相手のマスター鍵への署名と分けて先に送る（相手の都合で止めない）。
    const own = await worker.request({ op: "revocationSignatures", revocationId });
    await guarded(() => uploadSignaturesWithReauth(deps.client, [own.device], reauth));
    // 失効を始めてから相手がマスター鍵を作り直していると、その署名はサーバーが 404 で拒む。送る前に
    // 今のマスター鍵と照合して外し（注記に回す）、照合と送信のあいだに作り直されたら照合し直す。
    let signatures = own;
    for (let attempt = 0; ; attempt += 1) {
      const targets = summary.resigned_master_keys;
      if (targets.length > 0) {
        const current = await queryKeys(deps.client, targets);
        await worker.request({
          op: "revocationDropStaleMasterKeys",
          revocationId,
          masterKeys: (current.master_keys ?? {}) as JsonObject,
        });
        await progress();
        signatures = await worker.request({ op: "revocationSignatures", revocationId });
      }
      try {
        await guarded(() =>
          uploadSignaturesWithReauth(deps.client, signatures.master_keys, reauth),
        );
        break;
      } catch (error) {
        if (!(error instanceof ApiError && error.status === 404) || attempt >= 2) throw error;
      }
    }
    if (signatures.pins.length > 0) {
      await guarded(() => uploadPins(deps.client, signatures.pins, true, reauth));
    }
    await worker.request({ op: "revocationAdvance", revocationId, stage: "signatures_saved" });
    await progress();
  }

  if (!stageAtLeast(summary, "backup_created")) {
    const body = await worker.request({ op: "revocationBackupVersionRequest", revocationId });
    const version = await guarded(() => createBackupVersion(deps.client, body, reauth));
    if (!version) throw new Error("版の番号が返りませんでした");
    await worker.request({ op: "revocationBackupCreated", revocationId, version });
    await progress();
  }

  if (!stageAtLeast(summary, "migrated")) {
    const newVersion = summary.new_backup_version;
    if (!newVersion) throw new Error("新しい版がありません");
    const versions = await listBackupVersions(deps.client);
    const migrate = await worker.request({
      op: "revocationVersionsToMigrate",
      revocationId,
      versions,
    });
    for (const version of migrate) {
      const rows = await listBackupSessions(deps.client, version);
      // 別の端末が先に移行を終えて消した。
      if (rows === null) continue;
      // 捨てた行の数は、失効の記録に版ごとに残る（古い版を消したあとで止まっても数え直せないため）。
      const result = await worker.request({
        op: "revocationReencrypt",
        revocationId,
        version,
        rows,
      });
      await guarded(() =>
        uploadBackupSessions(deps.client, newVersion, secretsRevision as number, result.rows),
      );
      // 今の版ではないので、再認証は要らない。
      await deleteBackupVersion(deps.client, version, null);
    }
    await worker.request({ op: "revocationAdvance", revocationId, stage: "migrated" });
    await progress();
  }

  const [secretStorage, keysQuery, current] = await Promise.all([
    getSecretStorage(deps.client),
    queryOwnKeys(deps.client, deps.userId),
    currentBackupVersion(deps.client),
  ]);
  const finished = summary;
  // 一覧は、記録を消す（`revocationVerify`）前に読む。あとで読むと、読めなかったときに、終わった失効を
  // 失敗として扱い、記録がないので続けられなくなる。
  const devices = await loadDevices(deps, null);
  try {
    await worker.request({
      op: "revocationVerify",
      revocationId,
      secretStorage: secretStorage as unknown as JsonObject,
      keysQuery,
      currentVersion: current ?? {},
    });
  } catch (error) {
    // 別の失効に置き換わった（Worker の照合）。読み直しの不一致ではないので、今の失効を捨てさせない。
    if (codeOf(error) === REVOCATION_CHANGED) throw new RevocationChangedError();
    throw new ServerKeyMismatchError((error as Error).message);
  }
  return {
    summary: { ...finished, stage: "verified" },
    unsignedDevices: devices.filter((device) => !device.signed),
    droppedSessions: finished.dropped_sessions,
  };
}

function ssssConflictCode(error: RevocationConflictError): boolean {
  return error.code === "E2EE_SSSS_CONFLICT";
}

/** やり直すために、保存した失効を捨てる。 */
export async function abandonRevocation(
  deps: E2eeSetupDeps,
  /**
   * 捨てる失効（`revocation_id`）。暗号ストアの記録が別の失効なら捨てずに {@link RevocationChangedError}。
   * 省略したら、記録をそのまま捨てる。
   */
  expectedId?: string,
): Promise<void> {
  try {
    await deps.worker.request({ op: "revocationAbandon", expectedId });
  } catch (error) {
    if (codeOf(error) === REVOCATION_CHANGED) throw new RevocationChangedError();
    throw error;
  }
}

// ---------------------------------------------------------------------------
// 他のユーザーの検証

/** 他のユーザーの検証状態。 */
export interface UserVerification {
  userId: string;
  state: UserState;
}

/**
 * 暗号化チャンネルを共有する相手の検証状態を読む。初めて見た相手は TOFU でピンを置く
 * （配布先の判定の規則 4。送信と同じ）。
 */
export async function loadVerifications(
  deps: E2eeSetupDeps,
  userIds: string[],
): Promise<{ users: UserVerification[]; warnings: TrustWarning[] }> {
  const others = [...new Set(userIds)].filter((user) => user !== deps.userId);
  if (others.length === 0) return { users: [], warnings: [] };
  for (let attempt = 0; attempt < 3; attempt += 1) {
    const keysQuery = await queryKeys(deps.client, [...others, deps.userId]);
    const result = await deps.worker.request({ op: "trustEvaluate", keysQuery, userIds: others });
    if (result.new_pins.length === 0 || attempt === 2) {
      return {
        users: others.map((userId) => ({
          userId,
          state: result.users[userId]?.state ?? { state: "no_cross_signing_keys" },
        })),
        warnings: result.warnings,
      };
    }
    for (const pin of result.new_pins) {
      try {
        await uploadPins(deps.client, [pin.pin], false, null);
        await deps.worker.request({ op: "trustPinResult", pin, registered: true });
      } catch (error) {
        if (!pinConflict(error)) throw error;
        await deps.worker.request({ op: "trustPinResult", pin, registered: false });
      }
    }
  }
  return { users: [], warnings: [] };
}

/**
 * 利用者が相手の今のマスター鍵を受け入れる（再検証。TOFU を受け入れ直す）。`replace: true` のピンと
 * マスター鍵への署名には、直前の再認証が要る。記録は、ピンと署名を保存できてから登録待ちを外す
 * （保存できなければ、ピンも署名もないまま記録だけで配ることはない）。
 */
export async function reverifyUser(
  deps: E2eeSetupDeps,
  targetUserId: string,
  reauth: string | null,
): Promise<void> {
  const keysQuery = await queryKeys(deps.client, [targetUserId]);
  const masterKey = (keysQuery.master_keys as Record<string, JsonObject> | undefined)?.[
    targetUserId
  ];
  if (!masterKey) throw new Error("相手のマスター鍵がありません");
  const accepted = await deps.worker.request({
    op: "acceptMasterKey",
    targetUserId,
    masterKey,
  });
  await saveAcceptedMasterKey(deps, accepted, reauth);
}

/**
 * 受け入れ（#362）か SAS（#368）で作った `replace: true` のピンとマスター鍵への署名を保存し、
 * 保存できたら記録の登録待ちを外す。
 */
export async function saveAcceptedMasterKey(
  deps: Pick<E2eeSetupDeps, "worker" | "client">,
  accepted: AcceptedMasterKey,
  reauth: string | null,
): Promise<void> {
  await guarded(() => uploadPins(deps.client, [accepted.pin], true, reauth));
  await guarded(() => uploadSignaturesWithReauth(deps.client, [accepted.signature], reauth));
  await deps.worker.request({
    op: "confirmAcceptedMasterKey",
    targetUserId: accepted.target_user_id,
    masterKey: accepted.master_key,
  });
}
