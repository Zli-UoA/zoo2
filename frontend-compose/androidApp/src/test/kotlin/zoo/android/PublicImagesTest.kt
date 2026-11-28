package zoo.android

import io.ktor.client.*
import io.ktor.client.engine.mock.*
import io.ktor.http.*
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.test.runTest
import kotlin.test.*

class PublicImagesTest {
    @Test fun reportsHttpFailure() = runTest {
        val http = HttpClient(MockEngine { respond("", HttpStatusCode.GatewayTimeout) })
        try {
            assertEquals(504, assertFailsWith<ImageHttpFailure> { loadPublicImage(http, "https://proxy.example/image") }.status)
        } finally { http.close() }
    }

    @Test fun queryOnlyRedirectKeepsTheImagePath() = runTest {
        val urls = mutableListOf<String>()
        val http = HttpClient(MockEngine { request ->
            urls += request.url.toString()
            if (urls.size == 1) respond("", HttpStatusCode.Found, headersOf(HttpHeaders.Location, "?token=new"))
            else respond("image")
        }) { followRedirects = false }
        try {
            assertEquals("image", loadPublicImage(http, "https://cdn.example/images/avatar.png?token=old").decodeToString())
            assertEquals(listOf("https://cdn.example/images/avatar.png?token=old",
                "https://cdn.example/images/avatar.png?token=new"), urls)
        } finally { http.close() }
    }

    @Test fun keepsStreamingSizeLimitAfterRedirects() = runTest {
        val http = HttpClient(MockEngine { request ->
            if (request.url.encodedPath == "/redirect")
                respond("", HttpStatusCode.Found, headersOf(HttpHeaders.Location, "/large"))
            else respond("123456789")
        }) { followRedirects = false }
        try {
            assertFailsWith<IllegalArgumentException> {
                loadPublicImage(http, "https://zoo.example/redirect", limit = 8)
            }
        } finally { http.close() }
    }

    @Test fun followsPublicHttpsRedirectsWithoutAuthentication() = runTest {
        val urls = mutableListOf<String>()
        val http = HttpClient(MockEngine { request ->
            urls += request.url.toString()
            assertNull(request.headers[HttpHeaders.Authorization])
            if (urls.size == 1) respond("", HttpStatusCode.Found, headersOf(HttpHeaders.Location, "https://cdn.example/image.png"))
            else respond(byteArrayOf(1, 2, 3))
        }) { followRedirects = false }
        try {
            assertContentEquals(byteArrayOf(1, 2, 3), loadPublicImage(http, "https://zoo.example/image"))
            assertEquals(listOf("https://zoo.example/image", "https://cdn.example/image.png"), urls)
        } finally { http.close() }
    }

    @Test fun rejectsDowngradesAndRedirectLoops() = runTest {
        var location = "http://cdn.example/image"
        var requests = 0
        val http = HttpClient(MockEngine {
            requests++
            respond("", HttpStatusCode.Found, headersOf(HttpHeaders.Location, location))
        }) { followRedirects = false }
        try {
            assertFailsWith<IllegalArgumentException> { loadPublicImage(http, "https://zoo.example/image") }
            assertEquals(1, requests)
            location = "/image"
            requests = 0
            assertFailsWith<IllegalStateException> { loadPublicImage(http, "https://zoo.example/image") }
            assertEquals(6, requests)
        } finally { http.close() }
    }
    @Test fun rejectsDedicatedHostDowngradeBeforeProxyRouting() = runBlocking {
        val start = "https://emoji.zliusercontent.com/image.png"
        val requested = mutableListOf<String>()
        val http = HttpClient(MockEngine { request ->
            requested += request.url.toString()
            respond("", HttpStatusCode.Found, headersOf(HttpHeaders.Location, "http://cdn.example/image.png"))
        }) { followRedirects = false }
        try {
            assertFailsWith<IllegalArgumentException> {
                loadPublicImage(http, start, redirectRoute = { error("HTTP redirect reached the proxy router") })
            }
            assertEquals(listOf(start), requested)
        } finally { http.close() }
    }

    @Test fun routesEachDedicatedImageRedirectBeforeConnecting() = runBlocking {
        val workspace = "https://zoo.example/image"
        val start = "https://emoji.zli.works/image"
        val cdn = "https://emoji.zliusercontent.com/image"
        val external = "https://outside.example/image"
        val proxyOrigin = "https://proxy.example/"
        val urls = mutableListOf<String>()
        val http = HttpClient(MockEngine { request ->
            val url = request.url.toString()
            urls += url
            when (url) {
                workspace -> respond("", HttpStatusCode.Found, headersOf(HttpHeaders.Location, start))
                start -> respond("", HttpStatusCode.Found, headersOf(HttpHeaders.Location, cdn))
                cdn -> respond("", HttpStatusCode.Found, headersOf(HttpHeaders.Location, external))
                else -> {
                    assertEquals("proxy.example", request.url.host)
                    assertEquals(external, request.url.parameters["url"])
                    respond("image")
                }
            }
        }) { followRedirects = false }
        try {
            val result = loadPublicImage(http, workspace, redirectRoute = { image ->
                assertNotNull(imageRequest("https://zoo.example", image, proxyOrigin))
            })
            assertEquals("image", result.decodeToString())
            assertEquals(4, urls.size)
            assertEquals(listOf(workspace, start, cdn), urls.take(3))
        } finally { http.close() }
    }

    @Test fun doesNotFollowMediaProxyRedirectToTheOriginalHost() = runTest {
        val urls = mutableListOf<String>()
        val http = HttpClient(MockEngine { request ->
            urls += request.url.toString()
            respond("", HttpStatusCode.Found, headersOf(HttpHeaders.Location, "https://external.example/image"))
        }) { followRedirects = false }
        try {
            assertFailsWith<IllegalArgumentException> {
                loadPublicImage(http, "https://proxy.example/image", restrictOrigin = true)
            }
            assertEquals(listOf("https://proxy.example/image"), urls)
        } finally { http.close() }
    }

}
