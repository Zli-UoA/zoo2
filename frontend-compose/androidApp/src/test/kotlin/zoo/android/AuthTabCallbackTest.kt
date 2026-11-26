package zoo.android

import androidx.browser.auth.AuthTabIntent
import kotlin.test.*
import zoo.crypto.ReauthCallback
import zoo.crypto.ReauthException
import zoo.crypto.ReauthRequest

class AuthTabCallbackTest {
    private val request = ReauthRequest.create(
        authorizationEndpoint = "https://accounts.example/authorize",
        clientId = "zoo-android",
        redirectUri = "https://zoo.example/android/reauth-callback",
        resource = "",
    )

    @Test
    fun successfulResultRequiresMatchingRedirectAndState() {
        val redirect = "${request.redirectUri}?code=example&state=${request.state}"
        assertEquals(ReauthCallback.Code("example"), authTabCallback(AuthTabIntent.RESULT_OK, redirect, request))
        for (invalid in listOf(null, redirect.replace(request.state, "old-request"),
            redirect.replace("zoo.example", "other.example"), redirect.replace("reauth-callback", "other"))) {
            val failure = assertFailsWith<ReauthException> { authTabCallback(AuthTabIntent.RESULT_OK, invalid, request) }
            assertEquals(ReauthException.Reason.INVALID_CALLBACK, failure.reason)
        }
    }

    @Test
    fun canceledOrUnverifiedResultCannotYieldACode() {
        val redirect = "${request.redirectUri}?code=example&state=${request.state}"
        for (result in listOf(AuthTabIntent.RESULT_CANCELED, AuthTabIntent.RESULT_VERIFICATION_FAILED,
            AuthTabIntent.RESULT_VERIFICATION_TIMED_OUT)) {
            val failure = assertFailsWith<ReauthException> { authTabCallback(result, redirect, request) }
            assertEquals(
                if (result == AuthTabIntent.RESULT_CANCELED) ReauthException.Reason.CANCELLED
                else ReauthException.Reason.CALLBACK_VERIFICATION_FAILED,
                failure.reason,
            )
        }
    }
}
