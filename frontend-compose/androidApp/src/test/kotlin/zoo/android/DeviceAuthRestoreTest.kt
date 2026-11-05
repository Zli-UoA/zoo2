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
            assertEquals(listOf<String?>(null, "rotated"), saved)
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
            assertEquals(listOf<String?>(null, "saved"), saved)
            assertEquals("Bearer access", auth.headers()["Authorization"])
        } finally { http.close() }
    }

    @Test
    fun invalidGrantOrServerFailureAfterSendingRequiresFreshLogin() = runTest {
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
                assertTrue(error.requiresLogin)
                assertEquals(listOf<String?>(null, null), saved)
                assertEquals(1, expired)
                assertFailsWith<IllegalStateException> { auth.headers() }
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
                assertEquals(listOf<String?>(null, null), saved)
                assertEquals(0, expired)
                assertFailsWith<IllegalStateException> { auth.headers() }
            } finally { http.close() }
        }
    }
    @Test
    fun cancellationAfterSendingWaitsForRotationAndSavesItsResult() = runTest {
        val saved = mutableListOf<String?>()
        val started = CompletableDeferred<Unit>()
        val release = CompletableDeferred<Unit>()
        val sent = mutableListOf<String?>()
        val http = HttpClient(MockEngine { request ->
            if (request.url.encodedPath.endsWith("openid-configuration")) respond(discovery)
            else {
                sent += (request.body as FormDataContent).formData["refresh_token"]
                started.complete(Unit)
                release.await()
                respond("""{"access_token":"access","refresh_token":"rotated","expires_in":3600}""")
            }
        })
        try {
            val auth = DeviceAuth(http, saved::add)
            val job = launch { auth.restore("https://id.example", "client", "", "saved") }
            started.await()
            job.cancel()
            runCurrent()
            val waitedForResponse = !job.isCompleted
            release.complete(Unit)
            job.join()
            assertTrue(waitedForResponse)
            assertEquals(listOf<String?>(null, "rotated"), saved)
            auth.close()
            val next = DeviceAuth(http, saved::add)
            next.restore("https://id.example", "client", "", saved.last()!!)
            assertEquals(listOf<String?>("saved", "rotated"), sent)
        } finally { release.complete(Unit); http.close() }
    }

    @Test
    fun lostResponseAndFailedSaveCannotLeaveAReusableOldToken() = runTest {
        for (loseResponse in listOf(true, false)) {
            var stored: String? = "saved"
            var requests = 0
            var expired = 0
            val http = HttpClient(MockEngine { request ->
                if (request.url.encodedPath.endsWith("openid-configuration")) respond(discovery)
                else {
                    requests++
                    assertNull(stored, "Remove the old token durably before sending it")
                    if (loseResponse) throw java.io.IOException("response lost after issuing a new token")
                    respond("""{"access_token":"access","refresh_token":"rotated","expires_in":3600}""")
                }
            })
            try {
                val auth = DeviceAuth(http, persistRefresh = {
                    if (it != null) throw LoginFailure("test disk failure")
                    stored = null
                }, onExpired = { expired++ })
                assertTrue(assertFailsWith<LoginFailure> {
                    auth.restore("https://id.example", "client", "", "saved")
                }.requiresLogin)
                assertNull(stored)
                assertEquals(1, expired)
                assertFailsWith<IllegalStateException> { auth.headers() }
                assertEquals(1, requests, "Never submit the used refresh token again")
            } finally { http.close() }
        }
    }

    @Test
    fun failureToRemoveOldTokenAbortsBeforeTheRefreshRequest() = runTest {
        var refreshes = 0
        val http = HttpClient(MockEngine { request ->
            if (request.url.encodedPath.endsWith("openid-configuration")) respond(discovery)
            else { refreshes++; respond("{}") }
        })
        try {
            val auth = DeviceAuth(http, consumeRefresh = { throw LoginFailure("test remove failure") })
            assertFailsWith<LoginFailure> { auth.restore("https://id.example", "client", "", "saved") }
            assertEquals(0, refreshes)
        } finally { http.close() }
    }

    @Test
    fun connectionFailuresDuringRefreshKeepTheSessionAndRetryTheSameUnusedToken() = runTest {
        val failures = listOf(
            java.net.UnknownHostException("offline"),
            java.net.ConnectException("connection refused"),
            io.ktor.client.network.sockets.ConnectTimeoutException("connect timeout"),
            java.io.IOException("wrapped", java.net.UnknownHostException("offline")),
        )
        for (failure in failures) {
            var stored: String? = "saved"
            var expired = 0
            var offline = false
            val sent = mutableListOf<String?>()
            val http = HttpClient(MockEngine { request ->
                if (request.url.encodedPath.endsWith("openid-configuration")) respond(discovery)
                else {
                    assertNull(stored)
                    sent += (request.body as FormDataContent).formData["refresh_token"]
                    if (offline) throw failure
                    respond("""{"access_token":"access","refresh_token":"rotated","expires_in":1}""")
                }
            })
            try {
                val auth = DeviceAuth(http, persistRefresh = { stored = it }, onExpired = { expired++ })
                auth.restore("https://id.example", "client", "", "saved")
                offline = true
                repeat(2) {
                    assertFalse(assertFailsWith<LoginFailure> { auth.headers() }.requiresLogin)
                    assertEquals("rotated", stored)
                    assertEquals(0, expired)
                }
                offline = false
                assertEquals("Bearer access", auth.headers()["Authorization"])
                assertEquals(listOf<String?>("saved", "rotated", "rotated", "rotated"), sent)
            } finally { http.close() }
        }
    }

    @Test
    fun failedRecoverySaveAndLogoutDuringConnectionFailureCannotRestoreTheToken() = runTest {
        for (logout in listOf(false, true)) {
            var stored: String? = "saved"
            var expired = 0
            lateinit var auth: DeviceAuth
            val http = HttpClient(MockEngine { request ->
                if (request.url.encodedPath.endsWith("openid-configuration")) respond(discovery)
                else {
                    assertNull(stored)
                    if (logout) auth.forget()
                    throw java.net.ConnectException("offline")
                }
            })
            try {
                auth = DeviceAuth(http, persistRefresh = {
                    if (it != null) error("cannot save recovery token")
                    stored = null
                }, onExpired = { expired++ })
                assertTrue(assertFailsWith<LoginFailure> {
                    auth.restore("https://id.example", "client", "", "saved")
                }.requiresLogin)
                assertNull(stored)
                assertEquals(if (logout) 0 else 1, expired)
                assertFailsWith<IllegalStateException> { auth.headers() }
            } finally { http.close() }
        }
    }

    @Test
    fun readTimeoutAfterSendingCannotRestoreTheUsedToken() = runTest {
        var stored: String? = "saved"
        val http = HttpClient(MockEngine { request ->
            if (request.url.encodedPath.endsWith("openid-configuration")) respond(discovery)
            else throw java.net.SocketTimeoutException("response read timeout")
        })
        try {
            val auth = DeviceAuth(http, persistRefresh = { stored = it })
            assertTrue(assertFailsWith<LoginFailure> {
                auth.restore("https://id.example", "client", "", "saved")
            }.requiresLogin)
            assertNull(stored)
        } finally { http.close() }
    }

}
