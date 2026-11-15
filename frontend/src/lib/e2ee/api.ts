import type { ApiClient } from "../api/client";
import type { KeyUpload, KeysQueryResponse, RegistrationRequest } from "./protocol";

/**
 * 端末と鍵、to-device の API（docs/e2ee.md の「端末とワンタイム鍵」「to-device メッセージ」）。
 */

export interface KeyCounts {
  one_time_key_counts: { signed_curve25519?: number };
  unused_fallback_key_types: string[];
}

/** `keys/query` で一度に問い合わせるユーザーの数（docs/e2ee.md の【決定】）。 */
export const KEYS_QUERY_BATCH = 50;
/** `to_device/ack` で一度に送る ID の数（サーバーの上限）。 */
export const TO_DEVICE_ACK_BATCH = 1000;

export async function registerDevice(
  client: ApiClient,
  request: RegistrationRequest & { display_name: string },
): Promise<KeyCounts> {
  const { data } = await client.post<KeyCounts>("/api/e2ee/devices", request);
  return data;
}

export async function uploadKeys(
  client: ApiClient,
  deviceId: string,
  keys: KeyUpload | null,
  signal?: AbortSignal,
): Promise<KeyCounts> {
  const { data } = await client.post<KeyCounts>(
    "/api/e2ee/keys/upload",
    {
      device_id: deviceId,
      one_time_keys: keys?.one_time_keys ?? {},
      fallback_keys: keys?.fallback_keys ?? {},
    },
    { signal },
  );
  return data;
}

/** `keys/query`（{@link KEYS_QUERY_BATCH} 人ずつ）。`device_keys`、`master_keys`、`signatures` などをまとめる。 */
export async function queryKeys(
  client: ApiClient,
  userIds: string[],
  signal?: AbortSignal,
): Promise<KeysQueryResponse> {
  const merged: KeysQueryResponse = {
    device_keys: {},
    master_keys: {},
    self_signing_keys: {},
    user_signing_keys: {},
    tofu_pins: {},
    signatures: [],
  };
  for (let start = 0; start < userIds.length; start += KEYS_QUERY_BATCH) {
    const page = await client.request<Partial<KeysQueryResponse>>("/api/e2ee/keys/query", {
      method: "POST",
      body: { user_ids: userIds.slice(start, start + KEYS_QUERY_BATCH) },
      signal,
    });
    Object.assign(merged.device_keys, page.device_keys ?? {});
    Object.assign(merged.master_keys, page.master_keys ?? {});
    Object.assign(merged.self_signing_keys, page.self_signing_keys ?? {});
    Object.assign(merged.user_signing_keys, page.user_signing_keys ?? {});
    Object.assign(merged.tofu_pins, page.tofu_pins ?? {});
    merged.signatures.push(...(page.signatures ?? []));
  }
  return merged;
}

export async function ackToDevice(
  client: ApiClient,
  deviceId: string,
  ids: string[],
  signal?: AbortSignal,
): Promise<void> {
  for (let start = 0; start < ids.length; start += TO_DEVICE_ACK_BATCH) {
    await client.post(
      "/api/e2ee/to_device/ack",
      { device_id: deviceId, ids: ids.slice(start, start + TO_DEVICE_ACK_BATCH) },
      { signal },
    );
  }
}

/** 端末を消す。消し済みでも 200（冪等）。 */
export async function deleteDevice(
  client: ApiClient,
  deviceId: string,
  options: { signal?: AbortSignal; keepalive?: boolean } = {},
): Promise<void> {
  await client.delete(`/api/e2ee/devices/${encodeURIComponent(deviceId)}`, options);
}
