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

    private fun TestScope.app(profiles: String = "{\"accounts\":[]}", handler: MockRequestHandler): Session {
        val http =
            HttpClient(MockEngine) {
                engine {
                    dispatcher = StandardTestDispatcher(testScheduler)
                    addHandler { request ->
                        if (request.url.encodedPath == "/api/emoji-accounts")
                            respond(profiles, headers = headersOf(HttpHeaders.ContentType, "application/json"))
                        else handler.invoke(this, request)
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

    @Test
    fun accountProfilesComeFromTheDedicatedDirectory() = runTest {
        val session = app(profiles = """{"accounts":[{"username":"alice","display_name":"AuthZ name","avatar_url":null}]}""") { request ->
            assertEquals("/api/emojis", request.url.encodedPath)
            respond("[]", headers = headersOf(HttpHeaders.ContentType, "application/json"))
        }
        session.loadWorkspaceEmojis()
        assertEquals("AuthZ name", session.accountProfiles.single().text("display_name"))
        assertTrue(session.workspaceEmojis.isEmpty())
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
    @Test
    fun refreshingAfterRegistrationAndDeletionShowsChangesImmediately() = runTest {
        var name = "cat"
        var requests = 0
        val session = app { request ->
            if (!request.url.encodedPath.startsWith("/api/emojis")) awaitCancellation()
            when (request.method) {
                HttpMethod.Post -> { name = "dog"; respond("{}") }
                HttpMethod.Delete -> { name = ""; respond("{}") }
                else -> {
                    requests++
                    respond(if (name.isEmpty()) "[]" else "[{\"name\":\"$name\"}]",
                        headers = headersOf(HttpHeaders.ContentType, "application/json"))
                }
            }
        }
        session.loadWorkspaceEmojis()
        session.api.request("/api/emojis", "POST")
        session.loadWorkspaceEmojis(force = true)
        assertEquals("dog", session.workspaceEmojis.single().text("name"))
        session.api.request("/api/emojis/dog", "DELETE")
        session.loadWorkspaceEmojis(force = true)
        assertTrue(session.workspaceEmojis.isEmpty())
        assertEquals(3, requests)
    }

    @Test
    fun explicitRetryDoesNotWaitAfterAnError() = runTest {
        var requests = 0
        val session = app { request ->
            if (request.url.encodedPath != "/api/emojis") awaitCancellation()
            requests++
            if (requests == 1) respond("{}", HttpStatusCode.ServiceUnavailable)
            else respond("[]", headers = headersOf(HttpHeaders.ContentType, "application/json"))
        }
        assertFailsWith<Exception> { session.loadWorkspaceEmojis() }
        assertTrue(session.workspaceEmojiError)
        session.loadWorkspaceEmojis(force = true)
        assertFalse(session.workspaceEmojiError)
        assertEquals(2, requests)
    }

}
