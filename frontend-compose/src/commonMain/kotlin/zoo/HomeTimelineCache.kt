package zoo

/** Account-owned navigation cache. Bounded like the Web timeline cache; never shared across sessions. */
internal data class HomeTimelineSnapshot(
    val posts: List<Row>, val hasMore: Boolean, val index: Int, val offset: Int, val checkedAt: Long,
)
internal class HomeTimelineCache {
    private val entries = linkedMapOf<String, HomeTimelineSnapshot>()
    fun get(key: String): HomeTimelineSnapshot? = entries.remove(key)?.also { entries[key] = it }
    fun save(key: String, value: HomeTimelineSnapshot) {
        entries.remove(key)
        if (value.posts.size > 1000 || value.posts.sumOf { it.toString().length.toLong() * 2 } > 4 * 1024 * 1024) return
        entries[key] = value
        while (entries.size > 8) entries.remove(entries.keys.first())
    }
    fun remove(postId: String) {
        entries.entries.removeAll { (_, snapshot) -> snapshot.posts.any {
            it.text("id") == postId || it.nested("parent").text("id") == postId || it.nested("quote").nested("post").text("id") == postId
        } }
    }
    fun clear() = entries.clear()
}

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
