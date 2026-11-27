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
    @Test fun removesAnUnavailableQuoteFromWrappedEntriesAndCachedSnapshots() {
        val invisible = obj("id" to "hidden", "content" to "old body", "media" to "old media")
        val embeddedQuote = obj("uri" to "https://social.test/hidden", "post" to invisible)
        val entry = obj("entry_id" to "repost", "post" to obj("id" to "visible", "content" to "comment",
            "parent" to obj("id" to "parent", "quote" to embeddedQuote), "quote" to embeddedQuote))
        val result = assertNotNull(entry.withoutUnavailableHomePost("hidden"))
        assertEquals("repost", result.text("entry_id"))
        assertEquals("comment", result.nested("post").text("content"))
        val quote = result.nested("post").nested("quote")
        assertEquals("https://social.test/hidden", quote.text("uri"))
        assertTrue(quote.nested("post").isEmpty())
        val parent = result.nested("post").nested("parent")
        assertEquals("parent", parent.text("id"))
        assertTrue(parent.nested("quote").nested("post").isEmpty())
        assertNull(invisible.withoutUnavailableHomePost("hidden"))
        val cache = HomeTimelineCache()
        cache.save("home", HomeTimelineSnapshot(listOf(entry), false, 0, 0, 1))
        cache.remove("hidden")
        assertNull(cache.get("home"))
    }
}
