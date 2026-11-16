package zoo

import androidx.compose.runtime.*
import io.ktor.http.encodeURLParameter
import kotlinx.coroutines.*
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonPrimitive

/** Owns the retained paging window; live refreshes wait for an outstanding older page. */
@Stable
class HomeTimeline(
    private val app: Session,
    private val scope: CoroutineScope,
    private val postId: String? = null,
    private val actorId: String? = null,
) {
    private val settingsKey = homePreferenceKey(app, "zoo-home-timeline:v1:")
    var timeline by mutableStateOf("home")
        private set

    var posts by mutableStateOf<List<Row>>(emptyList())
        private set

    var loading by mutableStateOf(true)
        private set

    var refreshing by mutableStateOf(false)
        private set

    /** Includes retained live refreshes, even when the pull-to-refresh spinner stays hidden. */
    var updating by mutableStateOf(false)
        private set

    var loadingOlder by mutableStateOf(false)
        private set

    var hasMore by mutableStateOf(false)
        private set

    var error by mutableStateOf(false)
        private set

    val canLoadOlder: Boolean
        get() = hasMore && !loading && !loadingOlder && !updating && !error && !olderError

    private var refreshJob: Job? = null
    private var olderJob: Job? = null
    private var deferred = false
    private var deferredHead = false
    private var validatedIds = emptySet<String>()
    var viewportError by mutableStateOf(false)
        private set
    private var generation = 0
    private var visibleIds = emptyList<String>()
    private var atTop = true
    private var checkedAt = 0L
    var olderError by mutableStateOf(false)
        private set
    private val cacheKey get() = listOf(timeline, postId.orEmpty(), actorId.orEmpty()).joinToString("\u0000")
    fun viewport(ids: List<String>, top: Boolean): Boolean {
        val next = ids.take(100)
        val changed = visibleIds != next
        visibleIds = next
        atTop = top
        return changed && next.any { it !in validatedIds }
    }
    internal fun restore(): HomeTimelineSnapshot? = app.homeTimelines.get(cacheKey)?.also {
        posts = it.posts; hasMore = it.hasMore; loading = false; checkedAt = it.checkedAt
        validatedIds = it.posts.take(50).map(::key).toSet()
    }
    fun needsRefresh(): Boolean = posts.isEmpty() || kotlin.time.Clock.System.now().toEpochMilliseconds() - checkedAt >= 30_000
    private fun save(index: Int = 0, offset: Int = 0) {
        app.homeTimelines.save(cacheKey, HomeTimelineSnapshot(posts, hasMore, index, offset, checkedAt))
    }

    private var selectionChanged = false
    suspend fun restoreSelection() {
        val saved = readHomePreference(app, settingsKey)
        if (!selectionChanged) timeline = saved.takeIf { it in listOf("home", "local", "social") } ?: "home"
    }

    fun select(value: String) {
        if (timeline == value) return
        timeline = value
        selectionChanged = true
        scope.launch { writeHomePreference(app, settingsKey, value) }
        deferred = false
        generation++
        refreshJob?.cancel()
        olderJob?.cancel()
        posts = emptyList()
        loading = true
        loadingOlder = false
        olderError = false
        visibleIds = emptyList()
        atTop = true
        refresh()
    }

    fun refresh(retain: Boolean = false, includeHead: Boolean = true) {
        if (retain && (loadingOlder || refreshJob?.isActive == true)) {
            deferred = true
            deferredHead = deferredHead || includeHead
            return
        }
        deferred = false
        deferredHead = false
        refreshJob?.cancel()
        val version = ++generation
        if (!retain) {
            olderJob?.cancel()
            loadingOlder = false
        }
        if (!retain) olderError = false
        refreshing = !retain
        updating = true
        refreshJob =
            scope.launch {
                error = false
                try {
                    val fetchHead = includeHead || posts.isEmpty() || postId != null
                    val head = if (!fetchHead) emptyList() else if (postId != null)
                        app.api.request("/api/home/${postId.encodeURLParameter()}/thread").rows()
                    else app.api.request(path(50, "")).rows()
                    val headIds = head.map(::key).toSet()
                    val complete = fetchHead && postId == null && head.size < 50
                    val check = if (retain && postId == null && !complete) visibleIds.filterNot { it in headIds } else emptyList()
                    var checked = emptyList<Row>()
                    var checkedSuccessfully = true
                    if (check.isNotEmpty()) {
                        try {
                            checked = app.api.request("/api/home/entries", "POST", obj(
                                "entries" to JsonArray(check.map(::JsonPrimitive)), "timeline" to if (actorId == null) timeline else "social", "author" to actorId,
                            )).rows()
                        } catch (e: CancellationException) { throw e }
                        catch (_: Exception) { checkedSuccessfully = false }
                    }
                    if (version == generation) {
                        posts = if (retain && postId == null)
                            mergeHomeWindow(posts, head, if (complete) posts.map(::key) else if (checkedSuccessfully) check else emptyList(), checked, fetchHead && atTop || complete)
                        else head
                        if (fetchHead && (!retain || complete || posts.size <= 50)) hasMore = !complete && postId == null && head.size == 50
                        if (fetchHead) {
                            checkedAt = kotlin.time.Clock.System.now().toEpochMilliseconds()
                            validatedIds = headIds
                        }
                        if (checkedSuccessfully) validatedIds = validatedIds + check
                        viewportError = !checkedSuccessfully
                    }
                } catch (e: CancellationException) {
                    throw e
                } catch (_: Exception) {
                    if (version == generation) error = true
                } finally {
                    if (version == generation) {
                        loading = false
                        refreshing = false
                        refreshJob = null
                        updating = false
                        if (deferred && app.env.visible && app.env.online) refresh(true, deferredHead)
                    }
                }
            }
    }

    fun remove(postId: String) {
        app.homeTimelines.remove(postId)
        posts = posts.filterNot { it.text("id") == postId }
    }

    suspend fun awaitUpdate() { refreshJob?.join() }

    fun older() {
        if (loading || loadingOlder || updating || !hasMore || posts.isEmpty()) return
        val version = generation
        loadingOlder = true
        error = false
        olderJob =
            scope.launch {
                try {
                    val page = app.api.request(path(50, key(posts.last()))).rows()
                    if (version == generation) {
                        val known = posts.map(::key).toSet()
                        val added = page.filterNot { key(it) in known }
                        posts = posts + added
                        validatedIds = validatedIds + page.map(::key)
                        hasMore = page.size == 50 && added.isNotEmpty()
                        olderError = false
                    }
                } catch (e: CancellationException) {
                    throw e
                } catch (_: Exception) {
                    if (version == generation) olderError = true
                } finally {
                    if (version == generation) {
                        loadingOlder = false
                        if (deferred) {
                            refresh(true, deferredHead)
                        }
                    }
                }
            }
    }

    fun close(index: Int = 0, offset: Int = 0) {
        if (posts.isNotEmpty()) save(index, offset)
        generation++
        updating = false
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
