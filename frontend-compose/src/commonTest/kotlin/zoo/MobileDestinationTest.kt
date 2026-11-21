package zoo

import kotlin.test.*

class MobileDestinationTest {
    @Test
    fun nestedScreensKeepTheirParentDestinationSelected() {
        assertEquals(MobileDestination.Home, mobileDestination("/home/posts/one?thread=true"))
        assertEquals(MobileDestination.Channels, mobileDestination("/channels/one?members=true"))
        assertEquals(MobileDestination.Channels, mobileDestination("/"))
        assertEquals(MobileDestination.Channels, mobileDestination("/search?q=a&type=files"))
        assertEquals(MobileDestination.Notifications, mobileDestination("/notifications"))
        assertEquals(MobileDestination.Bubble, mobileDestination("/bubble"))
        assertEquals(MobileDestination.Settings, mobileDestination("/settings?channel=one"))
        assertNull(mobileDestination("/apps-authorize"))
    }
}
