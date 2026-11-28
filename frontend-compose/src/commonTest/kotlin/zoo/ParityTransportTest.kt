package zoo

import io.ktor.client.HttpClient
import io.ktor.client.engine.mock.*
import io.ktor.http.*
import io.ktor.http.content.TextContent
import kotlin.test.*
import kotlinx.coroutines.*
import kotlinx.coroutines.test.*
import kotlinx.serialization.json.*

@OptIn(ExperimentalCoroutinesApi::class)
class ParityTransportTest {
    private class Browser : Platform {
        override var messageEncryption: MessageEncryption? = null
        var channelCache: String? = null
        val cache = mutableMapOf<String, String>()
        val exits = mutableListOf<Row>()
        private var sequence = 0

        override suspend fun headers() = mapOf("X-Dev-User" to "alice")

        override suspend fun readCache(key: String): String? = if (":channels:" in key) channelCache else cache[key]

        override suspend fun writeCache(key: String, value: String) { cache[key] = value }

        override fun uuid() = "id-${++sequence}"

        var timestamp = "2026-01-02T09:00:00Z"

        override fun now() = timestamp

        override fun navigate(path: String) {}

        override fun translate(key: String, args: Map<String, String>) = key

        override fun action(name: String) {}

        override fun time(timestamp: String) = timestamp

        override fun day(timestamp: String) = timestamp.take(10)

        override suspend fun invoke(name: String, payload: String): String {
            if (name == "bubbleExit") exits += json.parseToJsonElement(payload).jsonObject
            return "{}"
        }
    }

    @Test
    fun creationIntentPreventsPlaintextAfterDowngradeAndRestart() = runTest {
        val platform = Browser()
        var messageWrites = 0
        var downgrade = true
        val attempted = mutableListOf<String>()
        fun client() = HttpClient(MockEngine { request ->
            if (request.url.encodedPath in listOf("/api/channels", "/api/dms")) {
                val body = json.parseToJsonElement((request.body as TextContent).text).jsonObject
                val id = body.text("id")
                assertTrue(platform.cache.values.any { id in it }) // HTTP より前に永続化する。
                val returned = if (request.url.encodedPath == "/api/dms") "existing-dm" else id
                attempted += returned
                respond(obj("data" to obj("id" to returned, "encrypted" to !downgrade)).toString())
            } else { messageWrites++; error("No plaintext write is allowed") }
        })
        val connection = Config("https://zoo.test", "/shapes", "alice", false, txidMode = "none")
        fun session(config: Config = connection) = Session(Repository(config, platform, client(), backgroundScope, synchronize = false), backgroundScope, Environment())
        val first = session()
        assertFailsWith<IllegalStateException> { first.createChannel(obj("is_private" to true, "encrypted" to true, "channel_name" to "secret")) }
        assertFailsWith<IllegalStateException> { first.createDm(setOf("bob"), true) }
        attempted.forEach { assertFalse(first.send(it, "secret", null)) }
        first.close()
        val reopened = session()
        attempted.forEach { assertFalse(reopened.send(it, "still secret", null)) }
        assertEquals(0, messageWrites)
        assertTrue(attempted.all { reopened.isEncryptedChannel(it) })
        downgrade = false
        val created = reopened.createChannel(obj("is_private" to true, "encrypted" to true, "channel_name" to "valid"))
        assertTrue(reopened.isEncryptedChannel(created.jsonObject.getValue("data").jsonObject.text("id")))
        val anotherAccount = session(connection.copy(userId = "bob"))
        anotherAccount.requirePlaintextChannel(attempted.first()) // 別アカウントに記録を流用しない。
        anotherAccount.close()
        reopened.close()
    }

    @Test
    fun encryptedChannelUsesOnlyEncryptionAndNeverFallsBackToCommandsOrPlaintext() = runTest {
        val platform = Browser().apply { channelCache = """[{"id":"secret","encrypted":"t"}]""" }
        var sent = ""
        var fail = false
        var writes = 0
        platform.messageEncryption = object : MessageEncryption {
            override val changes = kotlinx.coroutines.flow.MutableStateFlow(0L)
            override val recipients = kotlinx.coroutines.flow.MutableStateFlow(emptyMap<String, RecipientNotice>())
            override val attachmentCipher: AttachmentCipher get() = error("Unexpected attachment")
            override suspend fun send(channel: String, id: String, parent: String?, body: String,
                mentions: EncryptedMentions, attachments: List<kotlinx.serialization.json.JsonObject>) {
                if (fail) error("e2ee.sendNeedsUnlock")
                sent = body
            }
            override suspend fun edit(message: Row, body: String, mentions: EncryptedMentions?) = error("Unexpected edit")
            override suspend fun decrypt(message: Row) = error("Unexpected decrypt")
        }
        val http = HttpClient(MockEngine { request ->
            if (request.method != HttpMethod.Get) writes++
            awaitCancellation()
        })
        val app = Session(Repository(Config("https://zoo.test", "/shapes", "alice", false), platform, http, backgroundScope), backgroundScope, Environment())
        runCurrent()
        assertTrue(app.send("secret", "/echo secret text", "parent"))
        assertEquals("/echo secret text", sent)
        fail = true
        assertFalse(app.send("secret", "never send plaintext", null))
        assertEquals("e2ee.sendNeedsUnlock", app.toast)
        assertFalse(app.send("secret", "file", null, attachmentIds = listOf("plaintext-file")))
        assertFalse(app.send("secret", "/echo secret text", null, commandAppId = "echo"))
        assertEquals(0, writes)
        app.close()
        http.close()
    }

    /** `Session.send` の平文・暗号化の両方の `catch` が、参加が必要な 403 だけ、参加の案内を出す。 */
    @Test
    fun aReplyRefusedForAMissingJoinShowsTheJoinHintOnBothSendPaths() = runTest {
        val platform = Browser().apply { channelCache = """[{"id":"secret","encrypted":"t"}]""" }
        var refusal = """{"error":"Join the thread before replying","code":"THREAD_JOIN_REQUIRED"}"""
        val encryptedWrites = mutableListOf<String?>()
        platform.messageEncryption = object : MessageEncryption {
            override val changes = kotlinx.coroutines.flow.MutableStateFlow(0L)
            override val recipients = kotlinx.coroutines.flow.MutableStateFlow(emptyMap<String, RecipientNotice>())
            override val attachmentCipher: AttachmentCipher get() = error("Unexpected attachment")
            override suspend fun send(channel: String, id: String, parent: String?, body: String,
                mentions: EncryptedMentions, attachments: List<kotlinx.serialization.json.JsonObject>) {
                encryptedWrites += parent
                throw ApiException(403, json.parseToJsonElement(refusal).jsonObject)
            }
            override suspend fun edit(message: Row, body: String, mentions: EncryptedMentions?) = error("Unexpected edit")
            override suspend fun decrypt(message: Row) = error("Unexpected decrypt")
        }
        val http = HttpClient(MockEngine { request ->
            if (request.method == HttpMethod.Get) awaitCancellation()
            respond(refusal, HttpStatusCode.Forbidden)
        })
        val app = Session(
            Repository(Config("https://zoo.test", "/shapes", "alice", false, txidMode = "none"), platform, http, backgroundScope),
            backgroundScope,
            Environment(),
        )
        runCurrent()
        for (channel in listOf("plain", "secret")) {
            app.dismissToast()
            assertFalse(app.send(channel, "reply", "root"))
            assertEquals("threads.joinToReply", app.toast, channel)
            // 親のない投稿は、スレッドの参加では直らない。
            app.dismissToast()
            assertFalse(app.send(channel, "post", null))
            assertEquals("common.writeError", app.toast, channel)
            // 投稿権限の 403 は、参加を案内しない。
            refusal = """{"error":"Only channel managers may post here","code":"POSTING_RESTRICTED"}"""
            app.dismissToast()
            assertFalse(app.send(channel, "reply", "root"))
            assertEquals("common.writeError", app.toast, channel)
            refusal = """{"error":"Join the thread before replying","code":"THREAD_JOIN_REQUIRED"}"""
        }
        assertEquals(listOf<String?>("root", null, "root"), encryptedWrites)
        app.close()
        http.close()
    }

    @Test
    fun bubbleRetriesLatestContentSerializesWritesAndClearsOnExit() = runTest {
        val platform = Browser()
        val writes = mutableListOf<Row>()
        val http =
            HttpClient(MockEngine) {
                engine {
                    dispatcher = StandardTestDispatcher(testScheduler)
                    addHandler { request ->
                        if (request.url.encodedPath != "/api/bubble") awaitCancellation()
                        if (request.method == HttpMethod.Get) respond("[]")
                        else {
                            writes +=
                                json
                                    .parseToJsonElement((request.body as TextContent).text)
                                    .jsonObject
                            if (writes.size == 1) respond("{}", HttpStatusCode.ServiceUnavailable)
                            else respond("{\"ok\":true,\"server_time\":1767344400000}")
                        }
                    }
                }
            }
        val repo =
            Repository(
                Config("https://zoo.test", "/shapes", "alice", false, txidMode = "none"),
                platform,
                http,
                backgroundScope,
            )
        val app = Session(repo, backgroundScope, Environment())
        val controller = BubbleController(app)
        controller.edit("first draft")
        val job = backgroundScope.launch { controller.run() }
        runCurrent()
        assertFalse(controller.sharing.value)
        controller.edit("latest draft")
        advanceTimeBy(1400)
        runCurrent()
        assertEquals("latest draft", writes.last().text("content"))
        assertTrue(controller.sharing.value)
        job.cancelAndJoin()
        assertEquals("", writes.last().text("content"))
        assertEquals(writes.indices.map { it + 1 }, writes.map { it.text("seq").toInt() })
        assertTrue(platform.exits.last().flag("clear"))
        assertFalse(controller.sharing.value)
        app.close()
    }

    @Test
    fun selectedCommandsKeepAppIdentityAndUnknownCommandsFallBackToMessages() = runTest {
        val platform = Browser()
        val requests = mutableListOf<Pair<String, Row>>()
        val http =
            HttpClient(
                MockEngine { request ->
                    if (request.method == HttpMethod.Get) awaitCancellation()
                    val path = request.url.encodedPath
                    val body =
                        json.parseToJsonElement((request.body as TextContent).text).jsonObject
                    requests += path to body
                    if (path == "/api/apps/commands/unknown") respond("{}", HttpStatusCode.NotFound)
                    else if (path == "/api/apps/forbidden/invoke")
                        respond("{}", HttpStatusCode.Forbidden)
                    else respond("{}")
                }
            )
        val repo =
            Repository(
                Config("https://zoo.test", "/shapes", "alice", false, txidMode = "none"),
                platform,
                http,
                backgroundScope,
            )
        val app = Session(repo, backgroundScope, Environment())
        assertTrue(app.send("c", "/echo hello", null, "notes"))
        assertEquals("/api/apps/notes/invoke", requests.last().first)
        assertEquals("hello", requests.last().second.nested("values").text("text"))
        assertFalse(app.send("c", "/echo secret", null, "forbidden"))
        assertTrue(requests.none { it.first == "/api/messages" })
        assertTrue(app.send("c", "/unknown ordinary text", null))
        assertEquals("/api/messages", requests.last().first)
        assertEquals("/unknown ordinary text", requests.last().second.text("content"))
        assertTrue(app.send("c", "/echo attached", null, "notes", listOf("attachment-1")))
        assertEquals("/api/messages", requests.last().first)
        assertEquals(
            "attachment-1",
            requests.last().second["attachment_ids"]!!.jsonArray.single().jsonPrimitive.content,
        )
        app.close()
    }

    @Test
    fun replyNormalizedToTheThreadRootMovesTheOpenThread() = runTest {
        val http =
            HttpClient(
                MockEngine { request ->
                    if (request.method == HttpMethod.Get) awaitCancellation()
                    respond("""{"data":{"id":"x","parent_id":"root"},"txid":null}""")
                }
            )
        val repo =
            Repository(
                Config("https://zoo.test", "/shapes", "alice", false, txidMode = "none"),
                Browser(),
                http,
                backgroundScope,
            )
        val app = Session(repo, backgroundScope, Environment())
        app.thread = "other"
        assertTrue(app.send("c", "hello", "child"))
        assertEquals("other", app.thread)
        app.thread = "child"
        assertTrue(app.send("c", "hello", "child"))
        assertEquals("root", app.thread)
        app.close()
    }

    @Test
    fun attachmentUrlsAreCachedUntilExpiryAndRecoverFromRefreshFailures() = runTest {
        val platform = Browser()
        var calls = 0
        val http =
            HttpClient(
                MockEngine { request ->
                    if (request.url.encodedPath != "/api/attachments/file/url") awaitCancellation()
                    calls++
                    if (calls == 2) respond("{}", HttpStatusCode.ServiceUnavailable)
                    else respond("{\"download_url\":\"https://storage.test/file.png\"}")
                }
            )
        val repo =
            Repository(
                Config("https://zoo.test", "/shapes", "alice", false, txidMode = "none"),
                platform,
                http,
                backgroundScope,
            )
        val app = Session(repo, backgroundScope, Environment())
        assertNull(app.attachmentLinks.cachedUrl("file"))
        assertEquals("https://storage.test/file.png", app.attachmentLinks.get("file"))
        assertEquals("https://storage.test/file.png", app.attachmentLinks.cachedUrl("file"))
        app.attachmentLinks.get("file")
        assertEquals(1, calls)
        platform.timestamp = "2026-01-02T09:05:00Z"
        assertNull(app.attachmentLinks.cachedUrl("file"))
        assertFailsWith<ApiException> { app.attachmentLinks.get("file") }
        assertTrue(app.attachmentLinks.errors["file"] == true)
        assertEquals("https://storage.test/file.png", app.attachmentLinks.get("file", true))
        assertEquals(3, calls)
        assertFalse(app.attachmentLinks.errors["file"] == true)
        app.close()
    }

    @Test
    fun failedFinalBubbleClearKeepsTheKeepaliveSnapshot() = runTest {
        val platform = Browser()
        var writes = 0
        val http =
            HttpClient(MockEngine) {
                engine {
                    dispatcher = StandardTestDispatcher(testScheduler)
                    addHandler { request ->
                        if (
                            request.url.encodedPath != "/api/bubble" ||
                                request.method == HttpMethod.Get
                        )
                            awaitCancellation()
                        writes++
                        if (writes == 1) respond("{\"ok\":true}")
                        else respond("{}", HttpStatusCode.ServiceUnavailable)
                    }
                }
            }
        val repo =
            Repository(
                Config("https://zoo.test", "/shapes", "alice", false, txidMode = "none"),
                platform,
                http,
                backgroundScope,
            )
        val app = Session(repo, backgroundScope, Environment())
        val controller = BubbleController(app)
        controller.edit("shared")
        val job = backgroundScope.launch { controller.run() }
        runCurrent()
        assertTrue(controller.sharing.value)
        job.cancelAndJoin()
        assertEquals(2, writes)
        assertFalse(platform.exits.last().flag("clear"))
        assertEquals("", platform.exits.last().nested("body").text("content"))
        app.close()
    }
}
