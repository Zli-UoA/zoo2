package zoo

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import io.ktor.http.encodeURLParameter
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.jsonObject

/** 参加中のスレッドの 1 行（`GET /api/threads` の `items`）。 */
data class JoinedThread(val id: String, val channelId: String, val content: String, val replies: Int)

/**
 * チャンネル一覧の、各チャンネルの下に並べる参加中のスレッド（Web の `useThreadIndex`、iOS の `ChannelThreads`）。
 *
 * - 最初は 1 ページだけ取る。「さらに表示」で古い方へ広げる。広げたあとの更新は、広げた範囲（`through`）まで取り直す。
 * - 返信のないスレッドと、アーカイブ済みのスレッドは出さない。
 * - 取得に失敗しても直前の一覧を残す。
 */
class JoinedThreads(private val request: suspend (String) -> JsonElement) {
    var threads by mutableStateOf<List<JoinedThread>>(emptyList())
        private set
    var hasMore by mutableStateOf(false)
        private set
    var loading by mutableStateOf(false)
        private set
    var error by mutableStateOf(false)
        private set

    private val lock = Mutex()
    private var cursor: String? = null
    private var expanded = false

    fun byChannel(): Map<String, List<JoinedThread>> = threads.groupBy { it.channelId }

    suspend fun refresh() = fetch(append = false)

    suspend fun loadMore() {
        if (cursor == null) return
        fetch(append = true)
    }

    fun clear() {
        threads = emptyList()
        cursor = null
        expanded = false
        hasMore = false
        error = false
    }

    private suspend fun fetch(append: Boolean) = lock.withLock {
        if (append && cursor == null) return@withLock
        val rebuild = !append && expanded
        val through = if (rebuild) cursor else null
        loading = true
        try {
            var before = if (append) cursor else null
            val seen = mutableSetOf<String>()
            val rows = mutableListOf<Row>()
            do {
                before?.let(seen::add)
                val query = buildList {
                    before?.let { add("before=${it.encodeURLParameter()}") }
                    through?.let { add("through=${it.encodeURLParameter()}") }
                }.joinToString("&").let { if (it.isEmpty()) "" else "?$it" }
                val page = request("/api/threads$query").jsonObject
                rows += page.array("items").rows()
                val next = page.text("next_cursor").ifBlank { null }
                if (next != null && next in seen) throw IllegalStateException("Repeated thread cursor")
                before = next
            } while (rebuild && before != null)
            val fetched = rows.map {
                JoinedThread(it.text("id"), it.text("channel_id"), it.text("content"),
                    it.text("reply_count").toIntOrNull() ?: 0) to it.present("thread_archive_at")
            }
            // 更新中に並びが変わったスレッドは、ページの境目で重複することがある。新しい方を使う。
            val merged = LinkedHashMap<String, Pair<JoinedThread, Boolean>>()
            if (append) threads.forEach { merged[it.id] = it to false }
            fetched.forEach { merged[it.first.id] = it }
            threads = merged.values.filter { (thread, archived) -> thread.replies > 0 && !archived }.map { it.first }
            cursor = if (rebuild) through else before
            expanded = append || rebuild
            hasMore = cursor != null
            error = false
        } catch (cancelled: CancellationException) {
            throw cancelled
        } catch (_: Exception) {
            error = true
        } finally {
            loading = false
        }
    }
}
