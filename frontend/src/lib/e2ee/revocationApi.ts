import type { JsonObject, SignatureUpload } from "@zoo/crypto-wasm";

import { ApiError, type ApiClient } from "../api/client";
import { REAUTH_HEADER, errorCode } from "./secretStorageApi";

/**
 * 端末一覧、失効、他のユーザーの検証で使う API（docs/e2ee.md の「端末とワンタイム鍵」「クロス署名」
 * 「鍵バックアップ」。#362）。直前の再認証が要る要求には `reauth` に ID トークンを渡す。
 */

/** `GET /api/e2ee/devices` の 1 件（自分の端末だけ。端末名と最後に使った日時は本人にだけ返る）。 */
export interface OwnDevice {
  device_id: string;
  display_name: string | null;
  device_keys: JsonObject;
  created_at: string;
  last_seen_at: string | null;
}

/** 20 台に達したときの 400 `E2EE_LIMIT_EXCEEDED`（`limit` は `devices`）の `stale_devices`。 */
export interface StaleDevice {
  device_id: string;
  last_seen_at: string | null;
}

/**
 * 署名（`MAX_SIGNATURES_PER_UPLOAD`）とピン（`MAX_PINS_PER_REQUEST`）の 1 回の要求の上限。超える分は
 * 分けて送る（どちらも同じものを送り直しても成功する）。
 */
export const UPLOAD_BATCH = 1000;

function headers(reauth: string | null | undefined): Record<string, string> | undefined {
  return reauth ? { [REAUTH_HEADER]: reauth } : undefined;
}

export async function listDevices(client: ApiClient): Promise<OwnDevice[]> {
  return client.get<OwnDevice[]>("/api/e2ee/devices");
}

export async function renameDevice(
  client: ApiClient,
  deviceId: string,
  displayName: string,
): Promise<void> {
  await client.patch(`/api/e2ee/devices/${encodeURIComponent(deviceId)}`, {
    display_name: displayName,
  });
}

/** 端末の登録が 20 台の上限で拒否されたときの `stale_devices`。ほかの失敗なら `null`。 */
export function staleDevices(error: unknown): StaleDevice[] | null {
  if (!(error instanceof ApiError) || error.status !== 400) return null;
  if (errorCode(error) !== "E2EE_LIMIT_EXCEEDED") return null;
  const body = error.body as { limit?: unknown; stale_devices?: unknown };
  if (body.limit !== "devices") return null;
  return Array.isArray(body.stale_devices) ? (body.stale_devices as StaleDevice[]) : [];
}

/** `GET /api/e2ee/tofu_pins` を `next` が `null` になるまで読む。 */
export async function listAllPins(client: ApiClient): Promise<JsonObject[]> {
  const pins: JsonObject[] = [];
  let after: string | null = null;
  for (;;) {
    const query: string = after ? `?after=${encodeURIComponent(after)}` : "";
    const page: { pins: JsonObject[]; next: string | null } = await client.get(
      `/api/e2ee/tofu_pins${query}`,
    );
    pins.push(...page.pins);
    if (!page.next) return pins;
    after = page.next;
  }
}

/** `POST /api/e2ee/tofu_pins`。`replace: true` には直前の再認証が要る。 */
export async function uploadPins(
  client: ApiClient,
  pins: JsonObject[],
  replace: boolean,
  reauth: string | null,
): Promise<void> {
  // サーバーは `type` と `user_id` を自分で付けるので、署名付きのピンから外して送る。
  const body = pins.map((pin) => ({
    target_user_id: pin.target_user_id,
    master_key: pin.master_key,
    signatures: pin.signatures,
  }));
  for (let start = 0; start < body.length; start += UPLOAD_BATCH) {
    await client.post(
      "/api/e2ee/tofu_pins",
      { pins: body.slice(start, start + UPLOAD_BATCH), replace },
      { headers: headers(reauth) },
    );
  }
}

/** 409 `E2EE_PIN_CONFLICT` で返った、他の端末のピン。 */
export function pinConflict(error: unknown): JsonObject | null {
  if (!(error instanceof ApiError) || error.status !== 409) return null;
  if (errorCode(error) !== "E2EE_PIN_CONFLICT") return null;
  const pin = (error.body as { pin?: unknown }).pin;
  return pin && typeof pin === "object" ? (pin as JsonObject) : null;
}

/** 他のユーザーのマスター鍵への署名を含む署名の書き込み（直前の再認証が要る）。 */
export async function uploadSignaturesWithReauth(
  client: ApiClient,
  signatures: SignatureUpload[],
  reauth: string | null,
): Promise<void> {
  for (let start = 0; start < signatures.length; start += UPLOAD_BATCH) {
    await client.post(
      "/api/e2ee/cross_signing/signatures",
      { signatures: signatures.slice(start, start + UPLOAD_BATCH) },
      { headers: headers(reauth) },
    );
  }
}

export { queryKeys } from "./api";

/** `POST /api/e2ee/secret_storage/secrets`（端末だけの失効）。増やしたあとの `secrets_revision`。 */
export async function putSecrets(
  client: ApiClient,
  body: JsonObject,
  reauth: string | null,
): Promise<number | null> {
  const { data } = await client.post<{ secrets_revision?: number }>(
    "/api/e2ee/secret_storage/secrets",
    body,
    { headers: headers(reauth) },
  );
  return typeof data?.secrets_revision === "number" ? data.secrets_revision : null;
}

/** `GET /api/e2ee/key_backup/versions/current`。今の版がなければ `null`。 */
export async function currentBackupVersion(client: ApiClient): Promise<JsonObject | null> {
  try {
    return await client.get<JsonObject>("/api/e2ee/key_backup/versions/current");
  } catch (error) {
    if (error instanceof ApiError && error.status === 404) return null;
    throw error;
  }
}

/** `GET /api/e2ee/key_backup/keys?version=` を `next` が `null` になるまで読む。消した版は `null`。 */
export async function listBackupSessions(
  client: ApiClient,
  version: string,
): Promise<JsonObject[] | null> {
  const rows: JsonObject[] = [];
  let after: string | null = null;
  try {
    for (;;) {
      const query = `version=${encodeURIComponent(version)}${after ? `&after=${encodeURIComponent(after)}` : ""}`;
      const page: { sessions: JsonObject[]; next: string | null } = await client.get(
        `/api/e2ee/key_backup/keys?${query}`,
      );
      rows.push(...page.sessions);
      if (!page.next) return rows;
      after = page.next;
    }
  } catch (error) {
    if (error instanceof ApiError && error.status === 404) return null;
    throw error;
  }
}

/** 1 回のアップロードの上限（サーバーの `backup_sessions`）。 */
export const BACKUP_UPLOAD_BATCH = 1000;

export async function uploadBackupSessions(
  client: ApiClient,
  version: string,
  secretsRevision: number,
  sessions: JsonObject[],
): Promise<void> {
  for (let start = 0; start < sessions.length; start += BACKUP_UPLOAD_BATCH) {
    await client.post(
      `/api/e2ee/key_backup/keys?version=${encodeURIComponent(version)}&secrets_revision=${secretsRevision}`,
      { sessions: sessions.slice(start, start + BACKUP_UPLOAD_BATCH) },
    );
  }
}

/** 版を消す。すでに消えていれば（404）何もしない。今の版を消すときだけ再認証が要る。 */
export async function deleteBackupVersion(
  client: ApiClient,
  version: string,
  reauth: string | null,
): Promise<void> {
  try {
    await client.delete(`/api/e2ee/key_backup/versions/${encodeURIComponent(version)}`, {
      headers: headers(reauth),
    });
  } catch (error) {
    if (error instanceof ApiError && error.status === 404) return;
    throw error;
  }
}
