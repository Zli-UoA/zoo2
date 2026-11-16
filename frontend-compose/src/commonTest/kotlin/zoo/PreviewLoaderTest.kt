package zoo

import io.ktor.client.HttpClient
import io.ktor.client.engine.mock.MockEngine
import kotlin.test.*
import kotlinx.coroutines.*
import kotlinx.coroutines.test.*

@OptIn(ExperimentalCoroutinesApi::class)
class PreviewLoaderTest {
    private class PublicPlatform(val fetch: suspend () -> String) : Platform {
        override suspend fun headers() = emptyMap<String, String>()
        override suspend fun readCache(key: String): String? = null
        override suspend fun writeCache(key: String, value: String) {}
        override fun uuid() = "preview"
        override fun now() = "2026-11-15T00:00:00Z"
        override fun navigate(path: String) {}
        override fun action(name: String) {}
        override fun translate(key: String, args: Map<String, String>) = key
        override fun time(timestamp: String) = timestamp
        override fun day(timestamp: String) = timestamp
        override suspend fun invoke(name: String, payload: String): String {
            assertEquals("publicGet", name)
            return fetch()
        }
    }

    @Test fun sharesRequestsAcrossCardsAndKeepsTheSurvivingConsumer() = runTest {
        val gate = CompletableDeferred<Unit>()
        var requests = 0
        val platform = PublicPlatform {
            requests++
            gate.await()
            obj("body" to obj("title" to "Preview", "description" to "shared").toString()).toString()
        }
        val http = HttpClient(MockEngine { error("Previews must use the public platform path") })
        val app = Session(Repository(Config("https://zoo.test", "/shapes", "alice", false),
            platform, http, backgroundScope, synchronize = false), backgroundScope, Environment())
        try {
            val first = async { app.previews.load("https://summaly.test/url", "https://target.test/") }
            val second = async { app.previews.load("https://summaly.test/url", "https://target.test/") }
            runCurrent()
            assertEquals(1, requests)
            first.cancelAndJoin()
            gate.complete(Unit)
            assertEquals("Preview", second.await()?.title)
            assertEquals("Preview", app.previews.load("https://summaly.test/url", "https://target.test/")?.title)
            assertEquals(1, requests)
        } finally { app.close(); http.close() }
    }

    @Test fun retriesTemporaryFailuresWithoutCachingTheFailedResult() = runTest {
        var requests = 0
        val platform = PublicPlatform {
            requests++
            if (requests <= 3) error("temporary 422")
            obj("body" to obj("title" to "Recovered").toString()).toString()
        }
        val http = HttpClient(MockEngine { error("Unexpected API request") })
        val app = Session(Repository(Config("https://zoo.test", "/shapes", "alice", false),
            platform, http, backgroundScope, synchronize = false), backgroundScope, Environment())
        try {
            assertNull(app.previews.load("https://summaly.test/url", "https://target.test/"))
            assertEquals(3, requests)
            assertEquals("Recovered", app.previews.load("https://summaly.test/url", "https://target.test/")?.title)
            assertEquals(4, requests)
        } finally { app.close(); http.close() }
    }
    @Test fun staleMetadataStaysVisibleDuringSharedRefreshAndAfterFailure() = runTest {
        val clock = kotlin.time.TestTimeSource()
        var requests = 0
        var gate: CompletableDeferred<Unit>? = null
        var fail = false
        val platform = PublicPlatform {
            requests++
            gate?.await()
            if (fail) error("offline")
            obj("body" to obj("title" to if (requests == 1) "Cached" else "Updated").toString()).toString()
        }
        val http = HttpClient(MockEngine { error("Unexpected API request") })
        val app = Session(Repository(Config("https://zoo.test", "/shapes", "alice", false), platform, http, backgroundScope, synchronize = false), backgroundScope, Environment())
        val loader = PreviewLoader(app, clock)
        val service = "https://summaly.test/url"
        val url = "https://target.test/stale"
        try {
            assertEquals("Cached", loader.load(service, url)?.title)
            clock += kotlin.time.Duration.parse("6m")
            gate = CompletableDeferred()
            val first = async { loader.load(service, url) }
            val second = async { loader.load(service, url) }
            runCurrent()
            assertEquals(2, requests)
            assertEquals("Cached", loader.cached(service, url)?.title)
            first.cancelAndJoin()
            gate!!.complete(Unit)
            assertEquals("Updated", second.await()?.title)
            assertEquals("Updated", loader.cached(service, url)?.title)
            clock += kotlin.time.Duration.parse("6m")
            fail = true; gate = null
            assertEquals("Updated", loader.load(service, url)?.title)
            assertEquals(5, requests)
            assertEquals("Updated", loader.cached(service, url)?.title)
            assertEquals("Updated", loader.load(service, url)?.title)
            assertEquals(5, requests, "Failed refreshes must not retry on every remount")
        } finally { app.close(); http.close() }
    }

}
