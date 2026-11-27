package zoo.android

import java.time.Instant
import java.time.ZoneId
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import zoo.android.ui.TimelineItem
import zoo.obj
import zoo.ChannelReadPosition
import zoo.PageState
import zoo.timestampMillis

class ConversationDividersTest {
    @Test
    fun unreadStartRequiresCoverageOfTheReadBoundaryOrTheBeginningOfHistory() {
        val timestamp = "2026-11-01T15:00:00Z"
        val page = PageState(loaded = true, hasOlder = true, oldest = timestamp to "oldest")
        val covered = ChannelReadPosition(checkNotNull(timestampMillis(timestamp)), true)
        val older = covered.copy(timestamp = covered.timestamp - 1)
        assertTrue(unreadHistoryOutsideWindow(null, covered))
        assertTrue(unreadHistoryOutsideWindow(PageState(), covered))
        assertFalse(unreadHistoryOutsideWindow(null, covered.copy(hasUnread = false)))
        assertFalse(unreadHistoryOutsideWindow(page, covered))
        assertTrue(unreadHistoryOutsideWindow(page, older))
        assertTrue(unreadHistoryOutsideWindow(page, older.copy(timestamp = Long.MIN_VALUE)))
        assertFalse(unreadHistoryOutsideWindow(page.copy(hasOlder = false), older))
    }

    @Test
    fun groupsMessagesByLocalDateAndKeepsTheUnreadMessageKey() {
        val entries = listOf("one", "two", "three").map { TimelineItem(it, "Author", "Body", "") }
        val rows = mapOf(
            "one" to obj("created_at" to "2026-11-01T14:59:00Z"),
            "two" to obj("created_at" to "2026-11-01T15:00:00Z"),
            "three" to obj("created_at" to "2026-11-01T20:00:00Z"),
        )
        fun localDay(timestamp: String) =
            Instant.parse(timestamp).atZone(ZoneId.of("Asia/Tokyo")).toLocalDate().toString()

        val decorated = withConversationDividers(entries, rows, "two", ::localDay, ::localDay, "New messages")

        assertEquals(entries.map { it.entryKey }, decorated.map { it.entryKey })
        assertEquals(listOf("2026-11-01", "2026-11-02", null), decorated.map { it.dateDivider })
        assertEquals(listOf(null, "New messages", null), decorated.map { it.unreadDivider })
    }
}
