package zoo.android

import io.ktor.client.*
import io.ktor.client.engine.mock.*
import io.ktor.client.request.forms.*
import io.ktor.http.*
import kotlinx.coroutines.*
import kotlinx.coroutines.test.*
import kotlin.test.*

class DeviceAuthRestoreTest {
    private val discovery = """{"issuer":"https://id.example","token_endpoint":"https://id.example/token"}"""

    @Test
    fun resumesWithoutDeviceGrantAndPersistsRotatedTokenBeforeReturningHeaders() = runTest {
        val saved = mutableListOf<String?>()
        var refreshes = 0
        val http = HttpClient(MockEngine { request ->
            if (request.url.encodedPath == "/.well-known/openid-configuration") respond(discovery)
            else {
                assertEquals("/token", request.url.encodedPath)
                val form = (request.body as FormDataContent).formData
                assertEquals("refresh_token", form["grant_type"])
                assertEquals("client", form["client_id"])
                assertEquals("https://zoo.example/api", form["resource"])
                assertEquals(if (refreshes++ == 0) "saved" else "rotated", form["refresh_token"])
                respond("""{"access_token":"access","refresh_token":"rotated","expires_in":1}""")
            }
        })
        try {
            val auth = DeviceAuth(http, saved::add)
            auth.restore("https://id.example", "client", "https://zoo.example/api", "saved")
            assertEquals(listOf<String?>("rotated"), saved)
            assertEquals("Bearer access", auth.headers()["Authorization"])
            assertEquals(2, refreshes)
        } finally { http.close() }
    }

    @Test
    fun responseWithoutRotatedTokenKeepsSavedRefreshToken() = runTest {
        val saved = mutableListOf<String?>()
        val http = HttpClient(MockEngine { request ->
            if (request.url.encodedPath.endsWith("openid-configuration")) respond(discovery)
            else respond("""{"access_token":"access","expires_in":3600}""")
        })
        try {
            val auth = DeviceAuth(http, saved::add)
            auth.restore("https://id.example", "client", "", "saved")
            assertEquals(listOf<String?>("saved"), saved)
            assertEquals("Bearer access", auth.headers()["Authorization"])
        } finally { http.close() }
    }

    @Test
    fun invalidGrantClearsSavedSessionButServerFailureDoesNot() = runTest {
        for (status in listOf(HttpStatusCode.BadRequest, HttpStatusCode.InternalServerError)) {
            val saved = mutableListOf<String?>()
            var expired = 0
            val http = HttpClient(MockEngine { request ->
                if (request.url.encodedPath.endsWith("openid-configuration")) respond(discovery)
                else respond("""{"error":"invalid_grant","error_description":"private response"}""", status)
            })
            try {
                val auth = DeviceAuth(http, saved::add) { expired++ }
                val error = assertFailsWith<LoginFailure> { auth.restore("https://id.example", "client", "", "saved") }
                assertFalse(error.message.orEmpty().contains("private response"))
                assertEquals(if (status == HttpStatusCode.BadRequest) listOf<String?>(null) else emptyList(), saved)
                assertEquals(if (status == HttpStatusCode.BadRequest) 1 else 0, expired)
            } finally { http.close() }
        }
    }

    @Test
    fun networkFailurePreservesSavedSessionForRetry() = runTest {
        val saved = mutableListOf<String?>()
        val http = HttpClient(MockEngine { throw java.io.IOException("test network failure") })
        try {
            assertFailsWith<java.io.IOException> { DeviceAuth(http, saved::add).restore("https://id.example", "client", "", "saved") }
            assertTrue(saved.isEmpty())
        } finally { http.close() }
    }

    @Test
    fun mismatchedDiscoveryDoesNotReceiveSavedRefreshToken() = runTest {
        var requests = 0
        val http = HttpClient(MockEngine {
            requests++
            respond(discovery.replace("https://id.example\"", "https://other.example\""))
        })
        try {
            assertFailsWith<IllegalArgumentException> { DeviceAuth(http).restore("https://id.example", "client", "", "saved") }
            assertEquals(1, requests)
        } finally { http.close() }
    }

    @Test
    fun lateResponseCannotResurrectOrClearAnotherLoginAfterLogout() = runTest {
        for (status in listOf(HttpStatusCode.OK, HttpStatusCode.BadRequest)) {
            val saved = mutableListOf<String?>()
            var expired = 0
            val started = CompletableDeferred<Unit>()
            val release = CompletableDeferred<Unit>()
            val http = HttpClient(MockEngine { request ->
                if (request.url.encodedPath.endsWith("openid-configuration")) respond(discovery)
                else {
                    started.complete(Unit)
                    release.await()
                    respond(if (status == HttpStatusCode.OK) """{"access_token":"late","refresh_token":"late","expires_in":3600}""" else """{"error":"invalid_grant"}""", status)
                }
            })
            try {
                val auth = DeviceAuth(http, saved::add) { expired++ }
                val attempt = async { runCatching { auth.restore("https://id.example", "client", "", "saved") } }
                started.await()
                auth.forget()
                release.complete(Unit)
                assertTrue(attempt.await().isFailure)
                assertEquals(listOf<String?>(null), saved)
                assertEquals(0, expired)
                assertFailsWith<IllegalStateException> { auth.headers() }
            } finally { http.close() }
        }
    }
}
