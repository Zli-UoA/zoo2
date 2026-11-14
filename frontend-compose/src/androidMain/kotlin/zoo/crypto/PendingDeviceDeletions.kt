package zoo.crypto

import java.io.File
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.builtins.ListSerializer

/** 削除 API が失敗した端末。同じ接続先とユーザーの次のログインで消し直す。秘密ではない。 */
@Serializable
data class PendingDeviceDeletion(
    val server: String,
    @SerialName("user_id") val userId: String,
    @SerialName("device_id") val deviceId: String,
)

/**
 * 削除待ちの端末の一覧（docs/e2ee.md の「ログアウトと端末の削除」）。React の
 * `zoo.e2ee.pendingDeviceDelete`、TUI の `pending-device-delete.json` と同じ役目。
 *
 * 接続先、ユーザー、端末の組ごとに残し、別のユーザーの削除待ちを上書きしない。
 * 一覧は全ユーザーで共有するので、読んで書き戻すあいだはプロセスの中で直列にする。
 */
class PendingDeviceDeletions(private val file: File) {
    fun add(entry: PendingDeviceDeletion) = synchronized(LOCK) {
        val all = read()
        if (entry !in all) write(all + entry)
    }

    fun remove(entry: PendingDeviceDeletion) = synchronized(LOCK) {
        val rest = read() - entry
        if (rest.isEmpty()) file.delete() else write(rest)
    }

    fun all(): List<PendingDeviceDeletion> = synchronized(LOCK) { read() }

    private fun read(): List<PendingDeviceDeletion> =
        try {
            if (file.exists()) CryptoStore.json.decodeFromString(SERIALIZER, file.readText()) else emptyList()
        } catch (_: Exception) {
            emptyList()
        }

    private fun write(entries: List<PendingDeviceDeletion>) {
        CryptoStore.writeAtomically(file, CryptoStore.json.encodeToString(SERIALIZER, entries).encodeToByteArray())
    }

    companion object {
        private val LOCK = Any()
        private val SERIALIZER = ListSerializer(PendingDeviceDeletion.serializer())

        fun android(context: android.content.Context) =
            PendingDeviceDeletions(File(CryptoStoreFiles.root(context), "pending-device-delete.json"))
    }
}
