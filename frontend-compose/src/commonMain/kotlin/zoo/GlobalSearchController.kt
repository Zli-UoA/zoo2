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

    var type by mutableStateOf(GlobalSearchType.All)
        private set

    var filters by mutableStateOf(GlobalSearchFilters())
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

    private data class Request(
        val generation: Int,
        val query: String,
        val type: GlobalSearchType,
        val filters: GlobalSearchFilters,
        val cursor: String?,
    )

    private var generation = 0
    private var pending: Request? = null
    private var failed: Request? = null
    private var worker: Job? = null

    /** When the current first page arrived (epoch ms); `null` until then. */
    private var fetchedAt: Long? = null

    fun search(text: String, kind: GlobalSearchType = type, filter: GlobalSearchFilters = filters) {
        val next = text.trim()
        val normalized = filter.normalized()
        if (next == query && kind == type && normalized == filters) return
        start(next, kind, normalized)
    }

    /**
     * Follows the words, kind and filters of a `/search` route (`globalSearchRoute`). Returns the words for
     * the field when the route changed them (a link or a restored route); our own `replace` already matches.
     */
    fun followRoute(route: GlobalSearchRoute): String? {
        if (route.query.trim() == query && route.type == type && route.filters.normalized() == filters) return null
        search(route.query, route.type, route.filters)
        return route.query
    }

    /** Searches the same words again from the newest month (the IME search key, or a stale reopen). */
    fun refresh() {
        if (query.isNotEmpty()) start(query, type, filters)
    }

    /** On reopening the screen: reuse the loaded pages for [REUSE_MS] (as Web), then search again. */
    fun refreshIfStale(now: Long = nowMillis()) {
        val at = fetchedAt ?: return
        if (!loading && !loadingOlder && now - at >= REUSE_MS) refresh()
    }

    private fun start(next: String, kind: GlobalSearchType, filter: GlobalSearchFilters) {
        query = next
        type = kind
        filters = filter
        generation++
        items = emptyList()
        nextCursor = null
        searchedUntil = null
        error = false
        failed = null
        loaded = false
        loadingOlder = false
        fetchedAt = null
        // The server rejects these with 400 and a retry cannot pass, so the screen explains them instead.
        if (next.isEmpty() || searchQueryProblem(next) != null) {
            pending = null
            loading = false
            return
        }
        loading = true
        enqueue(Request(generation, next, kind, filter, null))
    }

    fun loadOlder() {
        val cursor = nextCursor ?: return
        if (loading || loadingOlder || error || pending != null) return
        loadingOlder = true
        enqueue(Request(generation, query, type, filters, cursor))
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
            val response =
                app.api.request(globalSearchRequestPath(request.query, request.type, request.filters, request.cursor))
                    .objectOrEmpty()
            if (request.generation != generation) return
            val incoming = response["items"]?.rows().orEmpty()
            items =
                if (request.cursor == null) incoming
                else {
                    val seen = items.map { globalSearchKey(it, request.type) }.toSet()
                    items + incoming.filterNot { globalSearchKey(it, request.type) in seen }
                }
            nextCursor = response.text("next_cursor").ifEmpty { null }
            searchedUntil = response.text("searched_until").ifEmpty { null }
            if (request.cursor == null) fetchedAt = nowMillis()
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

        /** Web's `REUSE_MS`: returning within this keeps the results, later ones search again. */
        const val REUSE_MS = 5 * 60_000L

        private fun nowMillis() = kotlin.time.Clock.System.now().toEpochMilliseconds()
    }
}

/** `All` (the default) searches messages and files together. */
enum class GlobalSearchType(val value: String, val labelKey: String) {
    All("all", "globalSearch.kinds.all"),
    Messages("messages", "globalSearch.kinds.messages"),
    Files("files", "globalSearch.kinds.files");

    companion object {
        fun from(value: String?): GlobalSearchType = entries.find { it.value == value } ?: All
    }
}

/** Senders and channels in one filter, like the server's `MAX_FILTER_IDS` (more is a 400). */
const val MAX_SEARCH_FILTER_IDS = 20

/**
 * Web's `SearchFilters` without the period: results from any of [from] (user ids) and any of [channels],
 * [noBots] leaves out bot (app) posts, [mine] keeps the conversations the user takes part in (their
 * channels and DMs, the threads they joined or were mentioned in).
 */
data class GlobalSearchFilters(
    val from: List<String> = emptyList(),
    val channels: List<String> = emptyList(),
    val mine: Boolean = false,
    val noBots: Boolean = false,
) {
    val active: Boolean get() = from.isNotEmpty() || channels.isNotEmpty() || mine || noBots

    /** Without blanks and repeats, at most [MAX_SEARCH_FILTER_IDS] each (Web's `splitIds`). */
    fun normalized(): GlobalSearchFilters = copy(from = from.cleanIds(), channels = channels.cleanIds())
}

private fun List<String>.cleanIds(): List<String> =
    map { it.trim() }.filter { it.isNotEmpty() }.distinct().take(MAX_SEARCH_FILTER_IDS)

/** `GET /api/search` for one page; the ids are comma-separated and match any (Web's `searchRequestPath`). */
fun globalSearchRequestPath(query: String, type: GlobalSearchType, filters: GlobalSearchFilters, cursor: String?): String =
    buildString {
        append("/api/search?type=${type.value}&limit=${GlobalSearchController.PAGE_SIZE}&q=${query.encodeURLParameter()}")
        if (filters.from.isNotEmpty()) append("&user_id=${filters.from.joinToString(",").encodeURLParameter()}")
        if (filters.channels.isNotEmpty()) append("&channel_id=${filters.channels.joinToString(",").encodeURLParameter()}")
        if (filters.noBots) append("&exclude_bots=true")
        if (filters.mine) append("&involving_me=true")
        if (cursor != null) append("&cursor=${cursor.encodeURLParameter()}")
    }

/**
 * `/search?q=&type=&from=&in=&mine=&nobots=` for the current query (the same names as Web's route); the
 * screen replaces the history entry. Filters that are not set are left out.
 */
fun globalSearchPath(query: String, type: GlobalSearchType, filters: GlobalSearchFilters = GlobalSearchFilters()): String =
    buildString {
        append("/search?")
        if (query.isNotEmpty()) append("q=${query.encodeURLParameter()}&")
        append("type=${type.value}")
        if (filters.from.isNotEmpty()) append("&from=${filters.from.joinToString(",").encodeURLParameter()}")
        if (filters.channels.isNotEmpty()) append("&in=${filters.channels.joinToString(",").encodeURLParameter()}")
        if (filters.mine) append("&mine=1")
        if (filters.noBots) append("&nobots=1")
    }

data class GlobalSearchRoute(val query: String, val type: GlobalSearchType, val filters: GlobalSearchFilters = GlobalSearchFilters())

/** Where a result opens: the channel at its message (Chat opens the thread for replies). */
fun globalSearchDestination(row: Row, type: GlobalSearchType): String {
    val message = if (isGlobalSearchFile(row, type)) row.text("message_id") else row.text("id")
    return "/channels/${row.text("channel_id").encodeURLParameter()}?message=${message.encodeURLParameter()}"
}

/** The words, kind and filters in a `/search` path ([globalSearchPath]), or `null` for another route. */
fun globalSearchRoute(path: String): GlobalSearchRoute? {
    if (!isGlobalSearchPath(path)) return null
    val parameters = io.ktor.http.Url("https://zoo.invalid$path").parameters
    fun ids(name: String) = parameters[name].orEmpty().split(',')
    fun flag(name: String) = parameters[name] == "1"
    return GlobalSearchRoute(
        parameters["q"].orEmpty(),
        GlobalSearchType.from(parameters["type"]),
        GlobalSearchFilters(ids("from"), ids("in"), flag("mine"), flag("nobots")).normalized(),
    )
}

internal fun isGlobalSearchPath(path: String): Boolean = path.substringBefore('?').substringBefore('#') == "/search"

/** A file result: `type=files`, or an `all` result the server marked `kind: "file"`. */
fun isGlobalSearchFile(row: Row, type: GlobalSearchType): Boolean =
    type == GlobalSearchType.Files || row.text("kind") == "file"

/** A list key unique across kinds: in `all`, a message and an attachment could share an id. */
fun globalSearchKey(row: Row, type: GlobalSearchType): String =
    (if (isGlobalSearchFile(row, type)) "file:" else "message:") + row.text("id")
