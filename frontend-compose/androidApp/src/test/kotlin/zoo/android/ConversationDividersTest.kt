package zoo.android

import java.time.Instant
import java.time.ZoneId
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import zoo.android.ui.EntryDelivery
import zoo.android.ui.TimelineItem
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
        val entries = listOf("one" to "14:59:00", "two" to "15:00:00", "three" to "20:00:00").map { (id, time) ->
            TimelineItem(id, "Author", "Body", "", createdAt = "2026-11-01T${time}Z")
        }
        fun localDay(timestamp: String) =
            Instant.parse(timestamp).atZone(ZoneId.of("Asia/Tokyo")).toLocalDate().toString()

        val decorated = withConversationDividers(entries, "two", ::localDay, ::localDay, "New messages")

        assertEquals(entries.map { it.entryKey }, decorated.map { it.entryKey })
        assertEquals(listOf("2026-11-01", "2026-11-02", null), decorated.map { it.dateDivider })
        assertEquals(listOf(null, "New messages", null), decorated.map { it.unreadDivider })
    }

    @Test
    fun onlyAdjacentPostsByTheSameAuthorContinueAcrossAnInclusiveFiveMinuteGap() {
        val times = listOf(
            "14:48:00", "14:53:00", "14:58:01", "14:59:00", "15:00:00",
            "15:01:00", "15:02:00", "15:03:00", "15:04:00", "15:03:59",
        )
        val entries = times.mapIndexed { index, time ->
            TimelineItem(
                index.toString(), "Same display name", "Body", "",
                createdAt = "2026-11-01T${time}Z",
                authorId = if (index in 3..4) "different-author-with-same-display-name" else "author",
                divider = "Encryption".takeIf { index == 7 },
                delivery = EntryDelivery.Sending.takeIf { index == 8 },
            )
        }
        fun localDay(timestamp: String) =
            Instant.parse(timestamp).atZone(ZoneId.of("Asia/Tokyo")).toLocalDate().toString()

        val decorated = withConversationDividers(entries, "6", ::localDay, ::localDay, "New messages")

        assertEquals(listOf("1", "8"), decorated.filter { it.continuesGroup }.map { it.id })
    }
}
