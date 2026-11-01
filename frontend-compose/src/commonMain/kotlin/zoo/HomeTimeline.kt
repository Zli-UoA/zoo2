package zoo

import androidx.compose.runtime.*
import io.ktor.http.encodeURLParameter
import kotlinx.coroutines.*

/** Owns the retained paging window; live refreshes wait for an outstanding older page. */
@Stable
class HomeTimeline(
    private val app: Session,
    private val scope: CoroutineScope,
    private val postId: String? = null,
    private val actorId: String? = null,
) {
    var timeline by mutableStateOf("home")
        private set

    var posts by mutableStateOf<List<Row>>(emptyList())
        private set

    var loading by mutableStateOf(true)
        private set

    var loadingOlder by mutableStateOf(false)
        private set

    var hasMore by mutableStateOf(false)
        private set

    var error by mutableStateOf(false)
        private set

    private var refreshJob: Job? = null
    private var olderJob: Job? = null
    private var deferred = false
    private var generation = 0
    private var windowSize = 50
    private var tail: Row? = null

    fun select(value: String) {
        if (timeline == value) return
        timeline = value
        deferred = false
        generation++
        refreshJob?.cancel()
        olderJob?.cancel()
        posts = emptyList()
        loading = true
        loadingOlder = false
        windowSize = 50
        tail = null
        refresh()
    }

    fun refresh(retain: Boolean = false) {
        if (retain && loadingOlder) {
            deferred = true
            return
        }
        refreshJob?.cancel()
        val version = ++generation
        if (!retain) {
            olderJob?.cancel()
            loadingOlder = false
        }
        val size = if (retain) windowSize else 50
        val retainedTail = if (retain) tail else null
        refreshJob =
            scope.launch {
                error = false
                try {
                    val result = mutableListOf<Row>()
                    var before = ""
                    var more: Boolean
                    do {
                        val page =
                            if (postId != null)
                                listOf(
                                    app.api.request("/api/home/${postId.encodeURLParameter()}")
                                        as Row
                                )
                            else app.api.request(path(size, before)).rows()
                        result.addAll(page.filter { row -> result.none { key(it) == key(row) } })
                        more = page.size >= size
                        before = page.lastOrNull()?.let(::key).orEmpty()
                        val reached =
                            retainedTail == null ||
                                result.any { key(it) == key(retainedTail) } ||
                                (result.lastOrNull()?.text("sort_key")?.let {
                                    numericCompare(it, retainedTail.text("sort_key")) < 0
                                } == true)
                        if (!more || (result.size >= size && reached)) break
                    } while (result.size < 500 && before.isNotEmpty())
                    if (version == generation) {
                        posts = result
                        hasMore = more
                        windowSize = maxOf(50, result.size)
                        tail = result.lastOrNull()
                    }
                } catch (e: CancellationException) {
                    throw e
                } catch (_: Exception) {
                    if (version == generation) error = true
                } finally {
                    if (version == generation) loading = false
                }
            }
    }

    fun older() {
        if (loading || loadingOlder || !hasMore || posts.isEmpty()) return
        val version = generation
        loadingOlder = true
        error = false
        olderJob =
            scope.launch {
                try {
                    val page = app.api.request(path(50, key(posts.last()))).rows()
                    if (version == generation) {
                        posts = posts + page.filter { row -> posts.none { key(it) == key(row) } }
                        hasMore = page.size == 50
                        windowSize = maxOf(windowSize, posts.size)
                        tail = posts.lastOrNull()
                    }
                } catch (e: CancellationException) {
                    throw e
                } catch (_: Exception) {
                    if (version == generation) error = true
                } finally {
                    if (version == generation) {
                        loadingOlder = false
                        if (deferred) {
                            deferred = false
                            refresh(true)
                        }
                    }
                }
            }
    }

    fun close() {
        generation++
        refreshJob?.cancel()
        olderJob?.cancel()
    }

    private fun path(size: Int, before: String) = homeTimelinePath(timeline, actorId, size, before)

    companion object {
        fun key(row: Row): String = row.text("entry_id").ifEmpty { row.text("id") }

        internal fun numericCompare(a: String, b: String): Int {
            fun clean(s: String) = s.trimStart('0').ifEmpty { "0" }
            val x = clean(a)
            val y = clean(b)
            return if (x.length != y.length) x.length.compareTo(y.length) else x.compareTo(y)
        }
    }
}

internal fun homeTimelinePath(
    timeline: String,
    actorId: String?,
    size: Int = 50,
    before: String = "",
): String =
    "/api/home?timeline=" +
        (if (actorId != null) "social&author=${actorId.encodeURLParameter()}"
        else timeline.encodeURLParameter()) +
        (if (size != 50) "&limit=$size" else "") +
        (if (before.isNotEmpty()) "&before=${before.encodeURLParameter()}" else "")
