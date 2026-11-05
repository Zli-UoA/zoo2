package zoo.android

import io.ktor.client.*
import io.ktor.client.engine.mock.*
import io.ktor.http.*
import kotlinx.coroutines.test.runTest
import kotlin.test.*

class PublicImagesTest {
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
}
