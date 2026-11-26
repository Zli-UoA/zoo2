package zoo

import kotlin.test.Test
import kotlin.test.assertEquals

class MessageTimestampTest {
    private class Clock(var instant: String) : Platform {
        override fun now() = instant
        override fun day(timestamp: String) = timestamp.substringBefore(' ')
        override fun time(timestamp: String) = timestamp.substringAfter(' ')
        override suspend fun headers() = emptyMap<String, String>()
        override suspend fun readCache(key: String): String? = null
        override suspend fun writeCache(key: String, value: String) {}
        override fun uuid() = "test"
        override fun navigate(path: String) {}
        override fun action(name: String) {}
        override fun translate(key: String, args: Map<String, String>) = key
    }

    @Test
    fun timestampsFollowTheLocalCalendarInBothDirections() {
        val december = "2026/12/31 23:59"
        val january = "2027/01/01 00:01"
        val clock = Clock(december)
        assertEquals("23:59", clock.messageTimestamp(december))
        assertEquals("12/30 01:05", clock.messageTimestamp("2026/12/30 01:05"))
        assertEquals(january, clock.messageTimestamp(january))
        clock.instant = january
        assertEquals("00:01", clock.messageTimestamp(january))
        assertEquals(december, clock.messageTimestamp(december))
        assertEquals("invalid", clock.messageTimestamp("invalid"))
    }
}
