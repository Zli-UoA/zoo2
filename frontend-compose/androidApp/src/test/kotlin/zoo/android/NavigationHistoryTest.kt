package zoo.android

import kotlin.test.*
import zoo.obj
import zoo.text

class NavigationHistoryTest {
    @Test
    fun backRestoresThePreviousEntryState() {
        val history = NavigationHistory()
        history.navigate("/channels/one", obj("source" to "channel"))
        history.navigate("/channels/one?members=true", obj("zooMembersParent" to "one"))
        assertEquals("one", history.current.state.text("zooMembersParent"))
        assertEquals("2", history.current.state.text("__TSR_index"))
        val channel = history.back()!!
        assertEquals("/channels/one", channel.path)
        assertEquals("channel", channel.state.text("source"))
        assertEquals("", channel.state.text("zooMembersParent"))
        assertEquals("1", channel.state.text("__TSR_index"))
        assertEquals("0", history.back()!!.state.text("__TSR_index"))
        assertNull(history.back())
    }

    @Test
    fun replaceAndRepeatedNavigationDoNotAddBackSteps() {
        val history = NavigationHistory()
        history.navigate("/channels/one")
        history.navigate("/channels/one")
        history.replace("/channels/two")
        assertEquals("1", history.current.state.text("__TSR_index"))
        assertEquals("/home", history.back()!!.path)
        assertNull(history.back())
    }

    @Test
    fun returningToTheSameSearchHasADifferentNavigationRevision() {
        val history = NavigationHistory("/search?q=photo")
        val requestRevision = history.revision
        history.navigate("/channels/one")
        history.back()
        assertEquals("/search?q=photo", history.current.path)
        assertNotEquals(requestRevision, history.revision)
    }

    @Test
    fun newEntryCannotSupplyItsOwnHistoryIndex() {
        val history = NavigationHistory()
        history.navigate("/channels/one", obj("__TSR_index" to 100))
        assertEquals("1", history.current.state.text("__TSR_index"))
    }
}
