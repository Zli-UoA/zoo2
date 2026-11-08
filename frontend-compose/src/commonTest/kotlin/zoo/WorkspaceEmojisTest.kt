@file:OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)
package zoo

import io.ktor.client.HttpClient
import io.ktor.client.engine.mock.*
import io.ktor.http.*
import kotlin.test.*
import kotlinx.coroutines.*
import kotlinx.coroutines.test.*

class WorkspaceEmojisTest {
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
    fun concurrentConsumersShareTheCatalogAndExplicitRefreshBypassesTheInterval() = runTest {
        var requests = 0
        val session = app { request ->
            if (request.url.encodedPath != "/api/emojis") awaitCancellation()
            requests++
            respond("[{\"name\":\"cat\"}]", headers = headersOf(HttpHeaders.ContentType, "application/json"))
        }
        coroutineScope { repeat(20) { launch { session.loadWorkspaceEmojis() } } }
        assertEquals(1, requests)
        assertEquals("cat", session.workspaceEmojis.single().text("name"))
        session.loadWorkspaceEmojis(force = true)
        assertEquals(2, requests)
    }
}
