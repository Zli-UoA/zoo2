package zoo.android

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertFalse
import kotlin.test.assertNull
import kotlin.test.assertTrue

class AndroidNavigationStateTest {
    @Test
    fun copyUrlResolvesNotePathAgainstTheServerOrigin() {
        assertEquals(
            "https://zoo.example/channels/one?note=two",
            androidCopyUrl("https://zoo.example/", "/channels/one?note=two"),
        )
        assertEquals(
            "https://zoo.example/channels/one",
            androidCopyUrl("https://unused.example", "https://zoo.example/channels/one"),
        )
        assertFailsWith<IllegalArgumentException> {
            androidCopyUrl("https://zoo.example", "javascript:alert(1)")
        }
    }

    @Test
    fun dirtyBackWaitsForExplicitDiscard() {
        val navigation = AndroidNavigationState("/home")
        navigation.navigate("/channels/one")
        navigation.setNoteDirty(true)

        assertTrue(navigation.back())
        assertEquals("/channels/one", navigation.path)
        assertEquals(AndroidNavigationRequest.Back, navigation.pendingRequest)

        navigation.cancelPendingRequest()
        assertEquals("/channels/one", navigation.path)
        assertNull(navigation.pendingRequest)

        navigation.back()
        navigation.confirmDiscard()
        assertEquals("/home", navigation.path)
        assertFalse(navigation.noteDirty)
    }

    @Test
    fun dirtyNavigationWaitsUntilDiscardIsConfirmed() {
        val navigation = AndroidNavigationState("/channels/one")
        navigation.setNoteDirty(true)

        navigation.navigate("/settings")
        assertEquals("/channels/one", navigation.path)
        assertEquals(AndroidNavigationRequest.Navigate("/settings"), navigation.pendingRequest)

        navigation.confirmDiscard()
        assertEquals("/settings", navigation.path)
        assertFalse(navigation.noteDirty)
    }

    @Test
    fun dirtyExitWaitsForExplicitDiscard() {
        val navigation = AndroidNavigationState("/home")
        navigation.setNoteDirty(true)

        assertTrue(navigation.back())
        assertEquals(AndroidNavigationRequest.Exit, navigation.pendingRequest)
        assertEquals("/home", navigation.path)

        assertEquals(AndroidNavigationRequest.Exit, navigation.confirmDiscard())
        assertFalse(navigation.noteDirty)
    }

    @Test
    fun dirtyLogoutWaitsForExplicitDiscard() {
        val navigation = AndroidNavigationState("/channels/one")
        navigation.setNoteDirty(true)

        navigation.requestLogout()
        assertEquals(AndroidNavigationRequest.Logout, navigation.pendingRequest)
        assertEquals("/channels/one", navigation.path)

        assertEquals(AndroidNavigationRequest.Logout, navigation.confirmDiscard())
        assertFalse(navigation.noteDirty)
    }
}
