package zoo

import kotlinx.coroutines.flow.StateFlow
import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue

/** Android の暗号ストアを共有チャットから使う。平文を同期キャッシュには書き込まない。 */
interface MessageEncryption {
    val changes: StateFlow<Long>
    suspend fun send(channel: String, id: String, parent: String?, body: String)
    suspend fun edit(message: Row, body: String)
    suspend fun decrypt(message: Row): DecryptedBody
}

enum class MessageRevision { LATEST, STALE, CONFLICT }

data class DecryptedBody(
    val body: String? = null,
    val notice: String? = null,
    val revision: MessageRevision = MessageRevision.LATEST,
) {
    val revisionNotice: String? get() = when (revision) {
        MessageRevision.LATEST -> null
        MessageRevision.STALE -> "e2ee.revision.stale"
        MessageRevision.CONFLICT -> "e2ee.revision.conflict"
    }
}

/** 表示メタデータを復号処理のキーに含めない。 */
internal fun encryptedMessagePayload(message: Row): Row = kotlinx.serialization.json.JsonObject(
    message.filterKeys { it in setOf("channel_id", "id", "parent_id", "user_id", "kind", "epoch", "revision", "ciphertext") }
)

/** 確認済みの本文は画面内だけで保持する。旧版・同版の競合で置き換えない。 */
internal class EncryptedMessageDisplayState {
    var decoded by androidx.compose.runtime.mutableStateOf<DecryptedBody?>(null)
        private set

    fun receive(result: DecryptedBody) {
        decoded = if (result.revision == MessageRevision.LATEST) result
        else result.copy(body = decoded?.body, notice = decoded?.notice)
    }
}
