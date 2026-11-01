package zoo

import electric4k.RuntimeVisibilityAdapter
import electric4k.ShapeStreamOptions
import io.ktor.client.*
import io.ktor.client.engine.mock.*
import io.ktor.http.*
import kotlin.test.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.test.*
import kotlinx.serialization.json.*

private class FakePlatform : Platform {
    var token = "first-token"
    val readKeys = mutableListOf<String>()
    val cache = mutableMapOf<String, String>()
    val saved = CompletableDeferred<Unit>()

    override suspend fun headers() = mapOf("Authorization" to "Bearer $token")

    override suspend fun readCache(key: String): String? {
        readKeys += key
        return cache[key]
    }

    override suspend fun writeCache(key: String, value: String) {
        cache[key] = value
        saved.complete(Unit)
    }

    override fun uuid() = "new-id"

    override fun now() = "2026-10-30T00:00:00Z"

    override fun navigate(path: String) {}

    override fun translate(key: String, args: Map<String, String>) = key

    override fun action(name: String) {}

    override fun time(timestamp: String) = timestamp

    override fun day(timestamp: String) = timestamp
}

@OptIn(ExperimentalCoroutinesApi::class)
class RepositoryTest {
    private val config = Config("https://zoo.test", "/api/shapes", "alice", false)
    private val options =
        ShapeStreamOptions(
            liveSse = true,
            detectSystemWake = false,
            runtimeVisibility =
                object : RuntimeVisibilityAdapter {
                    override fun isVisible() = true

                    override fun subscribe(callback: (Boolean) -> Unit): () -> Unit = {}
                },
        )

    @Test
    fun restResolvesAuthenticationForEachRequestAndDoesNotSendSecretsInTheUrl() = runTest {
        val platform = FakePlatform()
        val tokens = mutableListOf<String?>()
        val http =
            HttpClient(
                MockEngine { request ->
                    tokens += request.headers[HttpHeaders.Authorization]
                    assertFalse(request.url.toString().contains("token"))
                    assertEquals(ContentType.Application.Json, request.body.contentType)
                    respond("{\"data\":{},\"txid\":null}", HttpStatusCode.OK)
                }
            )
        try {
            val api = ZooApi(config, platform, http)
            api.request("/api/messages", "POST", buildJsonObject { put("content", "hello") })
            platform.token = "renewed-token"
            api.request("/api/messages", "POST", buildJsonObject { put("content", "again") })
            assertEquals<List<String?>>(
                listOf("Bearer first-token", "Bearer renewed-token"),
                tokens,
            )
        } finally {
            http.close()
        }
    }

    @Test
    fun browserFetchErrorIsRecoverableByFeatureEffects() = runTest {
        val failure = Error("Fail to fetch")
        val http = HttpClient(MockEngine { throw failure })
        try {
            val error =
                assertFailsWith<IllegalStateException> {
                    ZooApi(config, FakePlatform(), http).request("/api/channels/c1/read-state")
                }
            // JVM coroutine stacktrace recovery may copy the Error and keep the
            // original as its cause. The transport failure must remain in the chain.
            assertTrue(generateSequence(error.cause) { it.cause }.any { it === failure })
        } finally {
            http.close()
        }
    }

    @Test
    fun initialShapeIsMaterializedAndAFailedWriteRollsBackOnlyItsOwnOptimisticRow() = runTest {
        val platform = FakePlatform()
        var initialRequests = 0
        val http =
            HttpClient(
                MockEngine { request ->
                    when {
                        request.method == HttpMethod.Post -> respond("{}", HttpStatusCode.Forbidden)
                        request.url.parameters["offset"] == "-1" -> {
                            initialRequests++
                            assertNull(request.url.parameters["table"])
                            assertNull(request.url.parameters["where"])
                            assertEquals("channel-a", request.url.parameters["channel_id"])
                            respond(
                                """[{"key":"row-a","value":{"id":"m1","content":"server"},"headers":{"operation":"insert"}},{"headers":{"control":"up-to-date"}}]""",
                                headers =
                                    headersOf(
                                        "electric-handle" to listOf("test-handle"),
                                        "electric-offset" to listOf("1_0"),
                                        "electric-cursor" to listOf("1"),
                                        "electric-schema" to
                                            listOf(
                                                """{"id":{"type":"text"},"content":{"type":"text"}}"""
                                            ),
                                        "content-type" to listOf("application/json"),
                                    ),
                            )
                        }
                        else -> awaitCancellation()
                    }
                }
            )
        val table =
            SyncedTable(
                "messages",
                config,
                platform,
                http,
                backgroundScope,
                mapOf("channel_id" to "channel-a"),
                options,
            )
        try {
            table.rows.first { it.isNotEmpty() }
            assertEquals(1, initialRequests)
            assertEquals("server", table.rows.value.single().text("content"))
            val optimistic = buildJsonObject {
                put("id", "m2")
                put("content", "draft")
            }
            assertFailsWith<ApiException> {
                table.write(
                    ZooApi(config, platform, http),
                    "/api/messages",
                    "POST",
                    optimistic,
                    optimistic,
                )
            }
            runCurrent()
            assertEquals(listOf("m1"), table.rows.value.map { it.text("id") })
            assertTrue(platform.readKeys.single().contains(":alice:messages:"))
            platform.saved.await()
            assertTrue(platform.cache.values.any { it.contains("server") })
        } finally {
            table.close()
            http.close()
        }
    }

    @Test
    fun emptyInitialSnapshotIsPersistedWithoutAStateFlowChange() = runTest {
        val platform = FakePlatform()
        val http =
            HttpClient(
                MockEngine { request ->
                    if (request.url.parameters["offset"] != "-1") awaitCancellation()
                    respond(
                        """[{"headers":{"control":"up-to-date"}}]""",
                        headers =
                            headersOf(
                                "electric-handle" to listOf("empty-handle"),
                                "electric-offset" to listOf("0_0"),
                                "electric-cursor" to listOf("1"),
                                "electric-schema" to listOf("{}"),
                                "content-type" to listOf("application/json"),
                            ),
                    )
                }
            )
        val table =
            SyncedTable(
                "messages",
                config,
                platform,
                http,
                backgroundScope,
                mapOf("channel_id" to "channel-a"),
                options,
            )
        try {
            table.loading.first { !it }
            runCurrent()
            assertEquals(listOf("[]"), platform.cache.values.toList())
        } finally {
            table.close()
            http.close()
        }
    }

    @Test
    fun cancellationStopsTheTableAndReleasesItsLiveRequest() = runTest {
        val platform = FakePlatform()
        val started = CompletableDeferred<Unit>()
        val cancelled = CompletableDeferred<Unit>()
        val http =
            HttpClient(
                MockEngine {
                    started.complete(Unit)
                    try {
                        awaitCancellation()
                    } finally {
                        cancelled.complete(Unit)
                    }
                }
            )
        val table =
            SyncedTable(
                "channels",
                config,
                platform,
                http,
                backgroundScope,
                streamOptions = options,
            )
        started.await()
        table.close()
        cancelled.await()
        assertTrue(cancelled.isCompleted)
        http.close()
    }
}
