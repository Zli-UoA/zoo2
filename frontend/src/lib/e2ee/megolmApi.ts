import type { JsonObject, NewTofuPin } from "@zoo/crypto-wasm";

import { ApiError, type ApiClient } from "../api/client";
import type { ClaimRequest, ClaimedKeys, ToDeviceMessages } from "./store/megolm";

/**
 * 暗号化チャンネルの送信に使う API（docs/e2ee.md の「配布先のスナップショット」「端末とワンタイム鍵」
 * 「to-device メッセージ」「クロス署名」）。
 */

/** `keys/claim` で一度に払い出す端末の数（サーバーの上限）。 */
export const KEYS_CLAIM_BATCH = 1000;
/** `to_device` の 1 回の宛先の数（サーバーの上限）。 */
export const TO_DEVICE_BATCH = 100;
/** `recipients` を最初のページから取り直す回数の上限。 */
const RECIPIENTS_ATTEMPTS = 5;

/** API のエラーの `code`（`{"error", "code"}`）。なければ `null`。 */
export function apiErrorCode(error: unknown): string | null {
  if (!(error instanceof ApiError)) return null;
  const body = error.body as { code?: unknown } | null;
  return typeof body?.code === "string" ? body.code : null;
}

/** 409 `E2EE_STALE_REVISION` の `current_revision`。違うエラーなら `null`。 */
export function staleRevision(error: unknown): number | null {
  if (apiErrorCode(error) !== "E2EE_STALE_REVISION") return null;
  const current = ((error as ApiError).body as { current_revision?: unknown } | null)
    ?.current_revision;
  return typeof current === "number" ? current : null;
}

/** 409 `E2EE_STALE_EPOCH` の `current_epoch`。違うエラーなら `null`。 */
export function staleEpoch(error: unknown): bigint | null {
  if (apiErrorCode(error) !== "E2EE_STALE_EPOCH") return null;
  const current = ((error as ApiError).body as { current_epoch?: unknown } | null)?.current_epoch;
  return typeof current === "number" || typeof current === "string" ? BigInt(current) : null;
}

/**
 * 配布先のスナップショット（`GET /api/e2ee/channels/{id}/recipients`）を全ページ取る。
 * ページの途中で `membership_epoch` が変わったら（409）、最初のページから取り直す。
 * `minEpoch` より古いスナップショットも取り直す（409 で受けた `current_epoch` 以上にするため）。
 */
export async function fetchRecipients(
  client: ApiClient,
  channelId: string,
  minEpoch: bigint | null,
  signal?: AbortSignal,
): Promise<JsonObject[]> {
  const base = `/api/e2ee/channels/${encodeURIComponent(channelId)}/recipients`;
  for (let attempt = 0; attempt < RECIPIENTS_ATTEMPTS; attempt += 1) {
    const pages: JsonObject[] = [];
    let epoch: string | null = null;
    let after: string | null = null;
    try {
      do {
        const params = new URLSearchParams();
        if (after !== null) params.set("after", after);
        if (epoch !== null) params.set("epoch", epoch);
        const query = params.size > 0 ? `?${params}` : "";
        const page: JsonObject = await client.request<JsonObject>(`${base}${query}`, {
          method: "GET",
          signal,
        });
        pages.push(page);
        const pageEpoch = page.membership_epoch;
        epoch ??=
          typeof pageEpoch === "number" || typeof pageEpoch === "string" ? String(pageEpoch) : "0";
        after = typeof page.next_after === "string" ? page.next_after : null;
      } while (after !== null);
    } catch (error) {
      if (staleEpoch(error) !== null) continue;
      throw error;
    }
    if (minEpoch !== null && BigInt(epoch ?? "0") < minEpoch) continue;
    return pages;
  }
  throw new Error("配布先のスナップショットを取れませんでした");
}

/** TOFU のピンを 1 件置く。409 `E2EE_PIN_CONFLICT`（別の端末が先に置いた）なら `false`。 */
export async function postTofuPin(
  client: ApiClient,
  pin: NewTofuPin,
  signal?: AbortSignal,
): Promise<boolean> {
  try {
    await client.request("/api/e2ee/tofu_pins", {
      method: "POST",
      body: { pins: [pin.pin], replace: false },
      signal,
    });
    return true;
  } catch (error) {
    if (apiErrorCode(error) === "E2EE_PIN_CONFLICT") return false;
    throw error;
  }
}

/** ワンタイム鍵を払い出す（1000 台ずつ）。 */
export async function claimKeys(
  client: ApiClient,
  claim: ClaimRequest,
  signal?: AbortSignal,
): Promise<ClaimedKeys> {
  const entries = Object.entries(claim).flatMap(([userId, devices]) =>
    Object.keys(devices).map((deviceId) => [userId, deviceId] as const),
  );
  const claimed: ClaimedKeys = {};
  for (let start = 0; start < entries.length; start += KEYS_CLAIM_BATCH) {
    const batch: ClaimRequest = {};
    for (const [userId, deviceId] of entries.slice(start, start + KEYS_CLAIM_BATCH)) {
      (batch[userId] ??= {})[deviceId] = "signed_curve25519";
    }
    const { data: response } = await client.post<{ one_time_keys?: ClaimedKeys }>(
      "/api/e2ee/keys/claim",
      { one_time_keys: batch },
      { signal },
    );
    for (const [userId, devices] of Object.entries(response.one_time_keys ?? {})) {
      Object.assign((claimed[userId] ??= {}), devices);
    }
  }
  return claimed;
}

/**
 * to-device で送る（100 端末ずつ、リクエストごとに別の `txn_id`）。キューの上限で入れなかった
 * 宛先（`undelivered`）を返す。
 */
export async function sendToDevice(
  client: ApiClient,
  deviceId: string,
  messages: ToDeviceMessages,
  signal?: AbortSignal,
): Promise<{ userId: string; deviceId: string }[]> {
  const entries = Object.entries(messages).flatMap(([userId, devices]) =>
    Object.entries(devices).map(([target, content]) => ({ userId, deviceId: target, content })),
  );
  const undelivered: { userId: string; deviceId: string }[] = [];
  for (let start = 0; start < entries.length; start += TO_DEVICE_BATCH) {
    const batch: ToDeviceMessages = {};
    for (const entry of entries.slice(start, start + TO_DEVICE_BATCH)) {
      (batch[entry.userId] ??= {})[entry.deviceId] = entry.content;
    }
    const { data: response } = await client.post<{
      undelivered?: { user_id: string; device_id: string }[];
    }>(
      "/api/e2ee/to_device",
      {
        device_id: deviceId,
        txn_id: crypto.randomUUID(),
        type: "m.room.encrypted",
        messages: batch,
      },
      { signal },
    );
    for (const target of response.undelivered ?? []) {
      undelivered.push({ userId: target.user_id, deviceId: target.device_id });
    }
  }
  return undelivered;
}
