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
    /**
     * 登録できたかを確かめられていない（登録の要求の結果を受け取っていない）。この端末の 404 は、
     * あとから届いた登録で端末ができうるので、すぐには消えたものとしない。
     */
    val unconfirmed: Boolean = false,
    /** `unconfirmed` の端末で、続けて 404 を受け取った回数。 */
    @SerialName("not_found") val notFound: Int = 0,
) {
    /** 同じ端末かどうか（記録の状態は比べない）。 */
    fun sameDevice(other: PendingDeviceDeletion) =
        server == other.server && userId == other.userId && deviceId == other.deviceId
}

/**
 * 削除待ちの端末の一覧（docs/e2ee.md の「ログアウトと端末の削除」）。React の
 * `zoo.e2ee.pendingDeviceDelete`、TUI の `pending-device-delete.json` と同じ役目。
 *
 * 接続先、ユーザー、端末の組ごとに残し、別のユーザーの削除待ちを上書きしない。
 * 一覧は全ユーザーで共有するので、読んで書き戻すあいだはプロセスの中で直列にする。
 * ファイルを読めないときは例外を投げ、ファイルを変えない（空の一覧で上書きして、ほかの削除待ちを
 * 失わないため）。
 */
class PendingDeviceDeletions(private val file: File) {
    /** 足す。同じ端末があれば、`unconfirmed` を合わせて（どちらかが真なら真）置き換える。 */
    fun add(entry: PendingDeviceDeletion) = synchronized(LOCK) {
        val all = read()
        val existing = all.find { it.sameDevice(entry) }
        val merged = existing?.copy(unconfirmed = existing.unconfirmed || entry.unconfirmed) ?: entry
        if (merged != existing) write(all.filterNot { it.sameDevice(entry) } + merged)
    }

    /** 同じ端末の記録を、`entry` に置き換える（404 の回数を数えるため）。なければ何もしない。 */
    fun update(entry: PendingDeviceDeletion) = synchronized(LOCK) {
        val all = read()
        if (all.any { it.sameDevice(entry) }) write(all.map { if (it.sameDevice(entry)) entry else it })
    }

    fun remove(entry: PendingDeviceDeletion) = synchronized(LOCK) {
        val all = read()
        val rest = all.filterNot { it.sameDevice(entry) }
        if (rest.size == all.size) return@synchronized
        if (rest.isEmpty()) {
            if (!file.delete() && file.exists()) throw java.io.IOException("削除待ちの一覧を消せません")
        } else {
            write(rest)
        }
    }

    fun all(): List<PendingDeviceDeletion> = synchronized(LOCK) { read() }

    /** ファイルがなければ空。読めない、形式が違うときは例外を投げる。 */
    private fun read(): List<PendingDeviceDeletion> {
        if (!file.exists()) return emptyList()
        return CryptoStore.json.decodeFromString(SERIALIZER, file.readText())
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
