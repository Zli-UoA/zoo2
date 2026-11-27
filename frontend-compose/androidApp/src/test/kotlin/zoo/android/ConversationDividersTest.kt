package zoo.android

import java.time.Instant
import java.time.ZoneId
import org.junit.Assert.assertEquals
import org.junit.Test
import zoo.android.ui.TimelineItem
import zoo.obj

class ConversationDividersTest {
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
