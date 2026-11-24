package zoo.android

import android.content.Context
import java.util.UUID
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.longOrNull
import zoo.crypto.CryptoStore
import zoo.crypto.CryptoStoreFiles
import zoo.crypto.ffi.DecryptedMessage
import zoo.crypto.ffi.EncryptedMessage

/** 通知に出す本文の長さ（平文チャンネルの Push と同じく、先頭 240 文字。コードポイントで数える）。 */
private const val PUSH_BODY_MAX_CODE_POINTS = 240

private fun isUuid(value: String?): Boolean =
    value != null && runCatching { UUID.fromString(value).toString().equals(value, ignoreCase = true) }.getOrDefault(false)

/**
 * Push の `e2ee`（`backend/src/push/fcm.rs` が JSON の文字列で入れる。docs/e2ee.md の「Android の Push の復号」）を
 * 読む。宛先が `user` でない、形が違う、なければ `null`（復号せずに「新しいメッセージ」のまま出す）。中身の照合
 * （チャンネル、行、送信者、epoch など）は zoo-crypto の受信時の検証が行う。Web の `parsePushE2ee` と同じ規則。
 */
internal fun pushEncryptedMessage(raw: String?, user: String): EncryptedMessage? {
    val e2ee = runCatching { Json.parseToJsonElement(raw ?: return null) as? JsonObject }.getOrNull() ?: return null
    val recipient = (e2ee["recipient_id"] as? JsonPrimitive)?.takeIf { it.isString }?.content
    if (!isUuid(recipient) || recipient != user) return null
    val message = e2ee["message"] as? JsonObject ?: return null
    fun text(name: String) = (message[name] as? JsonPrimitive)?.takeIf { it.isString }?.content
    fun count(name: String) = (message[name] as? JsonPrimitive)?.takeIf { !it.isString }?.longOrNull?.takeIf { it >= 0 }
    val parent = message["parent_id"]
    val ciphertext = message["ciphertext"] as? JsonObject ?: return null
    if ((ciphertext["sender_key"] as? JsonPrimitive)?.isString != true ||
        (ciphertext["session_id"] as? JsonPrimitive)?.isString != true) return null
    return EncryptedMessage(
        channelId = text("channel_id") ?: return null,
        messageId = text("message_id") ?: return null,
        parentId = when (parent) {
            null, JsonNull -> null
            else -> (parent as? JsonPrimitive)?.takeIf { it.isString }?.contentOrNull ?: return null
        },
        userId = text("user_id") ?: return null,
        messageKind = text("message_kind") ?: return null,
        epoch = (count("epoch") ?: return null).toULong(),
        revision = (count("revision") ?: return null).toULong(),
        ciphertext = ciphertext.toString(),
    )
}

/** 通知の本文（復号した本文の先頭 240 コードポイント）。 */
internal fun pushBody(body: String): String {
    val end = body.offsetByCodePoints(0, minOf(PUSH_BODY_MAX_CODE_POINTS, body.codePointCount(0, body.length)))
    return body.substring(0, end)
}

/**
 * 暗号化チャンネルの Push の本文を、端末の暗号ストアで復号して返す（#510。Web の #365 と同じ規則）。復号できない
 * （`e2ee` がない、宛先が今のユーザーでない、暗号ストアがない、鍵が届いていない、照合に失敗した、古い版だと
 * 分かった）なら `null` で、サーバーの本文（「新しいメッセージ」）のまま出す。サーバーには何も送らない。
 */
internal fun decryptPushBody(
    data: Map<String, String>,
    user: String,
    decrypt: (EncryptedMessage) -> DecryptedMessage?,
): String? {
    val message = pushEncryptedMessage(data["e2ee"], user) ?: return null
    val decrypted = try { decrypt(message) } catch (_: Exception) { null } ?: return null
    return pushBody(decrypted.payload.content.body)
}

/**
 * [decryptPushBody] を、今のログイン（`binding` が今の端末の登録と一致するとき）の暗号ストアで行う。
 * 本文を出す直前にも、[showPushNotification] がもう一度 `binding` を確かめる。
 */
internal fun decryptPushBody(context: Context, data: Map<String, String>): String? {
    val binding = data["binding"] ?: return null
    val user = data["userId"] ?: return null
    if (data["e2ee"] == null) return null
    // 出さない通知（種類が違う、通知が許可されていない）のためには、保存鍵を読まない。
    if (data["notificationType"] !in setOf("mention", "dm", "reply", "reaction") || !notificationsAllowed(context)) return null
    val store = PushBinding(context)
    if (!store.matches(binding, user)) return null
    val base = store.base(binding) ?: return null
    return decryptPushBody(data, user) { message ->
        CryptoStore.decryptForNotification(CryptoStoreFiles.android(context, base, user), user, message)
    }
}
