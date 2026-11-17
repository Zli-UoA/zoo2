package zoo

import io.ktor.client.HttpClient
import io.ktor.client.engine.mock.*
import io.ktor.http.HttpStatusCode
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.test.*
import kotlin.test.*
import androidx.compose.runtime.snapshotFlow
import androidx.compose.runtime.snapshots.Snapshot
import kotlinx.coroutines.launch
import kotlinx.coroutines.flow.collect
import kotlinx.serialization.json.JsonArray

@OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)
class HomeRefreshStateTest {
    @Test
    fun nearEndPagingResumesAfterAnIdenticalRetainedRefresh() = runTest {
        val gate = CompletableDeferred<Unit>()
        var requests = 0
        val head = JsonArray((50 downTo 1).map { obj("id" to "$it", "sort_key" to "$it") }).toString()
        val http = HttpClient(MockEngine) {
            engine {
                dispatcher = StandardTestDispatcher(testScheduler)
                addHandler { request ->
                    requests++
                    if (request.url.parameters["before"] != null) respond("[{\"id\":\"older\",\"sort_key\":\"0\"}]")
                    else {
                        if (requests > 1) gate.await()
                        respond(head)
                    }
                }
            }
        }
        val app = Session(Repository(Config("https://zoo.test", "/shapes", "alice", false),
            TestPlatform(), http, backgroundScope, synchronize = false), backgroundScope, Environment())
        val feed = HomeTimeline(app, backgroundScope)
        try {
            feed.refresh()
            runCurrent()
            feed.refresh(retain = true)
            runCurrent()
            assertTrue(feed.updating)
            assertFalse(feed.refreshing)
            val paging = backgroundScope.launch {
                snapshotFlow { feed.canLoadOlder }.collect { ready -> if (ready) feed.older() }
            }
            runCurrent()
            assertEquals(2, requests)
            gate.complete(Unit)
            runCurrent()
            Snapshot.sendApplyNotifications()
            runCurrent()
            assertFalse(feed.updating)
            assertEquals(3, requests)
            assertEquals("older", feed.posts.last().text("id"))
            paging.cancel()
        } finally { feed.close(); app.close(); http.close() }
    }

    private class TestPlatform : Platform {
        override suspend fun headers() = emptyMap<String, String>()
        override suspend fun readCache(key: String): String? = null
        override suspend fun writeCache(key: String, value: String) {}
        override fun uuid() = "refresh-test"
        override fun now() = "2026-11-15T00:00:00Z"
        override fun navigate(path: String) {}
        override fun action(name: String) {}
        override fun translate(key: String, args: Map<String, String>) = key
        override fun time(timestamp: String) = timestamp
        override fun day(timestamp: String) = timestamp
    }

    @Test
    fun navigatingBetweenDestinationsRetainsTheDraftOnlyInItsOwningSession() = runTest {
        fun session(user: String): Session {
            val http = HttpClient(MockEngine { error("Draft navigation must not fetch data") })
            return Session(
                Repository(Config("https://zoo.test", "/api/shapes", user, false), TestPlatform(), http, backgroundScope, synchronize = false),
                backgroundScope, Environment(),
            )
        }
        val first = session("alice")
        val second = session("bob")
        try {
            val draft = first.homeComposerState
            draft.content = "書きかけの投稿"
            first.update(first.env.copy(path = "/settings"))
            first.update(first.env.copy(path = "/home"))
            assertSame(draft, first.homeComposerState)
            assertEquals("書きかけの投稿", first.homeComposerState.content)
            assertEquals("", second.homeComposerState.content)
        } finally { first.close(); second.close() }
    }

    @Test
    fun manualRefreshShowsProgressAndLiveInvalidationWaitsWithoutAddingParallelRequests() = runTest {
        var gate = CompletableDeferred<Unit>()
        var requests = 0
        var fail = false
        val http = HttpClient(MockEngine) {
            engine {
                dispatcher = StandardTestDispatcher(testScheduler)
                addHandler {
                    requests++
                    gate.await()
                    if (fail) respond("{}", HttpStatusCode.ServiceUnavailable)
                    else respond("[{\"id\":\"one\",\"sort_key\":\"1\"}]")
                }
            }
        }
        val app = Session(
            Repository(Config("https://zoo.test", "/api/shapes", "alice", false), TestPlatform(), http, backgroundScope, synchronize = false),
            backgroundScope,
            Environment(),
        )
        val feed = HomeTimeline(app, backgroundScope)
        try {
            feed.refresh()
            assertTrue(feed.refreshing)
            runCurrent()
            assertEquals(1, requests)
            feed.refresh(retain = true)
            runCurrent()
            assertEquals(1, requests, "Live invalidation must wait for the manual refresh")
            gate.complete(Unit)
            runCurrent()
            assertFalse(feed.refreshing)
            assertFalse(feed.loading)
            assertEquals(2, requests, "One coalesced live refresh follows the completed request")
            assertEquals("one", feed.posts.single().text("id"))

            gate = CompletableDeferred()
            fail = true
            feed.refresh()
            runCurrent()
            assertTrue(feed.refreshing)
            gate.complete(Unit)
            runCurrent()
            assertFalse(feed.refreshing)
            assertTrue(feed.error)
            assertEquals("one", feed.posts.single().text("id"), "A failed refresh retains readable content")
        } finally {
            feed.close()
            app.close()
        }
    }
    @Test
    fun retainedRefreshChecksOnlyTheHeadAndVisibleHistoryAndNavigationRestoresTheWindow() = runTest {
        val calls = mutableListOf<String>()
        var checked = false
        fun page(from: Int) = JsonArray((from downTo from - 49).map { obj("id" to "$it", "sort_key" to "$it") }).toString()
        val http = HttpClient(MockEngine) {
            engine {
                dispatcher = StandardTestDispatcher(testScheduler)
                addHandler { request ->
                    calls += request.url.toString()
                    if (request.url.encodedPath == "/api/home/entries") {
                        checked = true
                        respond("[{\"id\":\"400\",\"sort_key\":\"400\",\"content\":\"edited\"}]")
                    } else respond(page(request.url.parameters["before"]?.toInt()?.minus(1) ?: 500))
                }
            }
        }
        val app = Session(Repository(Config("https://zoo.test", "/shapes", "alice", false), TestPlatform(), http, backgroundScope, synchronize = false), backgroundScope, Environment())
        val feed = HomeTimeline(app, backgroundScope)
        try {
            feed.refresh(); runCurrent()
            repeat(3) { feed.older(); runCurrent() }
            assertEquals(200, feed.posts.size)
            feed.viewport(listOf("400", "399"), top = false)
            val before = calls.size
            feed.refresh(true); runCurrent()
            assertEquals(2, calls.size - before, "History length must not increase refresh requests")
            assertTrue(checked)
            assertEquals(199, feed.posts.size, "An authoritative missing viewport row is removed")
            assertEquals("edited", feed.posts.first { it.text("id") == "400" }.text("content"))
            val checkedBefore = calls.size
            feed.viewport(listOf("400"), top = false)
            feed.refresh(true, includeHead = false); runCurrent()
            assertEquals(1, calls.size - checkedBefore, "Moving within history must not refetch the head")
            assertTrue(calls.last().endsWith("/api/home/entries"))
            feed.close(101, 17)
            val restored = HomeTimeline(app, backgroundScope)
            assertEquals(101, restored.restore()?.index)
            assertEquals(199, restored.posts.size)
            assertFalse(restored.needsRefresh())
            assertTrue(restored.hasMore)
            restored.close()
        } finally { feed.close(); app.close(); http.close() }
    }

    @Test
    fun anOlderPageFailureStopsAutomaticPagingUntilAnExplicitRetry() = runTest {
        var fail = true
        val http = HttpClient(MockEngine) {
            engine {
                dispatcher = StandardTestDispatcher(testScheduler)
                addHandler { request ->
                    if (request.url.parameters["before"] != null) {
                        if (fail) respond("{}", HttpStatusCode.ServiceUnavailable) else respond("[]")
                    } else respond(JsonArray((50 downTo 1).map { obj("id" to "$it") }).toString())
                }
            }
        }
        val app = Session(Repository(Config("https://zoo.test", "/shapes", "alice", false), TestPlatform(), http, backgroundScope, synchronize = false), backgroundScope, Environment())
        val feed = HomeTimeline(app, backgroundScope)
        try {
            feed.refresh(); runCurrent(); feed.older(); runCurrent()
            assertTrue(feed.olderError)
            assertFalse(feed.canLoadOlder)
            fail = false
            feed.older(); runCurrent()
            assertFalse(feed.olderError)
            assertFalse(feed.hasMore)
        } finally { feed.close(); app.close(); http.close() }
    }

    @Test fun returningToTheTopFetchesAPreviouslyUnappliedHead() = runTest {
        var fresh = false
        fun page(from:Int) = JsonArray((from downTo from-49).map { obj("id" to "$it", "sort_key" to "$it") }).toString()
        val http = HttpClient(MockEngine) { engine { dispatcher=StandardTestDispatcher(testScheduler); addHandler { request ->
            if(request.url.encodedPath=="/api/home/entries") respond(JsonArray(listOf(obj("id" to "200", "sort_key" to "200"))).toString())
            else respond(page(if(fresh) 500 else 200))
        } } }
        val app = Session(Repository(Config("https://zoo.test","/shapes","alice",false),TestPlatform(),http,backgroundScope,synchronize=false),backgroundScope,Environment())
        val feed = HomeTimeline(app,backgroundScope)
        try {
            feed.refresh(); runCurrent()
            feed.viewport(listOf("200"),false)
            fresh=true
            feed.refresh(true,true); runCurrent()
            assertFalse(feed.posts.any { it.text("id")=="500" })
            assertTrue(feed.needsRefresh())
            assertTrue(feed.viewport(listOf("200"),true))
            feed.refresh(true,false); runCurrent()
            assertEquals("500",feed.posts.first().text("id"))
        } finally { feed.close(); app.close(); http.close() }
    }

    @Test
    fun knownPostIsVisibleBeforeTheOnlyThreadRequestCompletes() = runTest {
        val gate = CompletableDeferred<Unit>()
        var requests = 0
        val http = HttpClient(MockEngine) { engine {
            dispatcher = StandardTestDispatcher(testScheduler)
            addHandler { request ->
                assertEquals("/api/home/post/thread", request.url.encodedPath)
                requests++
                gate.await()
                respond("[{\"id\":\"post\",\"content\":\"updated\"},{\"id\":\"reply\",\"parent_id\":\"post\"}]")
            }
        } }
        val app = Session(Repository(Config("https://zoo.test", "/shapes", "alice", false), TestPlatform(), http, backgroundScope, synchronize = false), backgroundScope, Environment())
        app.homeTimelines.save("home\u0000post\u0000", HomeTimelineSnapshot(listOf(obj("id" to "post", "content" to "stale")), false, 3, 20, Long.MAX_VALUE))
        app.homeTimelines.preparePost(obj("id" to "post", "content" to "known"))
        val feed = HomeTimeline(app, backgroundScope, postId = "post")
        try {
            assertEquals("known", feed.posts.single().text("content"))
            assertFalse(feed.loading)
            assertNull(feed.restore(), "The clicked DTO takes priority over an older thread snapshot")
            assertTrue(feed.needsRefresh())
            assertEquals(0, requests)
            feed.refresh(retain = true)
            runCurrent()
            assertEquals("known", feed.posts.single().text("content"))
            assertEquals(1, requests)
            assertFalse(feed.viewport(listOf("post"), true), "Rendering the seed must not queue a second thread request")
            gate.complete(Unit)
            runCurrent()
            assertEquals(listOf("post", "reply"), feed.posts.map { it.text("id") })
            assertEquals("updated", feed.posts.first().text("content"))
            assertEquals(1, requests)
        } finally { feed.close(); app.close(); http.close() }
    }

    @Test
    fun threadFailuresRetainKnownPostsExceptWhenAccessIsDeniedOrThePostIsGone() = runTest {
        for (status in listOf(500, 403, 404)) {
            val http = HttpClient(MockEngine { respond("{}", HttpStatusCode.fromValue(status)) })
            val app = Session(Repository(Config("https://zoo.test", "/shapes", "alice", false), TestPlatform(), http, backgroundScope, synchronize = false), backgroundScope, Environment())
            app.homeTimelines.preparePost(obj("id" to "post", "content" to "known"))
            val feed = HomeTimeline(app, backgroundScope, postId = "post")
            try {
                feed.refresh(retain = true)
                feed.awaitUpdate()
                assertTrue(feed.error)
                assertFalse(feed.loading)
                if (status == 500) assertEquals("known", feed.posts.single().text("content"))
                else assertTrue(feed.posts.isEmpty())
            } finally { feed.close(); app.close(); http.close() }
        }
    }
}
