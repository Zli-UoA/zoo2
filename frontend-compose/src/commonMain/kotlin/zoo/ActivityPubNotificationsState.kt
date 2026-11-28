package zoo

import androidx.compose.runtime.*
import io.ktor.http.encodeURLParameter
import kotlinx.coroutines.*
import kotlinx.serialization.json.*

/** The notification feed is shared by the rail count and its screen. */
class ActivityPubNotificationsState(
    private val app: Session,
    private val subscribe: (() -> Unit) -> (() -> Unit) = { app.homeLive.subscribe(it) },
) {
    var items by mutableStateOf<List<Row>>(emptyList())
        private set

    var unreadCount by mutableStateOf(0)
        private set

    var loading by mutableStateOf(true)
        private set

    var loadingMore by mutableStateOf(false)
        private set

    var error by mutableStateOf(false)
        private set

    var appendError by mutableStateOf(false)
        private set

    var nextCursor by mutableStateOf<String?>(null)
        private set

    private var visibleIds = emptyList<String>()
    private data class Gap(val anchors: Set<String>, val resume: String?)
    private var gap: Gap? = null
    private var readRevision = 0
    fun viewport(ids: List<String>) {
        val next = ids.take(100)
        if (next != visibleIds && next.any { id -> items.indexOfFirst { it.text("id") == id } >= 100 }) updates.request()
        visibleIds = next
    }
    private var generation = 0
    private var started = false
    private var deferred = false
    private val pendingReads = mutableSetOf<String>()
    private var request: Job? = null
    private var unsubscribe: (() -> Unit)? = null
    private var closed = false
    private var queuedMore: Job? = null
    private val updates = CoalescedRefresh(app.scope, allowed = { !closed && app.env.visible && app.env.online }) { refresh().join() }

    fun ensureLoaded() {
        if (!started) refresh()
    }

    fun refresh(): Job {
        if (closed) return app.scope.launch {}
        request?.takeIf { it.isActive }?.let { deferred = true; return it }
        deferred = false
        started = true
        if (unsubscribe == null) unsubscribe = subscribe { invalidate() }
        val ticket = ++generation
        return app.scope.launch(start = CoroutineStart.LAZY) {
            do {
                deferred = false
                fetch(append = false, ticket = ticket)
            } while (deferred && !closed && app.env.visible && app.env.online)
        }.also { request = it; it.start() }
    }

    fun loadMore() {
        if (closed || nextCursor == null || loading || loadingMore) return
        val running = request?.takeIf { it.isActive }
        if (running != null) {
            if (queuedMore?.isActive != true) queuedMore = app.scope.launch {
                running.join()
                queuedMore = null
                loadMore()
            }
            return
        }
        val ticket = ++generation
        loadingMore = true
        app.scope.launch(start = CoroutineStart.LAZY) {
            fetch(append = true, ticket = ticket)
            if (deferred && !closed) { request = null; refresh() }
        }.also { request = it; it.start() }
    }

    private suspend fun fetch(append: Boolean, ticket: Int) {
        try {
            val readsAtStart = readRevision
            val suffix = if (append) nextCursor?.let { "?before=${it.encodeURLParameter()}" }.orEmpty() else ""
            val response = app.api.request("/api/activitypub/notifications$suffix").objectOrEmpty()
            if (ticket != generation) return
            val incoming = response["items"]?.rows().orEmpty()
            val headIds = incoming.map { it.text("id") }.toSet()
            val check = if (append) emptyList() else visibleIds.filterNot { it in headIds }
            val checked = if (check.isEmpty()) emptyList() else app.api.request(
                "/api/activitypub/notifications/entries", "POST", obj("entries" to JsonArray(check.map(::JsonPrimitive))),
            ).objectOrEmpty()["items"]?.rows().orEmpty()
            if (ticket != generation) return
            val updates = (incoming + checked).associateBy { it.text("id") }
            val retained = items
            val latest = retained.associateBy { it.text("id") }
            val cursor = response.text("next_cursor").takeIf { it.isNotEmpty() }
            items = (if (append) retained + incoming.filterNot { row -> retained.any { it.text("id") == row.text("id") } }
                else incoming + (if (cursor != null) retained.filterNot { it.text("id") in headIds } else emptyList()))
                .filter { it.text("id") !in check || it.text("id") in updates }
                .map { row -> (updates[row.text("id")] ?: row).let { updated ->
                    if (latest[row.text("id")]?.flag("is_read") == true) JsonObject(updated + ("is_read" to JsonPrimitive(true))) else updated
                } }
                .let { rows -> if (rows.any { it.text("created_at").isEmpty() }) rows else rows.sortedWith(compareByDescending<Row> { notificationTimestamp(it.text("created_at")) }.thenByDescending { it.text("id") }) }
                .map { if (it.text("id") in pendingReads) JsonObject(it + ("is_read" to JsonPrimitive(true))) else it }
            if (!append && readsAtStart == readRevision) unreadCount = ((response["unread_count"]?.jsonPrimitive?.intOrNull ?: 0) - incoming.count { !it.flag("is_read") && it.text("id") in pendingReads }).coerceAtLeast(0)
            if (append) {
                val missing = gap
                if (missing != null && incoming.any { it.text("id") in missing.anchors }) {
                    nextCursor = missing.resume
                    gap = null
                } else {
                    nextCursor = cursor
                    if (cursor == null) gap = null
                }
            } else if (cursor == null || retained.isEmpty()) {
                nextCursor = cursor
                gap = null
            } else if (retained.none { it.text("id") in headIds }) {
                if (gap == null) gap = Gap(retained.map { it.text("id") }.toSet(), nextCursor)
                nextCursor = cursor
            } else if (gap == null && nextCursor != null && retained.all { it.text("id") in headIds }) {
                nextCursor = cursor
            }
            if (append || nextCursor == null) appendError = false
            error = false
        } catch (e: CancellationException) {
            throw e
        } catch (_: Exception) {
            if (ticket == generation) {
                if (append) appendError = true else error = true
            }
        } finally {
            if (ticket == generation) {
                if (append) loadingMore = false else loading = false
            }
        }
    }

    suspend fun markRead(ids: List<String>? = null) {
        if (ids != null && ids.isEmpty()) return
        val selected = ids?.toSet() ?: items.filterNot { it.flag("is_read") }.map { it.text("id") }.toSet()
        val changed = items.filter { !it.flag("is_read") && it.text("id") in selected }.map { it.text("id") }.toSet()
        readRevision++
        pendingReads.addAll(selected)
        items = items.map { if (it.text("id") in selected) JsonObject(it + ("is_read" to JsonPrimitive(true))) else it }
        val previousCount = unreadCount
        unreadCount = if (ids == null) 0 else (unreadCount - changed.size).coerceAtLeast(0)
        try {
            app.api.request("/api/activitypub/notifications/read", "POST", if (ids == null) obj() else obj("ids" to JsonArray(selected.map(::JsonPrimitive))))
        } catch (e: Exception) {
            readRevision++
            items = items.map { if (it.text("id") in changed) JsonObject(it + ("is_read" to JsonPrimitive(false))) else it }
            unreadCount = previousCount
            throw e
        } finally { pendingReads.removeAll(selected) }
    }

    suspend fun act(path: String, body: Row) {
        app.api.request(path, "POST", body)
        refresh().join()
    }

    fun invalidate() {
        if (started && !closed) updates.request()
    }

    fun close() {
        closed = true
        updates.close()
        queuedMore?.cancel()
        generation++
        request?.cancel()
        unsubscribe?.invoke()
        unsubscribe = null
    }
}
