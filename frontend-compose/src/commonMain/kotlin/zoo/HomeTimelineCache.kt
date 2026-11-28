package zoo

import kotlinx.serialization.json.*
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

/** Account-owned navigation cache. Bounded like the Web timeline cache; never shared across sessions. */
internal data class HomeTimelineSnapshot(
    val posts: List<Row>, val hasMore: Boolean, val index: Int, val offset: Int, val checkedAt: Long,
)
internal class HomeTimelineCache {
    private val entries = linkedMapOf<String, HomeTimelineSnapshot>()
    var limit: Int = 8
        set(value) {
            field = value
            trim()
        }

    private fun trim() {
        while (entries.size > limit) entries.remove(entries.keys.first())
    }

    fun get(key: String): HomeTimelineSnapshot? = entries.remove(key)?.also { entries[key] = it }
    fun save(key: String, value: HomeTimelineSnapshot) {
        entries.remove(key)
        if (limit == 0 || value.posts.size > 1000 || value.posts.sumOf { it.toString().length.toLong() * 2 } > 4 * 1024 * 1024) return
        entries[key] = value
        trim()
    }
    fun preparePost(post: Row) {
        val id = post.text("id")
        if (id.isEmpty()) return
        val detail = JsonObject(post + obj("entry_id" to id, "tree_depth" to 0, "reposted_by" to null))
        save("post:$id", HomeTimelineSnapshot(listOf(detail), false, 0, 0, 0))
    }
    fun takePost(postId: String): Row? = entries.remove("post:$postId")?.posts?.singleOrNull()
    fun remove(postId: String) {
        entries.entries.removeAll { (_, snapshot) ->
            snapshot.posts.any { it.withoutUnavailableHomePost(postId) !== it }
        }
    }
    fun removeEntry(key: String) { entries.remove(key) }

    fun clear() = entries.clear()
}

/** Remove a post from a feed entry, including copies embedded as a parent or quote. */
fun Row.withoutUnavailableHomePost(id: String): Row? {
    val wrapped = nested("post")
    val post = wrapped.ifEmpty { this }
    if (post.text("id") == id) return null
    val quote = post.nested("quote")
    val parent = post.nested("parent")
    val quotedPost = quote.nested("post")
    val availableParent = parent.withoutUnavailableEmbeddedPost(id)
    val availableQuote = quotedPost.withoutUnavailableEmbeddedPost(id)
    var filtered = post
    if (availableParent !== parent) {
        filtered = JsonObject(if (availableParent == null) filtered - "parent" else filtered + ("parent" to availableParent))
    }
    if (availableQuote !== quotedPost) {
        val remainingQuote = JsonObject(if (availableQuote == null) quote - "post" else quote + ("post" to availableQuote))
        filtered = JsonObject(filtered + ("quote" to remainingQuote))
    }
    if (filtered === post) return this
    return if (wrapped.isEmpty()) filtered else JsonObject(this + ("post" to filtered))
}

private fun Row.withoutUnavailableEmbeddedPost(id: String): Row? =
    if (isEmpty()) this else withoutUnavailableHomePost(id)

internal fun mergeHomeWindow(current: List<Row>, head: List<Row>, checkedIds: List<String>, checked: List<Row>, atTop: Boolean): List<Row> {
    val incoming = (head + checked).associateBy(HomeTimeline::key)
    val headIds = head.map(HomeTimeline::key).toSet()
    val lastOverlap = current.indexOfLast { HomeTimeline.key(it) in headIds }
    val replace = atTop || lastOverlap >= 0
    val history = if (replace) { if (lastOverlap >= 0) current.drop(lastOverlap + 1) else emptyList() } else current
    val known = checkedIds.toSet()
    val old = current.associateBy(HomeTimeline::key)
    return ((if (replace) head else emptyList()) + history).distinctBy(HomeTimeline::key).mapNotNull { row ->
        val key = HomeTimeline.key(row)
        if (key in known && key !in incoming) null
        else (incoming[key] ?: row).let { next -> old[key]?.takeIf { it == next } ?: next }
    }
}

/** AccountCache supplies account isolation and atomic writes. Keep one recent head for cold startup. */
internal class StartupTimeline(private val platform: Platform) {
    private val key = "home-startup-v1"
    private val lock = Mutex()
    private var revision = 0

    suspend fun read(timeline: String): List<Row> = lock.withLock {
        val raw = try { platform.readCache(key) } catch (e: CancellationException) { throw e }
            catch (_: Exception) { null }
        if (raw == null || raw.length > 96_000) return@withLock emptyList()
        val record = runCatching { json.parseToJsonElement(raw).objectOrEmpty() }.getOrNull() ?: return@withLock emptyList()
        val savedAt = record.text("time").toLongOrNull() ?: return@withLock emptyList()
        val age = kotlin.time.Clock.System.now().toEpochMilliseconds() - savedAt
        if (record.text("timeline") == timeline && age in 0..300_000) record.array("posts").rows().take(12)
        else emptyList()
    }

    fun invalidate(): Int = ++revision
    suspend fun replace(version: Int, timeline: String, head: List<Row>) = lock.withLock {
        if (version != revision) return@withLock
        // Only keep openly visible, non-sensitive standalone posts. Private embedded content
        // and URLs belonging to quotes or parent posts must never enter this startup cache.
        val rows = head.take(12).takeWhile { entry ->
            val post = entry.nested("post").ifEmpty { entry }
            post.text("visibility") == "public" && !post.flag("sensitive") &&
                !post.present("parent") && !post.present("quote") && !post.present("ciphertext") &&
                post.array("media").rows().none { it.flag("is_sensitive") }
        }
        val value = obj("timeline" to timeline, "time" to kotlin.time.Clock.System.now().toEpochMilliseconds(),
            "posts" to JsonArray(rows)).toString()
        try {
            if (rows.isEmpty() || value.length > 96_000) platform.removeCache(key) else platform.writeCache(key, value)
        } catch (e: CancellationException) { throw e }
        catch (_: Exception) { /* Rendering the fetched head does not depend on disk space. */ }
    }
}
