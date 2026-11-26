import type { JsonObject } from "@zoo/crypto-wasm";

/**
 * 暗号化チャンネルのメンバーが、新しいメッセージを読めない理由（メンバー欄に出す）。
 *
 * - `notSetUp`：暗号化を設定していない（クロス署名のマスター鍵か自己署名鍵がない）。鍵を配れない。
 * - `noDevices`：設定はしたが、使える端末が 1 台もない。今の自己署名鍵で署名された端末だけを数える
 *   （配布先の判定と同じく、署名のない端末には鍵を配らない）。すべてログアウトした、解錠していない端末しか
 *   ない、など。
 *
 * 署名は、あるかどうかだけを見る（検証はしない）。表示のための目安で、鍵を配るかどうかは、送信のときに
 * 暗号コアが署名を検証して決める。
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

/** 自己署名鍵の鍵 ID（`ed25519:<公開鍵>`）。 */
function selfSigningKeyIds(selfSigningKey: unknown): string[] {
  const keys = objectAt(objectAt(selfSigningKey)?.keys);
  return keys ? Object.keys(keys).filter((id) => id.startsWith("ed25519:")) : [];
}

/** 端末に、自己署名鍵の署名があるか（端末鍵の `signatures` か、`signatures` の行）。 */
function signedBySelfSigningKey(
  userId: string,
  deviceId: string,
  deviceJson: unknown,
  keyIds: string[],
  rows: unknown[],
): boolean {
  const embedded = objectAt(objectAt(objectAt(deviceJson)?.signatures)?.[userId]);
  if (embedded && keyIds.some((id) => typeof embedded[id] === "string")) return true;
  return rows.some((value) => {
    const row = objectAt(value);
    return (
      !!row &&
      row.signer_user_id === userId &&
      row.target_user_id === userId &&
      row.target_key_id === deviceId &&
      typeof row.signer_key_id === "string" &&
      keyIds.includes(row.signer_key_id)
    );
  });
}

/** スナップショットの全ページから、読めないメンバーとその理由を求める。 */
export function unreadableMembers(pages: JsonObject[]): Map<string, MemberUnreadableReason> {
  const result = new Map<string, MemberUnreadableReason>();
  for (const page of pages) {
    const userIds = Array.isArray(page.user_ids) ? page.user_ids : [];
    const masterKeys = objectAt(page.master_keys) ?? {};
    const selfSigningKeys = objectAt(page.self_signing_keys) ?? {};
    const deviceKeys = objectAt(page.device_keys) ?? {};
    const rows = Array.isArray(page.signatures) ? page.signatures : [];
    for (const userId of userIds) {
      if (typeof userId !== "string") continue;
      const keyIds = selfSigningKeyIds(selfSigningKeys[userId]);
      if (!masterKeys[userId] || keyIds.length === 0) {
        result.set(userId, "notSetUp");
        continue;
      }
      const devices = objectAt(deviceKeys[userId]) ?? {};
      const usable = Object.entries(devices).some(([deviceId, json]) =>
        signedBySelfSigningKey(userId, deviceId, json, keyIds, rows),
      );
      if (!usable) result.set(userId, "noDevices");
    }
  }
  return result;
}
