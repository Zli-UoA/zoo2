package zoo.android

import io.ktor.client.*
import io.ktor.client.engine.mock.*
import io.ktor.client.request.forms.*
import io.ktor.http.*
import kotlin.test.*
import kotlinx.coroutines.test.*
import kotlinx.serialization.json.*

class DeviceAuthTest {
    private fun discovery(issuer: String = "https://id.example") =
        """{"issuer":"$issuer","token_endpoint":"https://id.example/token"}"""

    private val connection =
        LoginConnection("https://zoo.example", "https://id.example", "client", "")

    @Test
    fun rejectsMismatchedIssuerBeforeSendingClientOrRefreshToken() = runTest {
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
                DeviceAuth(http).restore("https://id.example", "client", "", "saved")
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
                DeviceAuth(http).restore("http://id.example", "client", "", "saved")
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
                    val form = (request.body as FormDataContent).formData
                    assertEquals("refresh_token", form["grant_type"])
                    refreshes.add(form["refresh_token"])
                    respond("""{"access_token":"new","refresh_token":"rotated","expires_in":1}""")
                }
            )
        try {
            val auth = DeviceAuth(http)
            // The browser sign-in result: tokens already exchanged by the authorization code flow.
            auth.acceptBrowserLogin(
                connection,
                BrowserAuthorization(
                    "https://id.example/token",
                    buildJsonObject {
                        put("access_token", "old")
                        put("refresh_token", "initial")
                        put("expires_in", 1)
                    },
                ),
            )
            assertEquals("Bearer new", auth.headers()["Authorization"])
            auth.headers()
            assertEquals(listOf<String?>("initial", "rotated"), refreshes)
        } finally {
            http.close()
        }
    }

    @Test
    fun tokenExchangeFailureKeepsTheStageAndOAuthCode() = runTest {
        val http =
            HttpClient(
                MockEngine { request ->
                    when (request.url.encodedPath) {
                        "/.well-known/openid-configuration" -> respond(discovery())
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
                    DeviceAuth(http).restore("https://id.example", "client", "", "saved")
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
                        DeviceAuth(http).restore("https://id.example", "client", "", "saved")
                    }
                assertContains(failure.message.orEmpty(), "HTTP 403")
                assertFalse(failure.message.orEmpty().contains("sensitive"))
            } finally {
                http.close()
            }
        }
    }
}
