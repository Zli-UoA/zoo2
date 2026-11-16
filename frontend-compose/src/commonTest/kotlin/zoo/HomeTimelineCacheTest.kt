package zoo

import kotlin.test.*

class HomeTimelineCacheTest {
    @Test fun restoresReadingPositionAndEvictsOldDestinationsWithinItsLimits() {
        val cache = HomeTimelineCache()
        val post = obj("id" to "post")
        val snapshot = HomeTimelineSnapshot(listOf(post), true, 42, 17, 1)
        cache.save("home", snapshot)
        assertSame(snapshot, cache.get("home"))
        repeat(8) { cache.save("actor-$it", snapshot) }
        assertNull(cache.get("home"))
        cache.save("huge", snapshot.copy(posts = List(1001) { post }))
        assertNull(cache.get("huge"))
        cache.save("large", snapshot.copy(posts = listOf(obj("id" to "large", "content" to "x".repeat(2 * 1024 * 1024)))))
        assertNull(cache.get("large"))
        assertNull(HomeTimelineCache().get("actor-7"), "Another account/session cannot access the cache")
        cache.clear()
        assertNull(cache.get("actor-7"))
    }
}
