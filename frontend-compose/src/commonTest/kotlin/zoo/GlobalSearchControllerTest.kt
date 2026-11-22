package zoo

import io.ktor.client.HttpClient
import io.ktor.client.engine.mock.*
import io.ktor.http.*
import kotlin.test.*
import kotlinx.coroutines.*
import kotlinx.coroutines.test.*

@OptIn(ExperimentalCoroutinesApi::class)
class GlobalSearchControllerTest {
    private class Browser : Platform {
        override suspend fun headers() = emptyMap<String, String>()

        override suspend fun readCache(key: String): String? = null

        override suspend fun writeCache(key: String, value: String) {}

        override fun uuid() = "search-test"

        override fun now() = "2026-11-21T09:00:00Z"

        override fun navigate(path: String) {}

        override fun action(name: String) {}

        override fun translate(key: String, args: Map<String, String>) = key

        override fun time(timestamp: String) = timestamp

        override fun day(timestamp: String) = timestamp
    }

    private fun TestScope.app(handler: MockRequestHandler): Session {
        val http =
            HttpClient(MockEngine) {
                engine {
                    dispatcher = StandardTestDispatcher(testScheduler)
                    addHandler { request ->
                        // Shape subscriptions are unrelated traffic; observe only /api/search.
                        if (request.url.encodedPath != "/api/search") awaitCancellation()
                        handler(request)
                    }
                }
            }
        return Session(
            Repository(
                Config("https://zoo.test", "/shapes", "alice", false, txidMode = "none"),
                Browser(),
                http,
                backgroundScope,
            ),
            backgroundScope,
            Environment(),
        )
    }

    private fun page(ids: List<String>, cursor: String?, until: String?) =
        obj(
                "items" to kotlinx.serialization.json.JsonArray(ids.map { obj("id" to it, "channel_id" to "c") }),
                "next_cursor" to cursor,
                "searched_until" to until,
            )
            .toString()

    @Test
    fun olderPagesUseTheEncodedCursorAndShortPagesKeepTheCursor() = runTest {
        val requests = mutableListOf<Parameters>()
        val cursor = """{"m":24310,"at":null,"id":null}"""
        val app = app { request ->
            requests += request.url.parameters
            when (request.url.parameters["cursor"]) {
                null -> respond(page(listOf("one"), cursor, "2025-12"))
                cursor -> respond(page(emptyList(), "{\"m\":24298}", "2024-12"))
                else -> respond(page(listOf("two"), null, "2024-06"))
            }
        }
        val search = GlobalSearchController(app, backgroundScope)
        search.search("  会議  資料 ", GlobalSearchType.Files)
        assertTrue(search.loading)
        runCurrent()
        assertEquals("会議  資料", requests.single()["q"])
        assertEquals("files", requests.single()["type"])
        assertNull(requests.single()["cursor"])
        assertEquals(listOf("one"), search.items.map { it.text("id") })
        assertEquals(cursor, search.nextCursor)
        assertEquals("2025-12", search.searchedUntil)
        assertFalse(search.loading)

        search.loadOlder()
        search.loadOlder()
        assertTrue(search.loadingOlder)
        runCurrent()
        assertEquals(2, requests.size, "A second tap while loading must not send another request")
        assertEquals(cursor, requests[1]["cursor"])
        // Nothing in the next 12 months, but the server still has older months.
        assertEquals(listOf("one"), search.items.map { it.text("id") })
        assertEquals("{\"m\":24298}", search.nextCursor)
        assertEquals("2024-12", search.searchedUntil)

        search.loadOlder()
        runCurrent()
        assertEquals(listOf("one", "two"), search.items.map { it.text("id") })
        assertNull(search.nextCursor)
        search.loadOlder()
        runCurrent()
        assertEquals(3, requests.size)
        app.close()
    }

    @Test
    fun requestsAreSerialAndStaleResultsAreDropped() = runTest {
        val gates = mutableMapOf<String, CompletableDeferred<Unit>>()
        val started = mutableListOf<String>()
        var active = 0
        var maxActive = 0
        val app = app { request ->
            val q = request.url.parameters["q"]!!
            started += q
            active++
            maxActive = maxOf(maxActive, active)
            try {
                gates.getOrPut(q) { CompletableDeferred() }.await()
            } finally {
                active--
            }
            respond(page(listOf("$q-${request.url.parameters["type"]}"), null, "2026-01"))
        }
        val search = GlobalSearchController(app, backgroundScope)
        search.search("a")
        runCurrent()
        search.search("ab")
        search.search("abc")
        search.search("abc", GlobalSearchType.Files)
        runCurrent()
        assertEquals(listOf("a"), started, "Newer queries wait for the running request")
        gates.getOrPut("a") { CompletableDeferred() }.complete(Unit)
        runCurrent()
        assertEquals(listOf("a", "abc"), started, "Only the latest query is sent next")
        assertTrue(search.items.isEmpty(), "The stale result for 'a' is dropped")
        assertTrue(search.loading)
        gates.getOrPut("abc") { CompletableDeferred() }.complete(Unit)
        runCurrent()
        assertEquals(listOf("abc-files"), search.items.map { it.text("id") })
        assertEquals(1, maxActive)
        search.search(" abc ", GlobalSearchType.Files)
        runCurrent()
        assertEquals(2, started.size, "The same query and kind does not search again")

        search.search("")
        assertFalse(search.loading)
        assertTrue(search.items.isEmpty())
        runCurrent()
        assertEquals(2, started.size, "An empty query is not sent")
        app.close()
    }

    @Test
    fun failuresKeepLoadedRowsAndRetryTheSameRequest() = runTest {
        var reject = false
        val requests = mutableListOf<Parameters>()
        val app = app { request ->
            requests += request.url.parameters
            if (reject) respond("{}", HttpStatusCode.InternalServerError)
            else if (request.url.parameters["cursor"] == null) respond(page(listOf("one"), "next", "2026-01"))
            else respond(page(listOf("two"), null, "2025-01"))
        }
        val search = GlobalSearchController(app, backgroundScope)
        reject = true
        search.search("x")
        runCurrent()
        assertTrue(search.error)
        assertFalse(search.loaded)
        assertFalse(search.loading)
        reject = false
        search.retry()
        assertTrue(search.loading)
        runCurrent()
        assertFalse(search.error)
        assertEquals(listOf("one"), search.items.map { it.text("id") })

        reject = true
        search.loadOlder()
        runCurrent()
        assertTrue(search.error)
        assertFalse(search.loadingOlder)
        assertEquals(listOf("one"), search.items.map { it.text("id") }, "A failed older page keeps loaded rows")
        assertEquals("next", search.nextCursor)
        search.loadOlder()
        runCurrent()
        assertEquals(3, requests.size, "Older pages wait for an explicit retry after a failure")
        reject = false
        search.retry()
        runCurrent()
        assertEquals("next", requests.last()["cursor"])
        assertEquals(listOf("one", "two"), search.items.map { it.text("id") })
        assertNull(search.nextCursor)
        app.close()
    }

    @Test
    fun routesAndDestinations() {
        assertEquals("/search?type=messages", globalSearchPath("", GlobalSearchType.Messages))
        assertEquals("/search?q=a%20b&type=files", globalSearchPath("a b", GlobalSearchType.Files))
        assertTrue(isGlobalSearchPath("/search?q=a"))
        assertFalse(isGlobalSearchPath("/searching"))
        assertEquals(GlobalSearchType.All, GlobalSearchType.from("notes"))
        assertEquals(GlobalSearchType.All, GlobalSearchType.from(null))
        // In `all`, a file result opens the message it is attached to.
        assertEquals(
            "/channels/c1?message=m9",
            globalSearchDestination(
                obj("id" to "a1", "kind" to "file", "message_id" to "m9", "channel_id" to "c1"),
                GlobalSearchType.All,
            ),
        )
        assertEquals(
            "/channels/c1?message=m1",
            globalSearchDestination(obj("id" to "m1", "channel_id" to "c1"), GlobalSearchType.Messages),
        )
        assertEquals(
            "/channels/c1?message=m2",
            globalSearchDestination(obj("id" to "f1", "message_id" to "m2", "channel_id" to "c1"), GlobalSearchType.Files),
        )
    }
}
