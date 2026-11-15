package zoo

import io.ktor.client.HttpClient
import io.ktor.client.engine.mock.*
import io.ktor.http.HttpStatusCode
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.test.*
import kotlin.test.*

@OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)
class HomeRefreshStateTest {
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
}
