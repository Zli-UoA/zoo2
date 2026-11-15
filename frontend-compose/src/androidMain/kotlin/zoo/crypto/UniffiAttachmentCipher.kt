package zoo.crypto

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonObject
import zoo.AttachmentCipher
import zoo.EncryptedBytes
import zoo.crypto.ffi.decryptAttachment
import zoo.crypto.ffi.encryptAttachment

/** zoo-crypto の UniFFI（#353）で添付を暗号化、復号する（#364）。 */
object UniffiAttachmentCipher : AttachmentCipher {
    override fun encrypt(plaintext: ByteArray): EncryptedBytes {
        val encrypted = encryptAttachment(plaintext)
        return EncryptedBytes(encrypted.ciphertext, Json.parseToJsonElement(encrypted.file).jsonObject)
    }

    override fun decrypt(file: JsonObject, ciphertext: ByteArray): ByteArray =
        decryptAttachment(file.toString(), ciphertext)
}
