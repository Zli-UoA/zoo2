package zoo.android

import android.content.Context
import io.ktor.http.Url
import io.ktor.http.HttpHeaders
import io.ktor.http.HttpStatusCode
import io.ktor.http.headersOf
import io.ktor.client.HttpClient
import io.ktor.client.engine.mock.MockEngine
import io.ktor.client.engine.mock.respond
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import okhttp3.mockwebserver.MockResponse
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import kotlin.test.assertFailsWith
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [33])
class AndroidMediaProxyTest {
    private val base = "https://workspace.test"
    private val source = "https://remote.test/picture.png"

    @Test
    fun externalImagesRequireTheConfiguredProxy() {
        assertNull(imageRequest(base, source, ""))
        assertNull(imageRequest(base, source, "//proxy.test"))
        val request = assertNotNull(imageRequest(base, source, "/proxy"))
        assertTrue(request.proxied)
        assertEquals("workspace.test", Url(request.url).host)
        assertEquals(source, Url(request.url).parameters["url"])
    }

    @Test
    fun workspaceImagesReuseTheWorkspaceUrl() {
        assertEquals(ImageRequest("$base/api/home/images/ab"), imageRequest(base, "/api/home/images/ab", ""))
        assertNull(imageRequest(base, "//remote.test/picture.png", "/proxy"))
    }
    @Test fun dedicatedHttpsImageOriginsBypassTheProxy() {
        for (host in listOf("attachments.zliusercontent.com", "avater.zliusercontent.com",
            "emoji.zliusercontent.com", "emoji.zli.works")) {
            val url = "https://$host/picture.png"
            assertEquals(ImageRequest(url), imageRequest(base, url, "/proxy"))
        }
        for (url in listOf("http://emoji.zliusercontent.com/picture.png",
            "https://emoji.zliusercontent.com:8443/picture.png",
            "https://emoji.zliusercontent.com.evil.test/picture.png",
            "https://media-proxy.zliusercontent.com/picture.png")) {
            assertTrue(assertNotNull(imageRequest(base, url, "/proxy")).proxied)
        }
    }

    @Test fun httpSourceIsOnlyUsedThroughHttpsProxy() {
        val http = "http://remote.test/image.png"
        assertNull(imageRequest(base, http, ""))
        val request = assertNotNull(imageRequest(base, http, "/proxy"))
        assertEquals("https", Url(request.url).protocol.name)
        assertEquals(http, Url(request.url).parameters["url"])
    }

    @Test fun missingProxyIsRetriedAndWorkingConfigurationSurvivesRestart() = runBlocking {
        val context = RuntimeEnvironment.getApplication()
        var now = 1_000L
        var requests = 0
        val http = HttpClient(MockEngine {
            when (++requests) {
                1 -> respond("", HttpStatusCode.NotFound)
                2 -> respond("/proxy", headers = headersOf(HttpHeaders.ContentType, "text/plain"))
                3 -> respond("<html>unavailable</html>", headers = headersOf(HttpHeaders.ContentType, "text/html"))
                else -> respond("/new-proxy", headers = headersOf(HttpHeaders.ContentType, "text/plain"))
            }
        })
        try {
            val proxy = AndroidMediaProxy(context, base) { now }
            assertFailsWith<IllegalArgumentException> { proxy.request(source, http, online = true) }
            assertFailsWith<IllegalArgumentException> { proxy.request(source, http, online = true) }
            assertEquals(1, requests)
            now += 30_001
            val route = proxy.request(source, http, online = true)
            val restarted = AndroidMediaProxy(context, base) { now }
            assertEquals(route, restarted.request(source, http, online = true))
            assertEquals(route, restarted.request(source, http, online = false))
            assertEquals(2, requests)
            now += 86_400_001
            assertEquals(route, restarted.request(source, http, online = true, timeoutMillis = 700))
            assertEquals(route, restarted.request(source, http, online = true, timeoutMillis = 700))
            assertEquals(3, requests)
            now += 30_001
            val refreshed = restarted.request(source, http, online = true, timeoutMillis = 700)
            assertEquals("/new-proxy", Url(refreshed.url).encodedPath)
            assertEquals(4, requests)
        } finally { http.close() }
    }

    @Test fun pushUsesSavedImageBeforeRefreshingExpiredProxySettings() = runBlocking {
        AndroidImageCacheTest.Fixture().use { fixture ->
            val context = RuntimeEnvironment.getApplication()
            context.getSharedPreferences("media-proxy", Context.MODE_PRIVATE).edit()
                .putString(base, fixture.url).apply()
            val route = assertNotNull(imageRequest(base, source, fixture.url))
            var settingsRequests = 0
            val http = HttpClient(MockEngine {
                settingsRequests++
                respond("", HttpStatusCode.NotFound)
            })
            try {
                fixture.server.enqueue(MockResponse().setHeader("Cache-Control", "public, max-age=3600").setBody("cached"))
                fixture.server.enqueue(MockResponse().setBody("unexpected download"))
                fixture.cache().use { cache ->
                    cache.download(route, 1024, source = source)
                    val proxy = AndroidMediaProxy(context, base) { 86_400_001 }
                    assertEquals("cached", proxy.image(source, http, cache, push = true).decodeToString())
                }
                assertEquals(0, settingsRequests)
                assertEquals(1, fixture.server.requestCount)
            } finally { http.close() }
        }
    }

    @Test fun pushFallsBackToSavedProxyWhenSettingsRequestTimesOut() = runBlocking {
        AndroidImageCacheTest.Fixture().use { fixture ->
            val context = RuntimeEnvironment.getApplication()
            val prefs = context.getSharedPreferences("media-proxy", Context.MODE_PRIVATE)
            prefs.edit().putString(base, fixture.url).apply()
            val http = HttpClient(MockEngine {
                delay(10_000)
                error("Proxy settings request was not cancelled")
            })
            try {
                fixture.server.enqueue(MockResponse().setBody("downloaded"))
                fixture.cache().use { cache ->
                    val proxy = AndroidMediaProxy(context, base) { 86_400_001 }
                    assertEquals("downloaded", proxy.image(source, http, cache, push = true).decodeToString())
                    prefs.edit().remove(base).apply()
                    val freshProxy = AndroidMediaProxy(context, base) { 86_400_001 }
                    val failure = assertFailsWith<IllegalArgumentException> {
                        freshProxy.image("https://remote.test/uncached.png", http, cache, push = true)
                    }
                    assertEquals("Image proxy lookup timed out", failure.message)
                }
                assertEquals(1, fixture.server.requestCount)
            } finally { http.close() }
        }
    }

    @Test fun missingProxyDoesNotDownloadExternalImages() = runBlocking {
        AndroidImageCacheTest.Fixture().use { fixture ->
            val context = RuntimeEnvironment.getApplication()
            val http = HttpClient(MockEngine { respond("", HttpStatusCode.NotFound) })
            try {
                fixture.cache().use { cache ->
                    val proxy = AndroidMediaProxy(context, base)
                    val failure = assertFailsWith<IllegalArgumentException> {
                        proxy.image(source, http, cache, push = true)
                    }
                    assertEquals("Image media-proxy is unavailable", failure.message)
                }
                assertEquals(0, fixture.server.requestCount)
            } finally { http.close() }
        }
    }

}
