package zoo

import kotlinx.coroutines.flow.StateFlow

/** Android の暗号ストアを共有チャットから使う。平文を同期キャッシュには書き込まない。 */
interface MessageEncryption {
    val changes: StateFlow<Long>
    suspend fun send(channel: String, id: String, parent: String?, body: String)
    suspend fun edit(message: Row, body: String)
    suspend fun decrypt(message: Row): DecryptedBody
}

data class DecryptedBody(val body: String? = null, val notice: String? = null)
