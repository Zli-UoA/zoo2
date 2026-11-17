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

    @Test fun passesKnownPostOnceWithoutRepostDecorationOrCrossingSessions() {
        val cache = HomeTimelineCache()
        val post = obj("id" to "remote/post", "entry_id" to "repost", "content" to "known",
            "tree_depth" to 4, "reposted_by" to "Bob", "attachments" to kotlinx.serialization.json.JsonArray(emptyList()))
        cache.preparePost(post)
        assertNull(HomeTimelineCache().takePost("remote/post"))
        val detail = assertNotNull(cache.takePost("remote/post"))
        assertEquals("remote/post", HomeTimeline.key(detail))
        assertEquals("known", detail.text("content"))
        assertEquals("0", detail.text("tree_depth"))
        assertFalse(detail.present("reposted_by"))
        assertEquals(post["attachments"], detail["attachments"])
        assertNull(cache.takePost("remote/post"))
        cache.preparePost(post)
        cache.remove("remote/post")
        assertNull(cache.takePost("remote/post"))
        cache.preparePost(post)
        cache.clear()
        assertNull(cache.takePost("remote/post"))
    }
}
