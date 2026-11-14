import type { JsonObject } from "@zoo/crypto-wasm";

/**
 * 暗号化チャンネルのメンバーが、新しいメッセージを読めない理由（メンバー欄に出す）。
 *
 * - `notSetUp`：暗号化を設定していない（クロス署名のマスター鍵か自己署名鍵がない）。鍵を配れない。
 * - `noDevices`：設定はしたが、使える端末が 1 台もない（すべてログアウトした、など）。
 *
 * 判定は配布先のスナップショット（`GET /api/e2ee/channels/{id}/recipients`）だけで行い、見る人の
 * 信頼の記録（再検証待ちなど）は使わない。見る人ごとの理由は、送信のときの「次の人は新しいメッセージを
 * 読めません」に出す（`EncryptionRecipientsNotice`）。
 */
export type MemberUnreadableReason = "notSetUp" | "noDevices";

function objectAt(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

/** スナップショットの全ページから、読めないメンバーとその理由を求める。 */
export function unreadableMembers(pages: JsonObject[]): Map<string, MemberUnreadableReason> {
  const result = new Map<string, MemberUnreadableReason>();
  for (const page of pages) {
    const userIds = Array.isArray(page.user_ids) ? page.user_ids : [];
    const masterKeys = objectAt(page.master_keys) ?? {};
    const selfSigningKeys = objectAt(page.self_signing_keys) ?? {};
    const deviceKeys = objectAt(page.device_keys) ?? {};
    for (const userId of userIds) {
      if (typeof userId !== "string") continue;
      if (!masterKeys[userId] || !selfSigningKeys[userId]) {
        result.set(userId, "notSetUp");
        continue;
      }
      const devices = objectAt(deviceKeys[userId]);
      if (!devices || Object.keys(devices).length === 0) result.set(userId, "noDevices");
    }
  }
  return result;
}
