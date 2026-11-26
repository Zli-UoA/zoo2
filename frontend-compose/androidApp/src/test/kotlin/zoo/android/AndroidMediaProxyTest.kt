package zoo.android

import io.ktor.http.Url
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

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
}
