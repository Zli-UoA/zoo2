package zoo.crypto

import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive

/**
 * 自分の端末のうち、今の自己署名鍵の署名がない端末（設定の端末一覧の「未署名」。Web の `DevicesSettings`）。
 * `keysQuery` は自分だけを問い合わせた `POST /api/e2ee/keys/query` の応答。
 *
 * 表示のために署名があるかだけを見る（検証はしない。Web の `memberReadability.ts` と同じ）。鍵を配るかどうかは、
 * 送信のときに暗号コアが署名を検証して決める。自己署名鍵がなければ（暗号化を設定していなければ）、すべて未署名。
 */
fun unsignedDevices(keysQuery: JsonObject, userId: String, deviceIds: Collection<String>): Set<String> {
    val selfSigning = (keysQuery["self_signing_keys"] as? JsonObject)?.get(userId) as? JsonObject
    val keyIds = (selfSigning?.get("keys") as? JsonObject)?.keys?.filter { it.startsWith("ed25519:") }.orEmpty()
    if (keyIds.isEmpty()) return deviceIds.toSet()
    val devices = (keysQuery["device_keys"] as? JsonObject)?.get(userId) as? JsonObject
    val rows = (keysQuery["signatures"] as? JsonArray).orEmpty().mapNotNull { it as? JsonObject }
    fun text(row: JsonObject, name: String) = (row[name] as? JsonPrimitive)?.takeIf { it.isString }?.content
    return deviceIds.filterNot { deviceId ->
        val embedded = (((devices?.get(deviceId) as? JsonObject)?.get("signatures") as? JsonObject)?.get(userId) as? JsonObject)
        val signedInKeys = embedded != null && keyIds.any { (embedded[it] as? JsonPrimitive)?.isString == true }
        signedInKeys || rows.any { row ->
            text(row, "signer_user_id") == userId && text(row, "target_user_id") == userId &&
                text(row, "target_key_id") == deviceId && text(row, "signer_key_id") in keyIds
        }
    }.toSet()
}
