package zoo

import io.ktor.http.encodeURLParameter
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.long
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put

/*
 * 暗号化チャンネルの添付（#364。docs/e2ee.md の「暗号化した添付」）。
 *
 * ファイルはクライアントで AES-256-CTR で暗号化してから S3 に上げ、鍵、IV、ハッシュと本当のファイル名、
 * MIME、画像の大きさは、Megolm で暗号化する本文の `attachments`（zoo-crypto の `AttachmentInfo`）に入れる。
 * サーバーには、決まったファイル名と Content-Type と暗号文の大きさだけを渡す。React（Web）の
 * `frontend/src/lib/e2ee/attachments.ts` と同じ手順。
 */

/** zoo-crypto の `ENCRYPTED_ATTACHMENT_FILE_NAME`。 */
const val ENCRYPTED_ATTACHMENT_FILE_NAME = "encrypted.bin"

/** zoo-crypto の `ENCRYPTED_ATTACHMENT_CONTENT_TYPE`。 */
const val ENCRYPTED_ATTACHMENT_CONTENT_TYPE = "application/octet-stream"

/** 暗号化の結果。`file` は Matrix の `EncryptedFile`（鍵を含むので、ログに出さない）。 */
class EncryptedBytes(val ciphertext: ByteArray, val file: JsonObject)

/** 添付の暗号化と復号（Android では zoo-crypto の UniFFI。`zoo.crypto.UniffiAttachmentCipher`）。 */
interface AttachmentCipher {
    fun encrypt(plaintext: ByteArray): EncryptedBytes

    /** 暗号文のハッシュを確かめてから復号する。合わなければ投げる。 */
    fun decrypt(file: JsonObject, ciphertext: ByteArray): ByteArray
}

/** 暗号化チャンネルの `POST /api/attachments/presign` の本文。決まった値と暗号文の大きさだけを送る。 */
fun encryptedPresignBody(channel: String, ciphertextSize: Int): JsonObject = buildJsonObject {
    put("channel_id", channel)
    put("file_name", ENCRYPTED_ATTACHMENT_FILE_NAME)
    put("file_size", ciphertextSize.toLong())
    put("content_type", ENCRYPTED_ATTACHMENT_CONTENT_TYPE)
}

/** 本文の `attachments` に入れる 1 つの添付（zoo-crypto の `AttachmentInfo` と同じ JSON）。 */
fun encryptedAttachmentInfo(
    attachmentId: String,
    name: String,
    mimetype: String,
    size: Int,
    file: JsonObject,
    width: Int? = null,
    height: Int? = null,
    sensitive: Boolean = false,
): JsonObject = buildJsonObject {
    put("attachment_id", attachmentId)
    put("name", name)
    put("mimetype", mimetype)
    put("size", size.toLong())
    if (width != null && height != null) {
        put("width", width)
        put("height", height)
    }
    if (sensitive) put("sensitive", true)
    put("file", file)
}

/**
 * `plaintext` を暗号化して上げ、本文の `attachments` に入れる値を返す。送信の `attachment_ids` には
 * 返した値の `attachment_id` を入れる。平文はサーバーにも S3 にも渡さない。
 */
suspend fun uploadEncryptedAttachment(
    api: ZooApi,
    cipher: AttachmentCipher,
    channel: String,
    plaintext: ByteArray,
    name: String,
    mimetype: String,
    width: Int? = null,
    height: Int? = null,
    sensitive: Boolean = false,
): JsonObject {
    val encrypted = cipher.encrypt(plaintext)
    val presign =
        attachmentPayload(
            api.request(
                "/api/attachments/presign",
                "POST",
                encryptedPresignBody(channel, encrypted.ciphertext.size),
            )
        )
    val attachmentId = presign.text("attachment_id")
    require(attachmentId.isNotEmpty())
    api.putBytes(presign.text("upload_url"), encrypted.ciphertext, ENCRYPTED_ATTACHMENT_CONTENT_TYPE)
    return encryptedAttachmentInfo(
        attachmentId,
        name,
        mimetype,
        plaintext.size,
        encrypted.file,
        width,
        height,
        sensitive,
    )
}

/**
 * 本文の `attachments` の 1 つ（`info`）を取得して復号する。暗号文のハッシュが本文の値と合わなければ投げる
 * （サーバーや S3 が中身を差し替えても、平文を返さない）。
 */
suspend fun fetchEncryptedAttachment(
    api: ZooApi,
    cipher: AttachmentCipher,
    info: JsonObject,
): ByteArray {
    val id = info.text("attachment_id")
    val url =
        attachmentPayload(api.request("/api/attachments/${id.encodeURLParameter()}/url"))
            .text("download_url")
    val ciphertext = api.getBytes(url)
    val plaintext = cipher.decrypt(info.getValue("file").jsonObject, ciphertext)
    check(plaintext.size.toLong() == info.getValue("size").jsonPrimitive.long) {
        "attachment size mismatch"
    }
    return plaintext
}
