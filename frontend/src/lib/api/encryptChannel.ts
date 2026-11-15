import type { ApiClient } from "./client";
import type { ChannelResponse } from "./types";

/**
 * 既存の非公開チャンネルか DM を暗号化に切り替える（#367。`POST /api/channels/{id}/encrypt`）。
 *
 * 元には戻せない。切り替える前のメッセージは平文のまま残り、切り替えたあとのメッセージだけを暗号化する。
 * すでに暗号化されていれば、何もせずに今の行を返す（`txid` は `null`）。
 */
export function encryptChannel(client: ApiClient, channelId: string) {
  return client.post<ChannelResponse>(`/api/channels/${encodeURIComponent(channelId)}/encrypt`);
}

/** 切り替えを止めているもの（409 `E2EE_CONVERSION_BLOCKED` の `blockers`）。 */
export type ConversionBlocker = "apps" | "webhooks" | "encrypted_dm";

const BLOCKERS: readonly ConversionBlocker[] = ["apps", "webhooks", "encrypted_dm"];

/** 409 `E2EE_CONVERSION_BLOCKED` の本文から、知っている `blockers` だけを取り出す。 */
export function conversionBlockers(body: unknown): ConversionBlocker[] {
  if (typeof body !== "object" || body === null) return [];
  const blockers = (body as { blockers?: unknown }).blockers;
  if (!Array.isArray(blockers)) return [];
  return BLOCKERS.filter((blocker) => blockers.includes(blocker));
}
