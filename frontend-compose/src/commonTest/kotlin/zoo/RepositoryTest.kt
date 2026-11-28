package zoo

import androidx.compose.runtime.snapshots.Snapshot

import electric4k.RuntimeVisibilityAdapter
import electric4k.ShapeStreamOptions
import io.ktor.client.*
import io.ktor.client.engine.mock.*
import io.ktor.http.*
import kotlin.test.*
import kotlinx.coroutines.*
import kotlinx.coroutines.channels.Channel
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
    @Test
    fun freshNotificationContextRestoresLinkDespiteAnEmptyCachedPreview() = runTest {
        val pendingWrite = CompletableDeferred<Unit>()
        var contextResponse = """{"message":{"id":"message"},"reactions":[]}"""
        val http = HttpClient(MockEngine) {
            engine {
                dispatcher = StandardTestDispatcher(testScheduler)
                addHandler { request ->
                    if (request.url.encodedPath.endsWith("/context")) respond(contextResponse)
                    else {
                        pendingWrite.await()
                        respond("{}")
                    }
                }
            }
        }
        val repo = Repository(Config("https://zoo.test", "/shapes", "me", false),
            FakePlatform(), http, backgroundScope, synchronize = false)
        val app = Session(repo, backgroundScope, Environment())
        try {
            backgroundScope.launch {
                repo.notifications.write(repo.api, "/pending-notification", "POST", row = obj(
                    "id" to "notice", "user_id" to "me", "actor_id" to "alice",
                    "channel_id" to "channel", "message_id" to "message", "notification_type" to "reaction",
                    "created_at" to "2026-11-01T00:00:00Z", "display_data" to obj("message" to null),
                ))
            }
            runCurrent()
            val inbox = NotificationInbox(app)
            val item = inbox.snapshot().single()
            assertNull(item.target)
            val context = assertNotNull(item.reactionContext)
            inbox.loadReactions(context)
            assertEquals("/channels/channel?message=message", inbox.snapshot().single().target)

            contextResponse = """{"message":null,"reactions":[]}"""
            inbox.loadReactions(context)
            assertNull(inbox.snapshot().single().target)

            pendingWrite.complete(Unit)
            runCurrent()
            assertTrue(inbox.snapshot().isEmpty(), "A row removed by sync must not remain in the inbox")
            inbox.loadReactions(context)
            assertTrue(inbox.snapshot().isEmpty())
        } finally { app.close() }
    }

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
    fun publicSessionDoesNotReadPrivateCachesOrStartShapeRequests() = runTest {
        val platform = FakePlatform()
        var requests = 0
        val http = HttpClient(MockEngine { requests++; error("Unexpected private request") })
        val repo = Repository(config, platform, http, backgroundScope, synchronize = false)
        val app = Session(repo, backgroundScope, Environment(path = "/public/users/alice"))
        app.start()
        runCurrent()
        assertEquals(0, requests)
        assertTrue(platform.readKeys.isEmpty())
        app.close()
        runCurrent()
        assertEquals(0, requests)
    }

    @Test
    fun channelReadUsesTheServerUnreadFlagAndSkipsOldOrHiddenMessages() = runTest {
        val first = "2026-11-01T01:00:00Z"
        val second = "2026-11-01T02:00:00Z"
        val third = "2026-11-01T03:00:00Z"
        var writes = 0
        val http = HttpClient(MockEngine) {
            engine {
                dispatcher = StandardTestDispatcher(testScheduler)
                addHandler { request ->
                    when (request.url.encodedPath) {
                        "/api/channel-read-states" -> {
                            val timestamp = listOf(first, second, third)[writes]
                            respond("""[{"channel_id":"c1","last_read_at":"$timestamp","has_unread":${writes < 2}}]""")
                        }
                        "/api/channels/c1/read-state" -> {
                            assertEquals(HttpMethod.Post, request.method)
                            writes++
                            val timestamp = if (writes == 1) second else third
                            respond("""{"data":{"last_read_at":"$timestamp","has_unread":${writes == 1}}}""")
                        }
                        else -> respond("", HttpStatusCode.NotFound)
                    }
                }
            }
        }
        val repo = Repository(config, FakePlatform(), http, backgroundScope, synchronize = false)
        val app = Session(repo, backgroundScope, Environment(path = "/channels/c1"))
        try {
            app.start()
            runCurrent()
            assertTrue(app.channelReadPositions.getValue("c1").hasUnread)
            val secondTime = checkNotNull(timestampMillis(second))
            val thirdTime = checkNotNull(timestampMillis(third))
            app.markChannelRead("c1", "second", secondTime)
            app.markChannelRead("c1", "second", secondTime)
            assertEquals(1, writes)
            assertTrue(app.channelReadPositions.getValue("c1").hasUnread)

            app.update(app.env.copy(visible = false))
            app.markChannelRead("c1", "third", thirdTime)
            assertEquals(1, writes)
            app.update(app.env.copy(visible = true))
            app.markChannelRead("c1", "third", thirdTime)
            assertEquals(2, writes)
            assertFalse(app.channelReadPositions.getValue("c1").hasUnread)
        } finally { app.close() }
    }

    @Test
    fun timedOutReadPositionsStopBlockingTheConversationAfterForegroundRecovery() = runTest {
        var reads = 0
        val http = HttpClient(MockEngine) {
            engine {
                dispatcher = StandardTestDispatcher(testScheduler)
                addHandler { request ->
                    if (request.url.encodedPath != "/api/channel-read-states") {
                        respond("", HttpStatusCode.NotFound)
                    } else {
                        if (++reads <= 2) awaitCancellation()
                        respond("""[{"channel_id":"c1","has_unread":false}]""")
                    }
                }
            }
        }
        val repo = Repository(config, FakePlatform(), http, backgroundScope, synchronize = false)
        val foreground = Environment(path = "/channels/c1", visible = true, online = true)
        val app = Session(repo, backgroundScope, foreground)
        try {
            app.start()
            runCurrent()
            assertEquals(1, reads)
            assertFalse(app.channelReadPositionsChecked)

            app.update(foreground.copy(visible = false))
            Snapshot.sendApplyNotifications()
            advanceTimeBy(20_000)
            assertEquals(1, reads)
            assertFalse(app.channelReadPositionsChecked)

            app.update(foreground)
            Snapshot.sendApplyNotifications()
            runCurrent()
            assertEquals(2, reads)
            advanceTimeBy(10_000)
            runCurrent()
            assertTrue(app.channelReadPositionsChecked)
            assertTrue(app.channelReadPositions.isEmpty())

            advanceTimeBy(10_000)
            runCurrent()
            assertEquals(3, reads)
            assertTrue("c1" in app.channelReadPositions)
        } finally {
            app.close()
        }
    }

    @Test
    fun returningToForegroundRefreshesAnExistingShapeEvenWhenStillOnline() =
        assertShapeReconnect("https://foreground.zoo.test") { app, foreground ->
            app.update(foreground.copy(visible = false))
            runCurrent()
            app.update(foreground)
        }

    @Test
    fun replacingTheDefaultNetworkRefreshesShapesEvenWhenStillOnline() =
        assertShapeReconnect("https://network-change.zoo.test") { app, foreground ->
            app.update(foreground, networkChanged = true)
        }

    @Test
    fun replacingTheDefaultNetworkReconnectsTheSharedHomeSocket() = runTest {
        val http = HttpClient(MockEngine { error("No Shape or REST request expected") })
        val repo = Repository(config, FakePlatform(), http, backgroundScope, synchronize = false)
        val foreground = Environment(online = true, visible = true)
        val app = Session(repo, backgroundScope, foreground)
        val events = mutableListOf<String>()
        val home = HomeLive(app) { _, _, _, _ ->
            val dispose = { events += "disconnect" }
            events += "connect"
            dispose
        }
        try {
            home.subscribe {}
            home.subscribe {}
            runCurrent()
            assertEquals(listOf("connect"), events)

            app.update(foreground, networkChanged = true)
            Snapshot.sendApplyNotifications()
            runCurrent()
            assertEquals(listOf("connect", "disconnect", "connect"), events)

            app.update(foreground)
            Snapshot.sendApplyNotifications()
            runCurrent()
            assertEquals(listOf("connect", "disconnect", "connect"), events)
        } finally {
            home.close()
            app.close()
        }
    }

    private fun assertShapeReconnect(
        base: String,
        reconnect: TestScope.(Session, Environment) -> Unit,
    ) = runTest {
        val requests = Channel<Url>(Channel.UNLIMITED)
        val platform = FakePlatform()
        val http = HttpClient(MockEngine) {
            engine {
                dispatcher = StandardTestDispatcher(testScheduler)
                addHandler { request ->
                    if (request.url.encodedPath != "/api/shapes/channels") awaitCancellation()
                    requests.send(request.url)
                    if (request.url.parameters["offset"] != "-1") awaitCancellation()
                    respond(
                        """[{"headers":{"control":"up-to-date"}}]""",
                        headers = headersOf(
                            "electric-handle" to listOf("test-handle"),
                            "electric-offset" to listOf("1_0"),
                            "electric-cursor" to listOf("1"),
                            "electric-schema" to listOf("{}"),
                            "content-type" to listOf("application/json"),
                        ),
                    )
                }
            }
        }
        val repo = Repository(config.copy(apiBaseUrl = base), platform, http, backgroundScope)
        val foreground = Environment(online = true, visible = true)
        val app = Session(repo, backgroundScope, foreground)
        try {
            assertEquals("-1", requests.receive().parameters["offset"])
            assertEquals("true", requests.receive().parameters["live"])

            app.update(foreground)
            runCurrent()
            assertTrue(requests.tryReceive().isFailure)

            reconnect(app, foreground)
            val resumed = requests.receive().parameters
            assertEquals("test-handle", resumed["handle"])
            assertEquals("1_0", resumed["offset"])
            assertNotEquals("true", resumed["live"])

            app.update(foreground)
            runCurrent()
            assertTrue(requests.tryReceive().isFailure)
        } finally {
            app.close()
        }
    }

    @Test
    fun foregroundRecoveryRestartsAStalledInitialShapeRequest() = runTest {
        val requests = Channel<Url>(Channel.UNLIMITED)
        val http = HttpClient(MockEngine) {
            engine {
                dispatcher = StandardTestDispatcher(testScheduler)
                addHandler { request ->
                    requests.send(request.url)
                    awaitCancellation()
                }
            }
        }
        val table = SyncedTable("channels", config, FakePlatform(), http, backgroundScope, streamOptions = options)
        try {
            runCurrent()
            val initial = requests.tryReceive().getOrThrow()
            assertEquals("-1", initial.parameters["offset"])
            assertTrue(table.loading.value)

            table.refresh(reconnect = true)
            runCurrent()
            assertEquals(initial, requests.tryReceive().getOrThrow())
            assertTrue(requests.tryReceive().isFailure)
        } finally {
            table.close()
            http.close()
        }
    }

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
            assertEquals(emptyList(), table.synchronizedRows.value)
        } finally {
            table.close()
            http.close()
        }
    }

    @Test
    fun cachedRowsDoNotPublishASuccessfulNetworkSnapshot() = runTest {
        val cached = obj("id" to "old", "content" to "cached")
        val platform = object : Platform by FakePlatform() {
            override suspend fun readCache(key: String) = JsonArray(listOf(cached)).toString()
        }
        val release = CompletableDeferred<Unit>()
        val http = HttpClient(MockEngine { request ->
            if (request.url.parameters["offset"] != "-1") awaitCancellation()
            release.await()
            respond(
                """[{"headers":{"control":"up-to-date"}}]""",
                headers = headersOf(
                    "electric-handle" to listOf("empty-handle"),
                    "electric-offset" to listOf("0_0"),
                    "electric-cursor" to listOf("1"),
                    "electric-schema" to listOf("{}"),
                    "content-type" to listOf("application/json"),
                ),
            )
        })
        val table = SyncedTable("messages", config, platform, http, backgroundScope,
            mapOf("channel_id" to "channel-a"), options)
        try {
            table.loading.first { !it }
            runCurrent()
            assertEquals(listOf(cached), table.replicatedRows.value)
            assertNull(table.synchronizedRows.value)
            release.complete(Unit)
            assertEquals(emptyList(), table.synchronizedRows.first { it != null })
        } finally { table.close(); http.close() }
    }

    @Test
    fun reloadAfterShapeFailureRestartsSyncAndCoalescesRepeatedTaps() = runTest {
        val retryStarted = CompletableDeferred<Unit>()
        val releaseRetry = CompletableDeferred<Unit>()
        var initialRequests = 0
        val http = HttpClient(MockEngine { request ->
            if (request.url.parameters["offset"] != "-1") awaitCancellation()
            if (++initialRequests == 1) {
                respond("denied", HttpStatusCode.Forbidden)
            } else {
                retryStarted.complete(Unit)
                releaseRetry.await()
                respond(
                    """[{"key":"channel","value":{"id":"c1"},"headers":{"operation":"insert"}},{"headers":{"control":"up-to-date"}}]""",
                    headers = headersOf(
                        "electric-handle" to listOf("reloaded-handle"),
                        "electric-offset" to listOf("1_0"),
                        "electric-cursor" to listOf("1"),
                        "electric-schema" to listOf("""{"id":{"type":"text"}}"""),
                        "content-type" to listOf("application/json"),
                    ),
                )
            }
        })
        val table = SyncedTable("channels", config, FakePlatform(), http, backgroundScope,
            streamOptions = options)
        try {
            table.error.first { it != null }
            assertFalse(table.loading.value)
            assertNull(table.synchronizedRows.value)

            table.refresh()
            retryStarted.await()
            table.refresh()
            runCurrent()
            releaseRetry.complete(Unit)

            val rows = table.synchronizedRows.first { it != null }
            assertEquals(listOf("c1"), rows?.map { it.text("id") })
            assertNull(table.error.value)
            assertFalse(table.loading.value)
            assertEquals(2, initialRequests)
        } finally {
            table.close()
            http.close()
        }
    }

    @Test
    fun liveFailureRetainsRowsAndFailedReloadCanBeRetried() = runTest {
        val failLive = CompletableDeferred<Unit>()
        val retryStarted = CompletableDeferred<Unit>()
        val failRetry = CompletableDeferred<Unit>()
        var initialRequests = 0
        val http = HttpClient(MockEngine { request ->
            if (request.url.parameters["offset"] != "-1") {
                if (initialRequests != 1) awaitCancellation()
                failLive.await()
                respond("live denied", HttpStatusCode.Forbidden)
            } else {
                val attempt = ++initialRequests
                if (attempt == 2) {
                    retryStarted.complete(Unit)
                    failRetry.await()
                    respond("reload denied", HttpStatusCode.Unauthorized)
                } else {
                    val id = if (attempt == 1) "old" else "new"
                    respond(
                        """[{"key":"channel","value":{"id":"$id"},"headers":{"operation":"insert"}},{"headers":{"control":"up-to-date"}}]""",
                        headers = headersOf(
                            "electric-handle" to listOf("handle-$attempt"),
                            "electric-offset" to listOf("1_0"),
                            "electric-cursor" to listOf("1"),
                            "electric-schema" to listOf("""{"id":{"type":"text"}}"""),
                            "content-type" to listOf("application/json"),
                        ),
                    )
                }
            }
        })
        val table = SyncedTable("channels", config, FakePlatform(), http, backgroundScope,
            streamOptions = options)
        try {
            val original = assertNotNull(table.synchronizedRows.first { it != null })
            assertEquals(listOf("old"), original.map { it.text("id") })
            failLive.complete(Unit)
            table.error.first { it != null }
            assertFalse(table.loading.value)
            assertEquals(original, table.replicatedRows.value)
            assertSame(original, table.synchronizedRows.value)

            table.refresh()
            retryStarted.await()
            assertTrue(table.loading.value)
            assertNull(table.error.value)
            failRetry.complete(Unit)
            table.error.first { it != null }
            assertFalse(table.loading.value)
            assertEquals(original, table.replicatedRows.value)
            assertSame(original, table.synchronizedRows.value)

            table.refresh()
            val recovered = assertNotNull(table.synchronizedRows.first { it != original })
            assertEquals(listOf("new"), recovered.map { it.text("id") })
            assertEquals(recovered, table.replicatedRows.value)
            assertFalse(table.loading.value)
            assertNull(table.error.value)
            assertEquals(3, initialRequests)
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

    // ---- on-demand（Android の messages）----

    private val onDemandSchema =
        """{"id":{"type":"text"},"content":{"type":"text"},"created_at":{"type":"text"},"parent_id":{"type":"text"}}"""

    private fun protocolHeaders(handle: String = "h", offset: String = "1_0", cursor: String = "1") =
        headersOf(
            "electric-handle" to listOf(handle),
            "electric-offset" to listOf(offset),
            "electric-cursor" to listOf(cursor),
            "electric-schema" to listOf(onDemandSchema),
            "content-type" to listOf("application/json"),
        )

    private fun message(index: Int) =
        """{"key":"m$index","value":{"id":"m${index.toString().padStart(3, '0')}","content":"c$index","created_at":"2026-11-01T${(index / 3600).toString().padStart(2, '0')}:${(index / 60 % 60).toString().padStart(2, '0')}:${(index % 60).toString().padStart(2, '0')}Z","parent_id":null},"headers":{"operation":"insert"}}"""

    private fun snapshot(messages: List<String>) =
        """{"metadata":{"xmin":"1","xmax":"1","xip_list":[],"snapshot_mark":1,"database_lsn":"1"},"data":[${messages.joinToString(",")}]}"""

    @Test
    fun onDemandMessagesReadOnePageThroughSubsetAndNeverTheWholeShape() = runTest {
        val platform = FakePlatform()
        val subsets = mutableListOf<Map<String, String>>()
        var wholeShapeRequests = 0
        val http =
            HttpClient(
                MockEngine { request ->
                    val p = request.url.parameters
                    when {
                        p["subset__where"] != null -> {
                            subsets += p.entries().associate { it.key to it.value.first() }
                            respond(snapshot((1..3).map(::message)), headers = protocolHeaders())
                        }
                        p["offset"] == "-1" -> {
                            wholeShapeRequests++
                            respond("[]", headers = protocolHeaders())
                        }
                        p["offset"] == "now" ->
                            respond("""[{"headers":{"control":"up-to-date"}}]""", headers = protocolHeaders())
                        else -> awaitCancellation()
                    }
                }
            )
        val table =
            SyncedTable("messages", config, platform, http, backgroundScope,
                mapOf("channel_id" to "channel-a"), options, onDemand = true)
        try {
            table.rows.first { it.isNotEmpty() }
            assertEquals(0, wholeShapeRequests)
            assertEquals(3, table.rows.value.size)
            val first = subsets.first()
            assertEquals("50", first["subset__limit"])
            assertEquals("created_at DESC, id DESC", first["subset__order_by"])
            assertEquals("parent_id IS NULL AND deleted_at IS NULL", first["subset__where"])
            assertEquals("changes_only", first["log"])
            val state = table.pageStates.first { it[PageScope.Main]?.loaded == true }.getValue(PageScope.Main)
            assertFalse(state.hasOlder)
            assertFalse(table.loading.value)
        } finally {
            table.close()
            http.close()
        }
    }

    @Test
    fun foregroundRecoveryRetriesThePendingFirstMessagePage() = runTest {
        val subsets = Channel<Url>(Channel.UNLIMITED)
        val cancelled = CompletableDeferred<Unit>()
        var attempts = 0
        val http = HttpClient(MockEngine) {
            engine {
                dispatcher = StandardTestDispatcher(testScheduler)
                addHandler { request ->
                    val parameters = request.url.parameters
                    when {
                        parameters["subset__where"] != null -> {
                            subsets.send(request.url)
                            if (++attempts == 1) {
                                try {
                                    awaitCancellation()
                                } finally {
                                    cancelled.complete(Unit)
                                }
                            }
                            assertTrue(cancelled.isCompleted)
                            respond(snapshot(listOf(message(1))), headers = protocolHeaders())
                        }
                        parameters["offset"] == "now" ->
                            respond("""[{"headers":{"control":"up-to-date"}}]""", headers = protocolHeaders())
                        else -> awaitCancellation()
                    }
                }
            }
        }
        val table = SyncedTable("messages", config, FakePlatform(), http, backgroundScope,
            mapOf("channel_id" to "resume-first-page"), options, onDemand = true)
        try {
            val pending = subsets.receive()
            assertTrue(table.pageStates.value.getValue(PageScope.Main).loading)

            table.refresh(reconnect = true)

            assertEquals(pending, subsets.receive())
            val rows = table.rows.first { it.isNotEmpty() }
            table.pageStates.first { it[PageScope.Main]?.loaded == true }
            assertEquals(listOf("m001"), rows.map { it.text("id") })
            assertEquals(2, attempts)
        } finally {
            table.close()
            http.close()
        }
    }

    @Test
    fun onDemandOlderPageContinuesFromTheOldestRowAndStopsWhenItIsNotFull() = runTest {
        val platform = FakePlatform()
        val subsets = mutableListOf<Map<String, String>>()
        val http =
            HttpClient(
                MockEngine { request ->
                    val p = request.url.parameters
                    when {
                        p["subset__where"] != null -> {
                            subsets += p.entries().associate { it.key to it.value.first() }
                            // サーバーの並び（created_at DESC, id DESC）で返す。最古の行は最後の m100。
                            val rows = if (subsets.size == 1) (100 until 150).reversed().map(::message) else listOf(message(1))
                            respond(snapshot(rows), headers = protocolHeaders())
                        }
                        p["offset"] == "now" ->
                            respond("""[{"headers":{"control":"up-to-date"}}]""", headers = protocolHeaders())
                        else -> awaitCancellation()
                    }
                }
            )
        val table =
            SyncedTable("messages", config, platform, http, backgroundScope,
                mapOf("channel_id" to "channel-a"), options, onDemand = true)
        try {
            val first = table.pageStates.first { it[PageScope.Main]?.loaded == true }.getValue(PageScope.Main)
            assertTrue(first.hasOlder)
            // 最古の行は、先頭の行（m149）ではなく、最後の行 m100。
            assertEquals("2026-11-01T00:01:40Z" to "m100", first.oldest)
            assertTrue(table.loadOlder(PageScope.Main))
            val second = subsets[1]
            assertTrue(second.getValue("subset__where").contains("created_at < \$1 OR (created_at = \$1 AND id < \$2)"))
            val bound = Json.parseToJsonElement(second.getValue("subset__params")).jsonObject
            assertEquals("2026-11-01T00:01:40Z", bound.getValue("1").jsonPrimitive.content)
            assertEquals("m100", bound.getValue("2").jsonPrimitive.content)
            val after = table.pageStates.value.getValue(PageScope.Main)
            assertFalse(after.hasOlder)
            // 取り終わった範囲は、もう取りに行かない。
            assertFalse(table.loadOlder(PageScope.Main))
            assertEquals(2, subsets.size)
        } finally {
            table.close()
            http.close()
        }
    }

    @Test
    fun onDemandKeepsCachedRowsUntilTheFirstPageAndNeverPersistsAnEmptyInitialSnapshot() = runTest {
        val cached = obj("id" to "old", "content" to "cached", "created_at" to "2026-01-01T00:00:00Z")
        val written = mutableMapOf<String, String>()
        val platform =
            object : Platform by FakePlatform() {
                override suspend fun readCache(key: String) =
                    if (":v3:" in key) JsonArray(listOf(cached)).toString() else null

                override suspend fun writeCache(key: String, value: String) {
                    written[key] = value
                }
            }
        val release = CompletableDeferred<Unit>()
        val http =
            HttpClient(
                MockEngine { request ->
                    val p = request.url.parameters
                    when {
                        p["subset__where"] != null -> {
                            release.await()
                            respond(snapshot(listOf(message(1))), headers = protocolHeaders())
                        }
                        p["offset"] == "now" ->
                            respond("""[{"headers":{"control":"up-to-date"}}]""", headers = protocolHeaders())
                        else -> awaitCancellation()
                    }
                }
            )
        val table =
            SyncedTable("messages", config, platform, http, backgroundScope,
                mapOf("channel_id" to "channel-a"), options, onDemand = true)
        try {
            table.loading.first { !it }
            runCurrent()
            // 最初のページが届く前: 控えの行を残し、空の `[]` を書かない。
            assertEquals(listOf(cached), table.replicatedRows.value)
            assertTrue(written.isEmpty())
            release.complete(Unit)
            table.rows.first { rows -> rows.any { it.text("id") == "m001" } }
            assertEquals(listOf("m001"), table.rows.value.map { it.text("id") })
        } finally {
            table.close()
            http.close()
        }
    }

    @Test
    fun onDemandKeepsOnlyTheNewestPageInTheOfflineCache() {
        val rows =
            (1..120).map {
                obj("id" to "m${it.toString().padStart(3, '0')}",
                    "created_at" to "2026-11-01T00:${(it / 60).toString().padStart(2, '0')}:${(it % 60).toString().padStart(2, '0')}Z")
            } + obj("id" to "reply", "parent_id" to "m001", "created_at" to "2026-12-01T00:00:00Z") +
                obj("id" to "gone", "deleted_at" to "2026-12-01T00:00:00Z", "created_at" to "2026-12-02T00:00:00Z")
        val kept = feedCacheRows(rows)
        assertEquals(MESSAGE_PAGE_SIZE, kept.size)
        assertEquals("m120", kept.first().text("id"))
        assertTrue(kept.none { it.text("id") == "reply" || it.text("id") == "gone" })
    }

    @Test
    fun messageCachePreservesConfirmedCoverageAndReadsLegacyArrays() {
        val rows = (1..60).map { id ->
            obj("id" to id.toString(), "created_at" to "2026-11-01T00:00:${(id - 1).toString().padStart(2, '0')}Z")
        }
        fun cursor(row: Row) = row.text("created_at") to row.text("id")
        val complete = cacheFeed(rows.take(10), PageState(loaded = true, oldest = cursor(rows.first())))
        val restored = CachedFeed.decode(json.parseToJsonElement(complete.encode()))
        assertEquals(complete, restored)
        assertFalse(restored.page!!.hasOlder)
        assertNull(CachedFeed.decode(JsonArray(complete.rows)).page)

        val trimmed = cacheFeed(rows, PageState(loaded = true, oldest = cursor(rows.first())))
        assertEquals(50, trimmed.rows.size)
        assertEquals(cursor(rows[10]), trimmed.page!!.oldest)
        assertTrue(trimmed.page.hasOlder)

        val sparse = cacheFeed(listOf(rows.first()) + rows.takeLast(10),
            PageState(loaded = true, hasOlder = true, oldest = cursor(rows[50])))
        assertEquals(cursor(rows[50]), sparse.page!!.oldest)
    }

    @Test
    fun newestFirstOrdersRowsOfTheSameTimeById() {
        val a = obj("id" to "a", "created_at" to "t")
        val b = obj("id" to "b", "created_at" to "t")
        assertEquals(listOf(b, a), listOf(a, b).sortedWith(::compareNewestFirst))
    }

    @Test
    fun onDemandRetriesTheFirstPageUntilItSucceeds() = runTest {
        val platform = FakePlatform()
        var attempts = 0
        val http =
            HttpClient(
                MockEngine { request ->
                    val p = request.url.parameters
                    when {
                        p["subset__where"] != null -> {
                            attempts++
                            if (attempts == 1) respond("{}", HttpStatusCode.BadRequest)
                            else respond(snapshot(listOf(message(1))), headers = protocolHeaders())
                        }
                        p["offset"] == "now" ->
                            respond("""[{"headers":{"control":"up-to-date"}}]""", headers = protocolHeaders())
                        else -> awaitCancellation()
                    }
                }
            )
        val table =
            SyncedTable("messages", config, platform, http, backgroundScope,
                mapOf("channel_id" to "channel-a"), options, onDemand = true)
        try {
            table.rows.first { it.isNotEmpty() }
            assertEquals(2, attempts)
            assertEquals(listOf("m001"), table.rows.value.map { it.text("id") })
            assertTrue(table.pageStates.value.getValue(PageScope.Main).loaded)
        } finally {
            table.close()
            http.close()
        }
    }

    @Test
    fun invalidationBeforeTheFirstPageDoesNotFetchAnExtraPage() = runTest {
        var subsetCalls = 0
        val http = HttpClient(MockEngine) {
            engine {
                dispatcher = StandardTestDispatcher(testScheduler)
                addHandler { request ->
                    val parameters = request.url.parameters
                    when {
                        parameters["handle"] == null ->
                            respond("[]", HttpStatusCode.Conflict, headersOf("electric-handle", "new"))
                        parameters["subset__where"] != null -> {
                            subsetCalls++
                            respond(snapshot((1..50).reversed().map(::message)), headers = protocolHeaders("new"))
                        }
                        parameters["live"] == "true" -> awaitCancellation()
                        else -> respond("", HttpStatusCode.NoContent, protocolHeaders("new"))
                    }
                }
            }
        }
        val table = SyncedTable("messages", config, FakePlatform(), http, backgroundScope,
            mapOf("channel_id" to "invalidated-before-first-page"), options, onDemand = true)
        try {
            runCurrent()
            assertEquals(1, subsetCalls)
            assertEquals(50, table.rows.value.size)
            assertTrue(table.pageStates.value.getValue(PageScope.Main).hasOlder)
        } finally {
            table.close()
            http.close()
        }
    }

    @Test
    fun foregroundRecoveryPublishesSharedRowsWhenElectricSuppressesACachedControlMessage() = runTest {
        var invalidate = false
        val liveUpdate = CompletableDeferred<Unit>()
        var sentUpdate = false
        val http = HttpClient(MockEngine) {
            engine {
                dispatcher = StandardTestDispatcher(testScheduler)
                addHandler { request ->
                    if (request.url.encodedPath != "/api/shapes/channels") awaitCancellation()
                    val parameters = request.url.parameters
                    val handle = parameters["handle"]
                    when {
                        parameters["live"] == "true" -> {
                            if (handle != "new" || sentUpdate) awaitCancellation()
                            liveUpdate.await()
                            sentUpdate = true
                            respond(
                                """
                                data: {"key":"live-channel","value":{"id":"live-channel"},"headers":{"operation":"insert"}}

                                data: {"headers":{"control":"up-to-date"}}

                                """.trimIndent() + "\n",
                                headers = headersOf(
                                    "content-type" to listOf("text/event-stream"),
                                    "electric-handle" to listOf("new"),
                                    "electric-offset" to listOf("3_0"),
                                    "electric-cursor" to listOf("live-cursor"),
                                ),
                            )
                        }
                        invalidate && handle == "old" -> {
                            invalidate = false
                            respond("[]", HttpStatusCode.Conflict, headersOf("electric-handle", "new"))
                        }
                        else -> {
                            val id = if (handle == "new") "new-channel" else "old-channel"
                            respond(
                                """[{"key":"$id","value":{"id":"$id"},"headers":{"operation":"insert"}},{"headers":{"control":"up-to-date"}}]""",
                                headers = protocolHeaders(handle ?: "old", cursor = "same-cursor"),
                            )
                        }
                    }
                }
            }
        }
        val repo = Repository(config.copy(apiBaseUrl = "https://resume-shared.zoo.test"), FakePlatform(), http, backgroundScope)
        val foreground = Environment(online = true, visible = true)
        val app = Session(repo, backgroundScope, foreground)
        try {
            runCurrent()
            assertEquals("old-channel", repo.channels.rows.value.single().text("id"))

            app.update(foreground.copy(visible = false))
            invalidate = true
            app.update(foreground)
            runCurrent()

            assertEquals(listOf("new-channel"), repo.channels.rows.value.map { it.text("id") })
            assertFalse(repo.channels.loading.value)

            liveUpdate.complete(Unit)
            runCurrent()
            assertEquals(setOf("new-channel", "live-channel"), repo.channels.rows.value.map { it.text("id") }.toSet())
        } finally {
            app.close()
        }
    }

    @Test
    fun foregroundRecoveryRefetchesWhenElectricSuppressesAReplayedControlMessage() =
        assertForegroundRecoveryAfterInvalidation("resume-replayed-cursor", HttpStatusCode.OK)

    @Test
    fun foregroundRecoveryRefetchesWhenElectricCatchesUpWithNoContent() =
        assertForegroundRecoveryAfterInvalidation("resume-no-content", HttpStatusCode.NoContent)

    private fun assertForegroundRecoveryAfterInvalidation(channelId: String, catchUpStatus: HttpStatusCode) = runTest {
        var invalidate = false
        var subsetCalls = 0
        var activeReplacementLiveCalls = 0
        var maximumReplacementLiveCalls = 0
        val http = HttpClient(MockEngine) {
            engine {
                dispatcher = StandardTestDispatcher(testScheduler)
                addHandler { request ->
                    if (request.url.encodedPath != "/api/shapes/messages") awaitCancellation()
                    val parameters = request.url.parameters
                    assertEquals(channelId, parameters["channel_id"])
                    val handle = parameters["handle"]
                    val live = parameters["live"] == "true"
                    val ready = """[{"headers":{"control":"up-to-date"}}]"""
                    when {
                        parameters["subset__where"] != null -> {
                            subsetCalls++
                            assertEquals("50", parameters["subset__limit"])
                            assertEquals(if (subsetCalls == 1) "old" else "new", handle)
                            val index = if (subsetCalls == 1) 1 else 2
                            respond(snapshot(listOf(message(index))), headers = protocolHeaders(handle!!, "${index}_0"))
                        }
                        live -> {
                            if (handle != "new") awaitCancellation()
                            activeReplacementLiveCalls++
                            maximumReplacementLiveCalls = maxOf(maximumReplacementLiveCalls, activeReplacementLiveCalls)
                            try {
                                awaitCancellation()
                            } finally {
                                activeReplacementLiveCalls--
                            }
                        }
                        invalidate && handle == "old" -> {
                            invalidate = false
                            respond("[]", HttpStatusCode.Conflict, headersOf("electric-handle", "new"))
                        }
                        handle == "new" ->
                            respond(if (catchUpStatus == HttpStatusCode.NoContent) "" else ready,
                                catchUpStatus, protocolHeaders("new", "2_0", "same-cursor"))
                        else -> respond(ready, headers = protocolHeaders("old", cursor = "same-cursor"))
                    }
                }
            }
        }
        val repository = Repository(config, FakePlatform(), http, backgroundScope)
        val foreground = Environment(online = true, visible = true)
        val app = Session(repository, backgroundScope, foreground)
        val table = repository.messages(channelId)
        try {
            runCurrent()
            assertEquals(1, subsetCalls)
            assertEquals(listOf("m001"), table.rows.value.map { it.text("id") })

            app.update(foreground.copy(visible = false))
            invalidate = true
            app.update(foreground)
            runCurrent()

            assertEquals(2, subsetCalls, "A caught-up replacement Shape must reload the first page")
            assertEquals(listOf("m002"), table.rows.value.map { it.text("id") })
            assertTrue(table.pageStates.value.getValue(PageScope.Main).loaded)
            assertEquals(1, activeReplacementLiveCalls)
            assertEquals(1, maximumReplacementLiveCalls)
        } finally {
            app.close()
        }
    }

    @Test
    fun onDemandConsecutiveMustRefetchCancelsTheOldRefetchAndPagingResumes() = runTest {
        val platform = FakePlatform()
        var subsetCalls = 0
        var offsetCounter = 1
        val requestLog = mutableListOf<String>()
        // 409 のたびに、サーバーは新しい handle を返す。古い handle のままだと、取り直しが「期限切れの handle」として扱われる。
        var handle = "h0"
        var handleCounter = 0
        val secondSubsetStarted = CompletableDeferred<Unit>()
        // live の要求が来たら、この数だけ 409（must-refetch）で返す。待っている間に取り消された要求は数えない。
        val invalidations = kotlinx.coroutines.flow.MutableStateFlow(0)
        val http =
            HttpClient(
                MockEngine { request ->
                    val p = request.url.parameters
                    requestLog += listOf("offset", "handle", "live", "live_sse", "cursor", "subset__where", "log")
                        .mapNotNull { k -> p[k]?.let { "$k=${it.take(24)}" } }.joinToString(",")
                    when {
                        p["subset__where"] != null -> {
                            subsetCalls++
                            when (subsetCalls) {
                                1 -> respond(snapshot(listOf(message(1))), headers = protocolHeaders(handle))
                                // 1 回目の取り直し: 失敗させて、ジョブを再試行の待機に入れる。
                                2 -> {
                                    secondSubsetStarted.complete(Unit)
                                    respond("{}", HttpStatusCode.BadRequest)
                                }
                                else -> {
                                    // The replacement page belongs to the second generation. A timer retry
                                    // from the first generation must not serve the second snapshot early.
                                    if (p["handle"] != "h2") respond("{}", HttpStatusCode.BadRequest)
                                    else respond(snapshot(listOf(message(10))), headers = protocolHeaders(handle))
                                }
                            }
                        }
                        p["live"] == "true" || p["live_sse"] == "true" -> {
                            invalidations.first { it > 0 }
                            invalidations.value = invalidations.value - 1
                            handle = "h${++handleCounter}"
                            respond("[]", HttpStatusCode.Conflict, headersOf("electric-handle" to listOf(handle)))
                        }
                        // 購読（最初の offset=now、snapshot の後の追いつき、must-refetch の後の取り直し）。
                        // 追いつきの要求も、live に進めるよう up-to-date で返す。offset は毎回進める。
                        else ->
                            respond(
                                """[{"headers":{"control":"up-to-date"}}]""",
                                // cursor も毎回変える。同じ cursor を返すと、ライブラリは「再生」と見なして up-to-date を捨てる。
                                headers = protocolHeaders(handle, "${++offsetCounter}_0", "c$offsetCounter"),
                            )
                    }
                }
            )
        val table =
            SyncedTable("messages", config, platform, http, backgroundScope,
                mapOf("channel_id" to "channel-a"), options, onDemand = true)
        // どこで止まったか分かるよう、段階ごとに待ち時間を区切る。
        suspend fun <T> step(name: String, block: suspend () -> T): T =
            try {
                // 実時間で区切る。仮想時間で区切ると、別スレッドの応答を待つ間に、時計だけが進んで打ち切ってしまう。
                withContext(Dispatchers.Default) { withTimeout(20_000) { block() } }
            } catch (e: TimeoutCancellationException) {
                fail("did not finish: $name (subsetCalls=$subsetCalls, invalidations=${invalidations.value}) requests=${requestLog.takeLast(25)} error=${table.error.value} pages=${table.pageStates.value} loading=${table.loading.value} rows=${table.rows.value.map { it.text("id") }}")
            }
        try {
            step("first page") { table.rows.first { rows -> rows.any { it.text("id") == "m001" } } }
            val initialGeneration = table.pageGeneration.value
            // must-refetch その 1: 取り直しの最初の取得が失敗して、再試行を待つ。
            invalidations.value = 1
            step("second subset request") { secondSubsetStarted.await() }
            // must-refetch その 2: 再試行を待つ古いジョブは取り消され、新しい取り直しが範囲を取り直す。
            invalidations.value = 1
            step("rows after the second refetch") { table.rows.first { rows -> rows.any { it.text("id") == "m010" } } }
            assertEquals(listOf("m010"), table.rows.value.map { it.text("id") })
            assertTrue(table.pageGeneration.value > initialGeneration)
            assertTrue(table.pageStates.value.getValue(PageScope.Main).loaded)
            // 取り直しが済んでいれば、loadOlder は待たずに終わる（古いジョブが解除できなくても、新しいジョブが解除する）。
            val result = step("loadOlder after the refetch") { table.loadOlder(PageScope.Main) }
            assertFalse(result) // 50 件に満たないので、これより古いページはない。
        } finally {
            table.close()
            http.close()
        }
    }
}
