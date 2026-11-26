package zoo

import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.json.*

/** サーバーが暗号化設定を格下げしても、作成時の指定と観測済みの設定をアカウントごとに保持する。 */
internal class EncryptedChannelHistory(private val platform: Platform, config: Config) {
    private val key = "zoo-e2ee:encrypted-channels:v1:${config.apiBaseUrl}:${config.userId}"
    private val mutex = Mutex()
    private val known = mutableSetOf<String>()
    private var persisted = emptySet<String>()
    /** 保存した記録を読み終えたか。 */
    var loaded = false
        private set

    fun observe(id: String) { known += id }
    fun contains(id: String): Boolean = id in known

    /** 読み取り・保存に失敗したときは例外で止め、平文の経路へ進ませない。 */
    suspend fun remember(ids: Collection<String> = emptyList()) = mutex.withLock {
        if (!loaded) {
            val saved = platform.readEncryptionHistory(key)?.let { json.parseToJsonElement(it).jsonArray.map { value -> value.jsonPrimitive.content }.toSet() }.orEmpty()
            known += saved
            persisted = saved
            loaded = true
        }
        known += ids
        val snapshot = known.toSet()
        if (snapshot != persisted) {
            platform.writeEncryptionHistory(key, JsonArray(snapshot.sorted().map(::JsonPrimitive)).toString())
            persisted = snapshot
        }
    }
}
