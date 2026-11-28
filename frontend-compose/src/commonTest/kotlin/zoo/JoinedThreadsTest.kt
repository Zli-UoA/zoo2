package zoo

import kotlin.test.*
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.*

class JoinedThreadsTest {
    private fun thread(id: String, channel: String = "c1", replies: Int = 1, archived: Boolean = false) = buildJsonObject {
        put("id", id); put("channel_id", channel); put("content", "root $id"); put("reply_count", replies)
        put("thread_archive_at", if (archived) JsonPrimitive("2026-11-01T00:00:00Z") else JsonNull)
    }

    private fun page(vararg items: JsonObject, next: String? = null) = buildJsonObject {
        put("items", JsonArray(items.toList()))
        put("next_cursor", next?.let(::JsonPrimitive) ?: JsonNull)
    }

    @Test fun keepsThreadsWithRepliesAndGroupsThemByChannel() = runTest {
        val threads = JoinedThreads {
            page(thread("t1"), thread("t2", replies = 0), thread("t3", archived = true), thread("t4", channel = "c2"))
        }
        threads.refresh()
        assertEquals(mapOf("c1" to listOf("t1"), "c2" to listOf("t4")),
            threads.byChannel().mapValues { (_, rows) -> rows.map { it.id } })
        assertFalse(threads.hasMore)
    }

    @Test fun loadMoreAppendsAndLaterRefreshesKeepTheExpandedRange() = runTest {
        val requests = mutableListOf<String>()
        val threads = JoinedThreads { path ->
            requests += path
            when (path) {
                "/api/threads" -> page(thread("t1"), next = "a")
                "/api/threads?before=a" -> page(thread("t1", replies = 2), thread("t2"), next = "b")
                "/api/threads?through=b" -> page(thread("t3"), thread("t1"), next = "a")
                "/api/threads?before=a&through=b" -> page(thread("t2"))
                else -> error(path)
            }
        }
        threads.refresh()
        assertTrue(threads.hasMore)
        threads.loadMore()
        assertEquals(listOf("t1", "t2"), threads.threads.map { it.id }, "the newer copy of t1 replaces the old one")
        assertEquals(2, threads.threads.first { it.id == "t1" }.replies)
        threads.refresh()
        assertEquals(listOf("t3", "t1", "t2"), threads.threads.map { it.id })
        assertEquals(listOf("/api/threads", "/api/threads?before=a", "/api/threads?through=b", "/api/threads?before=a&through=b"), requests)
        assertTrue(threads.hasMore)
    }

    @Test fun retryRepeatsTheFailedLoadMore() = runTest {
        val requests = mutableListOf<String>()
        var fail = false
        val threads = JoinedThreads { path ->
            requests += path
            if (fail && path.contains("before")) throw IllegalStateException("offline")
            if (path.contains("before")) page(thread("t2")) else page(thread("t1"), next = "a")
        }
        threads.refresh()
        fail = true
        threads.loadMore()
        assertTrue(threads.error)
        fail = false
        threads.retry()
        assertFalse(threads.error)
        assertEquals(listOf("t1", "t2"), threads.threads.map { it.id })
        assertEquals(listOf("/api/threads", "/api/threads?before=a", "/api/threads?before=a"), requests)
    }

    @Test fun aFailedRefreshKeepsTheList() = runTest {
        var fail = false
        val threads = JoinedThreads { if (fail) throw IllegalStateException("offline") else page(thread("t1")) }
        threads.refresh()
        fail = true
        threads.refresh()
        assertEquals(listOf("t1"), threads.threads.map { it.id })
        assertTrue(threads.error)
    }
}
