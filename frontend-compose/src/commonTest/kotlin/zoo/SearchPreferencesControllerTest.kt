package zoo

import io.ktor.client.HttpClient
import io.ktor.client.engine.mock.*
import io.ktor.http.*
import kotlin.test.*
import kotlinx.coroutines.*
import kotlinx.coroutines.test.*

@OptIn(ExperimentalCoroutinesApi::class)
class SearchPreferencesControllerTest {
    private class Browser : Platform {
        override suspend fun headers() = emptyMap<String, String>()

        override suspend fun readCache(key: String): String? = null

        override suspend fun writeCache(key: String, value: String) {}

        override fun uuid() = "preferences-test"

        override fun now() = "2026-01-02T09:00:00Z"

        override fun navigate(path: String) {}

        override fun action(name: String) {}

        override fun translate(key: String, args: Map<String, String>) = key

        override fun time(timestamp: String) = timestamp

        override fun day(timestamp: String) = timestamp
    }

    @Test
    fun remountsShareOneReadAndFailedSavesPreserveThePreviousEngine() = runTest {
        var reads = 0
        var rejectSave = true
        val http =
            HttpClient(MockEngine) {
                engine {
                    dispatcher = StandardTestDispatcher(testScheduler)
                    addHandler { request ->
                        if (request.url.encodedPath != "/api/users/me/preferences")
                            awaitCancellation()
                        if (request.method == HttpMethod.Get) {
                            reads++
                            respond("{\"search_engine\":\"duckduckgo\"}")
                        } else if (rejectSave) respond("{}", HttpStatusCode.InternalServerError)
                        else respond("{\"search_engine\":\"yandex\"}")
                    }
                }
            }
        val app =
            Session(
                Repository(
                    Config("https://zoo.test", "/shapes", "alice", false, txidMode = "none"),
                    Browser(),
                    http,
                    backgroundScope,
                ),
                backgroundScope,
                Environment(),
            )
        val firstPanel = app.searchPreferences
        firstPanel.ensureLoaded()
        runCurrent()
        assertEquals("duckduckgo", app.searchEngine)
        assertFalse(firstPanel.loading)
        val remountedPanel = app.searchPreferences
        remountedPanel.ensureLoaded()
        runCurrent()
        assertSame(firstPanel, remountedPanel)
        assertEquals(1, reads)
        firstPanel.save("baidu")
        assertTrue(firstPanel.saving)
        runCurrent()
        assertEquals("duckduckgo", app.searchEngine)
        assertEquals("save", firstPanel.error)
        assertFalse(firstPanel.saving)
        rejectSave = false
        firstPanel.save("yandex")
        runCurrent()
        assertEquals("yandex", app.searchEngine)
        assertEquals("", firstPanel.error)
        app.close()
    }

    @Test
    fun closingTheSessionCancelsAnOutstandingSave() = runTest {
        val release = CompletableDeferred<Unit>()
        var entered = false
        val http =
            HttpClient(MockEngine) {
                engine {
                    dispatcher = StandardTestDispatcher(testScheduler)
                    addHandler { request ->
                        if (request.url.encodedPath != "/api/users/me/preferences")
                            awaitCancellation()
                        if (request.method == HttpMethod.Patch) {
                            entered = true
                            release.await()
                        }
                        respond("{\"search_engine\":\"google\"}")
                    }
                }
            }
        val app =
            Session(
                Repository(
                    Config("https://zoo.test", "/shapes", "alice", false, txidMode = "none"),
                    Browser(),
                    http,
                    backgroundScope,
                ),
                backgroundScope,
                Environment(),
            )
        val preferences = app.searchPreferences
        preferences.ensureLoaded()
        runCurrent()
        preferences.save("baidu")
        runCurrent()
        assertTrue(entered)
        app.close()
        release.complete(Unit)
        runCurrent()
        assertEquals("google", app.searchEngine)
    }
}
