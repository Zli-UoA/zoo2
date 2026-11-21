package zoo

import androidx.compose.runtime.*
import io.ktor.http.encodeURLParameter
import kotlinx.coroutines.*

/**
 * Server-side search (`GET /api/search`, docs/search.md) for the `/search` screen.
 *
 * One request at a time: a newer query or kind waits for the running request and its
 * stale result is dropped. Older pages are only read on request (`loadOlder`), because the
 * server scans at most 12 months per request and may return fewer items with a cursor.
 * Held by the Session so returning from an opened result keeps the loaded pages.
 */
class GlobalSearchController(private val app: Session, private val scope: CoroutineScope = app.scope) {
    var query by mutableStateOf("")
        private set

    var type by mutableStateOf(GlobalSearchType.Messages)
        private set

    var items by mutableStateOf<List<Row>>(emptyList())
        private set

    var nextCursor by mutableStateOf<String?>(null)
        private set

    var searchedUntil by mutableStateOf<String?>(null)
        private set

    /** The first page of the current query is pending. */
    var loading by mutableStateOf(false)
        private set

    var loadingOlder by mutableStateOf(false)
        private set

    var error by mutableStateOf(false)
        private set

    /** The current query has a first page (possibly empty). */
    var loaded by mutableStateOf(false)
        private set

    private data class Request(val generation: Int, val query: String, val type: GlobalSearchType, val cursor: String?)

    private var generation = 0
    private var pending: Request? = null
    private var failed: Request? = null
    private var worker: Job? = null

    fun search(text: String, kind: GlobalSearchType = type) {
        val next = text.trim()
        if (next == query && kind == type) return
        query = next
        type = kind
        generation++
        items = emptyList()
        nextCursor = null
        searchedUntil = null
        error = false
        failed = null
        loaded = false
        loadingOlder = false
        if (next.isEmpty()) {
            pending = null
            loading = false
            return
        }
        loading = true
        enqueue(Request(generation, next, kind, null))
    }

    fun loadOlder() {
        val cursor = nextCursor ?: return
        if (loading || loadingOlder || error || pending != null) return
        loadingOlder = true
        enqueue(Request(generation, query, type, cursor))
    }

    fun retry() {
        val request = failed ?: return
        if (request.generation != generation) return
        failed = null
        error = false
        if (request.cursor == null) loading = true else loadingOlder = true
        enqueue(request)
    }

    private fun enqueue(request: Request) {
        pending = request
        if (worker?.isActive == true) return
        worker =
            scope.launch {
                while (true) {
                    val next = pending ?: break
                    pending = null
                    fetch(next)
                }
            }
    }

    private suspend fun fetch(request: Request) {
        try {
            val cursor = request.cursor?.let { "&cursor=${it.encodeURLParameter()}" }.orEmpty()
            val response =
                app.api
                    .request(
                        "/api/search?type=${request.type.value}&limit=$PAGE_SIZE" +
                            "&q=${request.query.encodeURLParameter()}$cursor"
                    )
                    .objectOrEmpty()
            if (request.generation != generation) return
            val incoming = response["items"]?.rows().orEmpty()
            items =
                if (request.cursor == null) incoming
                else items + incoming.filterNot { row -> items.any { it.text("id") == row.text("id") } }
            nextCursor = response.text("next_cursor").ifEmpty { null }
            searchedUntil = response.text("searched_until").ifEmpty { null }
            loaded = true
        } catch (e: CancellationException) {
            throw e
        } catch (_: Exception) {
            if (request.generation == generation) {
                failed = request
                error = true
            }
        } finally {
            if (request.generation == generation) {
                if (request.cursor == null) loading = false else loadingOlder = false
            }
        }
    }

    companion object {
        const val PAGE_SIZE = 20
    }
}

enum class GlobalSearchType(val value: String, val labelKey: String) {
    Messages("messages", "globalSearch.kinds.messages"),
    Files("files", "globalSearch.kinds.files");

    companion object {
        fun from(value: String?): GlobalSearchType = entries.find { it.value == value } ?: Messages
    }
}

/** `/search?q=&type=` for the current query; the screen replaces the history entry. */
fun globalSearchPath(query: String, type: GlobalSearchType): String =
    "/search?" +
        (if (query.isEmpty()) "" else "q=${query.encodeURLParameter()}&") +
        "type=${type.value}"

/** Where a result opens: the channel at its message (Chat opens the thread for replies). */
fun globalSearchDestination(row: Row, type: GlobalSearchType): String {
    val message = if (type == GlobalSearchType.Files) row.text("message_id") else row.text("id")
    return "/channels/${row.text("channel_id").encodeURLParameter()}?message=${message.encodeURLParameter()}"
}

internal fun isGlobalSearchPath(path: String): Boolean = path.substringBefore('?').substringBefore('#') == "/search"
