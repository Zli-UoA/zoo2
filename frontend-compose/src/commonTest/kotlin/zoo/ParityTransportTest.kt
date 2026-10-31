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
        val exits = mutableListOf<Row>()
        private var sequence = 0

        override suspend fun headers() = mapOf("X-Dev-User" to "alice")

        override suspend fun readCache(key: String): String? = null

        override suspend fun writeCache(key: String, value: String) {}

        override fun uuid() = "id-${++sequence}"

        override fun now() = "2026-01-02T09:00:00Z"

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
        app.close()
    }
}
