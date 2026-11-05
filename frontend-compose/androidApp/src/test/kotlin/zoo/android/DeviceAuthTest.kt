package zoo.android

import io.ktor.client.*
import io.ktor.client.engine.mock.*
import io.ktor.client.request.forms.*
import io.ktor.http.*
import kotlin.test.*
import kotlinx.coroutines.test.*

class DeviceAuthTest {
    private fun discovery(issuer: String = "https://id.example") =
        """{"issuer":"$issuer","device_authorization_endpoint":"https://id.example/device","token_endpoint":"https://id.example/token"}"""

    private val device =
        """{"device_code":"secret-device-code","user_code":"ABCD","verification_uri":"https://id.example/verify","expires_in":300,"interval":1}"""

    @Test
    fun pendingAndSlowDownRespectPollingIntervalAndSendResource() = runTest {
        var attempts = 0
        val pollTimes = mutableListOf<Long>()
        val http =
            HttpClient(
                MockEngine { request ->
                    val response =
                        when (request.url.encodedPath) {
                            "/.well-known/openid-configuration" -> discovery()
                            "/device" -> {
                                val form = (request.body as FormDataContent).formData
                                assertEquals("android-client", form["client_id"])
                                assertEquals("https://zoo.example", form["resource"])
                                device
                            }
                            else -> {
                                pollTimes.add(testScheduler.currentTime)
                                val form = (request.body as FormDataContent).formData
                                assertEquals("secret-device-code", form["device_code"])
                                assertEquals(
                                    "urn:ietf:params:oauth:grant-type:device_code",
                                    form["grant_type"],
                                )
                                when (attempts++) {
                                    0 -> """{"error":"authorization_pending"}"""
                                    1 -> """{"error":"slow_down"}"""
                                    else ->
                                        """{"access_token":"access","refresh_token":"refresh","expires_in":3600}"""
                                }
                            }
                        }
                    respond(
                        response,
                        status =
                            if (response.contains("\"error\"")) HttpStatusCode.BadRequest
                            else HttpStatusCode.OK,
                        headers = headersOf(HttpHeaders.ContentType, "application/json"),
                    )
                }
            )
        try {
            val auth = DeviceAuth(http)
            auth.login("https://id.example", "android-client", "https://zoo.example") { code, uri ->
                assertEquals("ABCD", code)
                assertEquals("https://id.example/verify", uri)
            }
            assertEquals(listOf(1000L, 2000L, 8000L), pollTimes)
            assertEquals(mapOf("Authorization" to "Bearer access"), auth.headers())
        } finally {
            http.close()
        }
    }

    @Test
    fun rejectsMismatchedIssuerBeforeSendingClientOrDeviceCode() = runTest {
        var requests = 0
        val http =
            HttpClient(
                MockEngine {
                    requests++
                    respond(discovery("https://wrong.example"))
                }
            )
        try {
            assertFailsWith<IllegalArgumentException> {
                DeviceAuth(http).login("https://id.example", "client", "") { _, _ -> fail() }
            }
            assertEquals(1, requests)
        } finally {
            http.close()
        }
    }

    @Test
    fun rejectsInsecureIssuerWithoutNetwork() = runTest {
        val http = HttpClient(MockEngine { fail("No request expected") })
        try {
            assertFailsWith<IllegalArgumentException> {
                DeviceAuth(http).login("http://id.example", "client", "") { _, _ -> fail() }
            }
        } finally {
            http.close()
        }
    }

    @Test
    fun refreshesBeforeExpiryAndUsesRotatedRefreshToken() = runTest {
        val refreshes = mutableListOf<String?>()
        val http =
            HttpClient(
                MockEngine { request ->
                    val response =
                        when (request.url.encodedPath) {
                            "/.well-known/openid-configuration" -> discovery()
                            "/device" -> device
                            else -> {
                                val form = (request.body as FormDataContent).formData
                                if (form["grant_type"] == "refresh_token") {
                                    refreshes.add(form["refresh_token"])
                                    """{"access_token":"new","refresh_token":"rotated","expires_in":1}"""
                                } else
                                    """{"access_token":"old","refresh_token":"initial","expires_in":1}"""
                            }
                        }
                    respond(response)
                }
            )
        try {
            val auth = DeviceAuth(http)
            auth.login("https://id.example", "client", "") { _, _ -> }
            assertEquals("Bearer new", auth.headers()["Authorization"])
            auth.headers()
            assertEquals(listOf<String?>("initial", "rotated"), refreshes)
        } finally {
            http.close()
        }
    }

    @Test
    fun deviceAuthorizationErrorsIdentifyTheConfigurationWithoutEchoingServerDetails() = runTest {
        for ((code, hint) in
            listOf(
                "unauthorized_client" to "デバイスコード認証",
                "invalid_target" to "Resource",
                "invalid_scope" to "offline_access",
            )) {
            val http =
                HttpClient(
                    MockEngine { request ->
                        if (request.url.encodedPath == "/.well-known/openid-configuration")
                            respond(discovery())
                        else
                            respond(
                                """{"error":"$code","error_description":"sensitive server text"}""",
                                HttpStatusCode.BadRequest,
                            )
                    }
                )
            try {
                val failure =
                    assertFailsWith<LoginFailure> {
                        DeviceAuth(http).login("https://id.example", "client", "") { _, _ ->
                            fail("No device code expected")
                        }
                    }
                assertContains(failure.message.orEmpty(), "確認コードの発行")
                assertContains(failure.message.orEmpty(), "HTTP 400")
                assertContains(failure.message.orEmpty(), code)
                assertContains(failure.message.orEmpty(), hint)
                assertFalse(failure.message.orEmpty().contains("sensitive server text"))
            } finally {
                http.close()
            }
        }
    }

    @Test
    fun tokenExchangeFailureKeepsTheStageAndOAuthCode() = runTest {
        val http =
            HttpClient(
                MockEngine { request ->
                    when (request.url.encodedPath) {
                        "/.well-known/openid-configuration" -> respond(discovery())
                        "/device" -> respond(device)
                        else ->
                            respond(
                                """{"error":"invalid_client","error_description":"hidden token response"}""",
                                HttpStatusCode.Unauthorized,
                            )
                    }
                }
            )
        try {
            val failure =
                assertFailsWith<LoginFailure> {
                    DeviceAuth(http).login("https://id.example", "client", "") { _, _ -> }
                }
            assertContains(failure.message.orEmpty(), "トークンの取得")
            assertContains(failure.message.orEmpty(), "HTTP 401 / invalid_client")
            assertFalse(failure.message.orEmpty().contains("hidden token response"))
        } finally {
            http.close()
        }
    }

    @Test
    fun unexpectedServerResponsesDoNotAppearInTheLoginMessage() = runTest {
        for (body in
            listOf(
                """{"error":"sensitive_unknown_code","error_description":"sensitive details"}""",
                "<html>sensitive details</html>",
            )) {
            val http =
                HttpClient(
                    MockEngine { request ->
                        if (request.url.encodedPath == "/.well-known/openid-configuration")
                            respond(discovery())
                        else respond(body, HttpStatusCode.Forbidden)
                    }
                )
            try {
                val failure =
                    assertFailsWith<LoginFailure> {
                        DeviceAuth(http).login("https://id.example", "client", "") { _, _ ->
                            fail()
                        }
                    }
                assertContains(failure.message.orEmpty(), "HTTP 403")
                assertFalse(failure.message.orEmpty().contains("sensitive"))
            } finally {
                http.close()
            }
        }
    }
}
