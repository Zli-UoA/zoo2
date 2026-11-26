package zoo.android

import io.ktor.http.Url
import io.ktor.http.HttpHeaders
import io.ktor.http.HttpStatusCode
import io.ktor.http.headersOf
import io.ktor.client.HttpClient
import io.ktor.client.engine.mock.MockEngine
import io.ktor.client.engine.mock.respond
import kotlinx.coroutines.runBlocking
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
            if (++requests == 1) respond("", HttpStatusCode.NotFound)
            else respond("/proxy", headers = headersOf(HttpHeaders.ContentType, "text/plain"))
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
        } finally { http.close() }
    }

}
