package zoo

import java.net.InetAddress
import kotlin.test.Test
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class PreviewAddressTest {
    @Test fun rejectsPrivateAndLocalPeerMediaAddresses() {
        for (address in listOf("127.0.0.1", "10.0.0.1", "192.168.0.1", "169.254.169.254", "100.64.0.1", "::1", "fc00::1", "fe80::1")) {
            assertFalse(publicPreviewAddress(InetAddress.getByName(address)), address)
        }
        assertTrue(publicPreviewAddress(InetAddress.getByName("8.8.8.8")))
        assertTrue(publicPreviewAddress(InetAddress.getByName("2606:4700:4700::1111")))
    }
}
