package zoo.android

import java.io.IOException
import java.net.SocketTimeoutException
import java.net.UnknownHostException
import javax.net.ssl.SSLHandshakeException
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNull
import kotlin.test.assertTrue

class LoginNetworkMessageTest {
    @Test
    fun distinguishesPortalDnsCertificateAndTimeoutWithoutExposingExceptionDetails() {
        assertEquals(wifiSignInMessage, loginNetworkMessage(IOException(), captivePortal = true))
        assertTrue(loginNetworkMessage(IOException(UnknownHostException("internal hostname")), false)!!.contains("名前を解決"))
        assertTrue(loginNetworkMessage(SSLHandshakeException("certificate details"), false)!!.contains("証明書"))
        assertTrue(loginNetworkMessage(SocketTimeoutException(), false)!!.contains("時間内"))
        assertNull(loginNetworkMessage(IllegalStateException(), false))
    }

    @Test
    fun diagnosticLogShowsTheFailedStageAndOmitsRawExceptionText() {
        val stages = listOf(LoginStage.Network, LoginStage.Discovery)
        val http = loginDiagnosticLog(stages, LoginFailure("server response secret-token (HTTP 503)"))
        assertTrue(http.contains("認証先の設定取得 — 失敗"))
        assertTrue(http.contains("HTTP 503"))
        assertFalse(http.contains("secret-token"))
        val dns = loginDiagnosticLog(stages, UnknownHostException("private.example"))
        assertTrue(dns.contains("DNS"))
        assertFalse(dns.contains("private.example"))
    }
}
