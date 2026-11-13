package zoo

import kotlin.test.*
import kotlinx.serialization.json.*

class ModelTest {
    @Test
    fun homeDetailsKeepTheHomePaneWithoutMatchingOtherRoutes() {
        assertTrue(isHomePath("/home?frontend=compose"))
        assertTrue(isHomePath("/home/posts/remote%2Fpost"))
        assertTrue(isHomePath("/home/users/remote%3Aactor?frontend=compose"))
        assertFalse(isHomePath("/home-other"))
        assertFalse(isHomePath("/channels/home"))
    }

    private fun row(value: String): Row = json.parseToJsonElement(value).jsonObject

    @Test
    fun channelNamesMatchUnicodeLettersMarksAndNumbers() {
        assertTrue(validChannelName("開発-𠮷-e\u0301-Ⅳ-１２"))
        assertTrue(validChannelName("𠮷".repeat(80)))
        assertFalse(validChannelName("𠮷".repeat(81)))
        assertFalse(validChannelName("hello😀"))
        assertFalse(validChannelName("bad name"))
        assertFalse(validChannelName("general"))
        assertFalse(validChannelName("bad\ud800"))
    }

    @Test
    fun electricStringBooleansDoNotTreatFalseAsTrue() {
        assertFalse(row("""{"is_private":"false"}""").flag("is_private"))
        assertTrue(row("""{"is_private":"true"}""").flag("is_private"))
        assertTrue(row("""{"is_private":true}""").flag("is_private"))
    }

    @Test
    fun deletedMessagesAndDirectRepliesStayInTheChannelFeed() {
        val root = row("""{"id":"r","parent_id":null,"created_at":"2026-10-01T00:00:00Z"}""")
        val reply = row("""{"id":"p","parent_id":null,"reply_to_message_id":"r","created_at":"2026-10-02T00:00:00Z"}""")
        val deleted = row("""{"id":"d","deleted_at":"2026-10-03T00:00:00Z"}""")
        assertEquals(listOf(root, reply), visibleMessages(listOf(deleted, reply, root)))
        assertEquals(listOf(reply), visibleMessages(listOf(deleted, reply, root), "r"))
    }

    @Test
    fun threadChannelsAreNotListedAsRegularChannels() {
        val regular = row("""{"id":"c","channel_name":"general"}""")
        val thread = row(
            """{"id":"t","channel_name":"thread-1","parent_channel_id":"c"}""")
        val deleted = row("""{"id":"d","deleted_at":"2026-11-01T00:00:00Z"}""")
        assertEquals(listOf(regular), visibleChannels(listOf(thread, deleted, regular)))
    }

    @Test
    fun membershipKeysAndPermissionsUseTheAuthenticatedIdentity() {
        val channel = row("""{"id":"c","created_user_id":"owner"}""")
        val membership = row("""{"user_id":"member","channel_id":"c"}""")
        assertEquals("member:c", rowKey(membership))
        assertTrue(isMember(listOf(membership), "c", "member"))
        assertFalse(isMember(listOf(membership), "c", "other"))
        assertTrue(canManage(channel, "owner", false))
        assertTrue(canManage(channel, "admin", true))
        assertFalse(canManage(channel, "member", false))
    }
}
