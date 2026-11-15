package zoo

import kotlin.test.*

class MobileDestinationTest {
    @Test
    fun nestedScreensKeepTheirParentDestinationSelected() {
        assertEquals(MobileDestination.Home, mobileDestination("/home/posts/one?thread=true"))
        assertEquals(MobileDestination.Channels, mobileDestination("/channels/one?members=true"))
        assertEquals(MobileDestination.Channels, mobileDestination("/"))
        assertEquals(MobileDestination.Notifications, mobileDestination("/notifications"))
        assertEquals(MobileDestination.Bubble, mobileDestination("/bubble"))
        assertEquals(MobileDestination.Settings, mobileDestination("/settings?channel=one"))
        assertNull(mobileDestination("/apps-authorize"))
    }
}
