package zoo

import kotlin.test.Test
import kotlin.test.assertEquals

class MessageTimestampTest {
    @Test
    fun timestampsFollowTheLocalCalendarInBothDirections() {
        val zone = kotlinx.datetime.TimeZone.of("Asia/Tokyo")
        val december = "2026-12-31 14:59:00"
        val january = "2026-12-31T15:01:00Z"
        val before = MessageCalendar(kotlinx.datetime.LocalDate(2026, 12, 31), zone)
        val after = before.copy(today = kotlinx.datetime.LocalDate(2027, 1, 1))
        assertEquals("23:59", before.format(december))
        assertEquals("12/30 01:05", before.format("2026-12-30T01:05:00+09:00"))
        assertEquals("2027/01/01 00:01", before.format(january))
        assertEquals("00:01", after.format(january))
        assertEquals("2026/12/31 23:59", after.format(december))
        assertEquals("invalid", after.format("invalid"))
    }
}
