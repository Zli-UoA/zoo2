package zoo.android

import kotlin.test.*
import zoo.*

class NotificationReplyTest {
    private val notification = obj("channel_id" to "channel", "message_id" to "target")

    @Test fun repliesToTheExistingThreadRoot() {
        val reply = obj("id" to "target", "channel_id" to "channel", "parent_id" to "root")
        assertEquals("root", notificationReplyParent(notification, listOf(reply)))
    }

    @Test fun startsAThreadAtATopLevelMessage() {
        val root = obj("id" to "target", "channel_id" to "channel", "parent_id" to null)
        assertEquals("target", notificationReplyParent(notification, listOf(root)))
    }

    @Test fun doesNotGuessWhenTheTargetIsMissingDeletedOrFromAnotherChannel() {
        assertNull(notificationReplyParent(notification, emptyList()))
        val wrongChannel = obj("id" to "target", "channel_id" to "other", "parent_id" to "root")
        assertNull(notificationReplyParent(notification, listOf(wrongChannel)))
        val deleted = obj("id" to "target", "channel_id" to "channel", "deleted_at" to "2026-11-15")
        assertNull(notificationReplyParent(notification, listOf(deleted)))
    }
}
