import { ApiError, type ApiClient } from "../api/client";
import type { JsonObject, KeySlot, SignatureUpload } from "@zoo/crypto-wasm";

/**
 * 初回設定と解錠で使う API（docs/e2ee.md の「クロス署名」「Secret Storage」「鍵バックアップ」。#360）。
 *
 * 直前の再認証が要る要求には、`reauth` に再認証で受け取った ID トークンを渡す（`X-Zoo-Reauth`）。
 */

/** `GET /api/e2ee/secret_storage` の応答。SSSS がまだなければ `default_key_id` などがない。 */
export interface SecretStorageResponse {
  default_key_id: string | null;
  secrets_revision: number | null;
  slots: (KeySlot & { created_at?: string; last_used_at?: string | null })[];
}

export const REAUTH_HEADER = "X-Zoo-Reauth";

function reauthHeaders(reauth: string | null | undefined): Record<string, string> | undefined {
  return reauth ? { [REAUTH_HEADER]: reauth } : undefined;
}

export function errorCode(error: unknown): string | null {
  if (!(error instanceof ApiError)) return null;
  const code = (error.body as { code?: unknown } | null)?.code;
  return typeof code === "string" ? code : null;
}

export function isReauthRequired(error: unknown): boolean {
  return (
    error instanceof ApiError && error.status === 403 && errorCode(error) === "E2EE_REAUTH_REQUIRED"
  );
}

/** 409 `E2EE_SSSS_CONFLICT` の追加フィールド。 */
export function ssssConflict(
  error: unknown,
): { default_key_id: string | null; secrets_revision: number | null } | null {
  if (!(error instanceof ApiError) || error.status !== 409) return null;
  if (errorCode(error) !== "E2EE_SSSS_CONFLICT") return null;
  const body = error.body as { default_key_id?: unknown; secrets_revision?: unknown };
  return {
    default_key_id: typeof body.default_key_id === "string" ? body.default_key_id : null,
    secrets_revision: typeof body.secrets_revision === "number" ? body.secrets_revision : null,
  };
}

export async function getSecretStorage(client: ApiClient): Promise<SecretStorageResponse> {
  return client.get<SecretStorageResponse>("/api/e2ee/secret_storage");
}

export function hasSecretStorage(response: SecretStorageResponse): boolean {
  return typeof response.default_key_id === "string" && response.default_key_id.length > 0;
}

/** `POST /api/e2ee/secret_storage/keys`。増やしたあとの `secrets_revision` を返す。 */
export async function createSecretStorageKey(
  client: ApiClient,
  body: JsonObject,
  reauth: string | null,
): Promise<number | null> {
  const { data } = await client.post<{ secrets_revision?: number }>(
    "/api/e2ee/secret_storage/keys",
    body,
    { headers: reauthHeaders(reauth) },
  );
  return typeof data?.secrets_revision === "number" ? data.secrets_revision : null;
}

export async function addKeySlot(
  client: ApiClient,
  keyId: string,
  slot: KeySlot,
  reauth: string | null,
): Promise<void> {
  // サーバーの `label` は省略できるが `null` は受け付けないので、ないときは送らない。
  const { label, ...rest } = slot;
  await client.post(
    "/api/e2ee/secret_storage/slots",
    { ...rest, key_id: keyId, ...(label ? { label } : {}) },
    { headers: reauthHeaders(reauth) },
  );
}

export async function deleteKeySlot(
  client: ApiClient,
  slotId: string,
  reauth: string | null,
): Promise<void> {
  await client.delete(`/api/e2ee/secret_storage/slots/${encodeURIComponent(slotId)}`, {
    headers: reauthHeaders(reauth),
  });
}

/** 最後に使った日時を記録する。失敗しても解錠は止めない。 */
export async function markKeySlotUsed(client: ApiClient, slotId: string): Promise<void> {
  try {
    await client.post(`/api/e2ee/secret_storage/slots/${encodeURIComponent(slotId)}/used`, {});
  } catch {
    // 表示用の記録なので、失敗は無視する。
  }
}

/** 自分だけを問い合わせた `keys/query`（クロス署名鍵と署名）。 */
export async function queryOwnKeys(client: ApiClient, userId: string): Promise<JsonObject> {
  return client.request<JsonObject>("/api/e2ee/keys/query", {
    method: "POST",
    body: { user_ids: [userId] },
  });
}

/** `GET /api/e2ee/key_backup/versions`（消した版を含むすべての版）。 */
export async function listBackupVersions(client: ApiClient): Promise<JsonObject> {
  return client.get<JsonObject>("/api/e2ee/key_backup/versions");
}

export async function uploadCrossSigningKeys(
  client: ApiClient,
  body: JsonObject,
  reauth: string | null,
): Promise<void> {
  await client.post("/api/e2ee/cross_signing/keys", body, { headers: reauthHeaders(reauth) });
}

export async function uploadSignatures(
  client: ApiClient,
  signatures: SignatureUpload[],
): Promise<void> {
  await client.post("/api/e2ee/cross_signing/signatures", { signatures });
}

/** `GET /api/e2ee/key_backup/versions/current`。今の版がなければ `null`。 */
export async function getCurrentBackupVersion(client: ApiClient): Promise<JsonObject | null> {
  try {
    return await client.get<JsonObject>("/api/e2ee/key_backup/versions/current");
  } catch (error) {
    if (error instanceof ApiError && error.status === 404) return null;
    throw error;
  }
}

/** `POST /api/e2ee/key_backup/keys`。 */
export async function uploadBackupKeys(
  client: ApiClient,
  version: string,
  secretsRevision: number,
  body: JsonObject,
): Promise<void> {
  const query = new URLSearchParams({ version, secrets_revision: String(secretsRevision) });
  await client.post(`/api/e2ee/key_backup/keys?${query.toString()}`, body);
}

/** `POST /api/e2ee/key_backup/versions`。作った（または同じ公開鍵の今の）版を返す。 */
export async function createBackupVersion(
  client: ApiClient,
  body: JsonObject,
  reauth: string | null,
): Promise<string | null> {
  const { data } = await client.post<{ version?: string }>("/api/e2ee/key_backup/versions", body, {
    headers: reauthHeaders(reauth),
  });
  return typeof data?.version === "string" ? data.version : null;
}
