@file:OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)

package zoo

import io.ktor.client.HttpClient
import io.ktor.client.engine.mock.*
import io.ktor.http.*
import kotlin.test.*
import kotlinx.coroutines.*
import kotlinx.coroutines.test.*

class ActivityPubNotificationsStateTest {
    private class Browser : Platform {
        override suspend fun headers() = emptyMap<String, String>()

        override suspend fun readCache(key: String): String? = null

        override suspend fun writeCache(key: String, value: String) {}

        override fun uuid() = "apub-test"

        override fun now() = "2026-11-01T10:00:00Z"

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
                    addHandler(handler)
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

    @Test
    fun paginationDeduplicatesAndWritesUseTheRealEndpointThenRefresh() = runTest {
        var approved = false
        val writes = mutableListOf<String>()
        val app = app { request ->
            if (
                request.url.encodedPath !in
                    setOf("/api/activitypub/notifications", "/api/activitypub/follows/respond")
            )
                awaitCancellation()
            if (request.method == HttpMethod.Post) {
                assertEquals("/api/activitypub/follows/respond", request.url.encodedPath)
                writes += request.body.toByteArray().decodeToString()
                approved = true
                respond("{}")
            } else {
                val older = request.url.parameters["before"]
                val rows =
                    if (older == null) "[{\"id\":\"one\",\"is_read\":$approved}]"
                    else "[{\"id\":\"one\"},{\"id\":\"two\"}]"
                respond(
                    "{\"items\":$rows,\"unread_count\":${if (approved) 0 else 1},\"next_cursor\":${if (older==null) "\"cursor / next\"" else "null"}}"
                )
            }
        }
        val state = ActivityPubNotificationsState(app, subscribe = { {} })
        state.ensureLoaded()
        state.ensureLoaded()
        runCurrent()
        assertEquals(1, state.unreadCount)
        state.loadMore()
        runCurrent()
        assertEquals(listOf("one", "two"), state.items.map { it.text("id") })
        assertNull(state.nextCursor)
        val body =
            obj(
                "actor" to "https://peer/users/bob",
                "activity_id" to "activity-one",
                "accept" to true,
            )
        backgroundScope.launch { state.act("/api/activitypub/follows/respond", body) }
        runCurrent()
        assertEquals(listOf(body.toString()), writes)
        assertEquals(0, state.unreadCount)
        assertEquals(listOf("one", "two"), state.items.map { it.text("id") }, "Approval refresh retains already loaded older notifications")
        assertFalse(state.loadingMore)
        app.close()
    }

    @Test
    fun refreshFailureKeepsExistingRowsAndMutationFailureDoesNotPretendSuccess() = runTest {
        var reject = false
        var reads = 0
        val app = app { request ->
            // Repository shape subscriptions are independent traffic, not feed refreshes.
            // Observe only the actual notification endpoints under this contract.
            if (
                request.url.encodedPath !in
                    setOf("/api/activitypub/notifications", "/api/activitypub/notifications/read")
            )
                awaitCancellation()
            if (request.method == HttpMethod.Post) {
                assertEquals("/api/activitypub/notifications/read", request.url.encodedPath)
                assertEquals(
                    obj("id" to "retained").toString(),
                    request.body.toByteArray().decodeToString(),
                )
                respond("{}", HttpStatusCode.InternalServerError)
            } else {
                reads++
                if (reject) respond("{}", HttpStatusCode.InternalServerError)
                else respond("{\"items\":[{\"id\":\"retained\"}],\"unread_count\":3}")
            }
        }
        val state = ActivityPubNotificationsState(app, subscribe = { {} })
        state.ensureLoaded()
        runCurrent()
        reject = true
        state.refresh()
        runCurrent()
        assertTrue(state.error)
        assertEquals("retained", state.items.single().text("id"))
        assertEquals(3, state.unreadCount)
        val mutation =
            backgroundScope.async {
                runCatching {
                    state.act("/api/activitypub/notifications/read", obj("id" to "retained"))
                }
            }
        runCurrent()
        val failure = assertIs<ApiException>(mutation.await().exceptionOrNull())
        assertEquals(500, failure.status)
        assertEquals(2, reads, "A rejected write must not trigger a success refresh")
        assertEquals("retained", state.items.single().text("id"))
        assertTrue(state.error)
        assertEquals(3, state.unreadCount)
        app.close()
    }
    @Test
    fun selectedReadAcknowledgesOnlyVisibleIdsAndRollsBackFailures() = runTest {
        var reject = false
        val writes = mutableListOf<String>()
        val app = app { request ->
            if (request.url.encodedPath !in setOf("/api/activitypub/notifications", "/api/activitypub/notifications/read")) awaitCancellation()
            if (request.method == HttpMethod.Post) {
                writes += request.body.toByteArray().decodeToString()
                respond("{}", if (reject) HttpStatusCode.InternalServerError else HttpStatusCode.OK)
            } else respond("""{"items":[{"id":"one","is_read":false},{"id":"two","is_read":false}],"unread_count":2}""")
        }
        val state = ActivityPubNotificationsState(app, subscribe = { {} })
        state.ensureLoaded(); runCurrent()
        val first = backgroundScope.async { state.markRead(listOf("one")) }
        runCurrent(); first.await()
        assertTrue(state.items[0].flag("is_read"))
        assertFalse(state.items[1].flag("is_read"))
        assertEquals(1, state.unreadCount)
        assertEquals(listOf("""{"ids":["one"]}"""), writes)
        reject = true
        val failed = backgroundScope.async { runCatching { state.markRead(listOf("two")) } }
        runCurrent(); assertTrue(failed.await().isFailure)
        assertFalse(state.items[1].flag("is_read"))
        assertEquals(1, state.unreadCount)
        app.close()
    }

    @Test
    fun updatesDuringSlowFetchDoNotCancelItAndAreCoalesced() = runTest {
        val finish = CompletableDeferred<Unit>()
        var calls = 0
        var cancelled = false
        val app = app { request ->
            if (request.url.encodedPath != "/api/activitypub/notifications") awaitCancellation()
            calls++
            if (calls == 1) try { finish.await() } catch (e: CancellationException) { cancelled = true; throw e }
            respond("""{"items":[{"id":"fresh","is_read":false}],"unread_count":1}""")
        }
        val state = ActivityPubNotificationsState(app, subscribe = { {} })
        state.ensureLoaded(); runCurrent()
        repeat(20) { state.refresh() }
        runCurrent()
        assertEquals(1, calls)
        assertFalse(cancelled)
        finish.complete(Unit); runCurrent()
        assertEquals(2, calls)
        assertEquals(listOf("fresh"), state.items.map { it.text("id") })
        state.close(); app.close()
    }

}
