package zoo.android

import kotlin.test.*
import zoo.*

class NotificationInboxTest {
    @Test fun suppressesHistoryDuplicatesAndReadNotifications() {
        val inbox = NotificationInbox()
        val old = obj("id" to "old", "is_read" to false)
        val next = obj("id" to "new", "is_read" to false)
        val read = obj("id" to "read", "is_read" to true)
        assertTrue(inbox.update(listOf(old, read)).isEmpty())
        assertEquals(listOf(next), inbox.update(listOf(old, next, next, read)))
        assertTrue(inbox.update(listOf(next, old)).isEmpty())
        assertTrue(inbox.update(listOf(obj("id" to "read", "is_read" to false))).isEmpty())
        assertTrue(inbox.update(emptyList()).isEmpty())
        assertTrue(inbox.update(listOf(old)).isEmpty())
    }
}
