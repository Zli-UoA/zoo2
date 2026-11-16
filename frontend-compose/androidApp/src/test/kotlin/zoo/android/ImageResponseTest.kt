package zoo.android

import io.ktor.client.HttpClient
import io.ktor.client.engine.mock.MockEngine
import io.ktor.client.engine.mock.respond
import io.ktor.client.request.prepareGet
import io.ktor.http.*
import kotlinx.coroutines.test.runTest
import kotlin.test.*

class ImageResponseTest {
    @Test
    fun acceptsExactlyTheLimitWithoutAnAccountHeader() = runTest {
        var requests = 0
        val http = HttpClient(MockEngine { request ->
            requests++
            assertNull(request.headers[HttpHeaders.Authorization])
            respond("12345678")
        })
        try {
            val bytes = http.prepareGet("https://media.example/image").execute {
                readImageResponse(it, limit = 8)
            }
            assertEquals("12345678", bytes.decodeToString())
            assertEquals(1, requests)
        } finally { http.close() }
    }

    @Test
    fun rejectsOversizedBodiesEvenWithoutContentLength() = runTest {
        val http = HttpClient(MockEngine { respond("123456789") })
        try {
            assertFailsWith<IllegalArgumentException> {
                http.prepareGet("https://media.example/image").execute {
                    readImageResponse(it, limit = 8)
                }
            }
        } finally { http.close() }
    }

    @Test
    fun rejectsAnOversizedDeclaredLengthAndHttpErrors() = runTest {
        val http = HttpClient(MockEngine { request ->
            if (request.url.encodedPath == "/large")
                respond("", headers = headersOf(HttpHeaders.ContentLength, "100"))
            else respond("failure", status = HttpStatusCode.ServiceUnavailable)
        })
        try {
            assertFailsWith<IllegalArgumentException> {
                http.prepareGet("https://media.example/large").execute { readImageResponse(it, 8) }
            }
            assertFailsWith<IllegalStateException> {
                http.prepareGet("https://media.example/error").execute { readImageResponse(it, 8) }
            }
        } finally { http.close() }
    }
}
