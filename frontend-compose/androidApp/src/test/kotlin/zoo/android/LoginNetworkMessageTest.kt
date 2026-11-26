package zoo.android

import java.io.IOException
import java.net.SocketTimeoutException
import java.net.UnknownHostException
import javax.net.ssl.SSLHandshakeException
import kotlin.test.Test
import kotlin.test.assertEquals
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
}
