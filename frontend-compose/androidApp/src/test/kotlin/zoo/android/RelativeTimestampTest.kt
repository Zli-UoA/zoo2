package zoo.android

import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import kotlin.test.*
import zoo.timestampMillis

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [26, 35], manifest = Config.NONE)
class RelativeTimestampTest {
    private val now = timestampMillis("2026-11-17T12:00:00Z")!!

    @Test
    fun usesUiLocaleAndHandlesPastFutureAndTimezones() {
        assertEquals("5分前", androidRelativeTimestamp("2026-11-17T11:55:00Z", now, "ja")?.filterNot { it.isWhitespace() })
        assertEquals("5 minutes ago", androidRelativeTimestamp("2026-11-17T20:55:00+09:00", now, "en"))
        assertEquals("in 5 minutes", androidRelativeTimestamp("2026-11-17T12:05:00Z", now, "en"))
        assertEquals("0 seconds ago", androidRelativeTimestamp("2026-11-17T12:00:00Z", now, "en"))
        assertNull(androidRelativeTimestamp("invalid", now, "ja"))
    }

    @Test
    fun advancesAcrossUnitBoundariesAndKeepsOldDatesCompact() {
        assertEquals("59 seconds ago", androidRelativeTimestamp("2026-11-17T11:59:01Z", now, "en"))
        assertEquals("1 minute ago", androidRelativeTimestamp("2026-11-17T11:59:00Z", now, "en"))
        assertEquals("1 hour ago", androidRelativeTimestamp("2026-11-17T11:00:00Z", now, "en"))
        assertEquals("1 day ago", androidRelativeTimestamp("2026-11-16T12:00:00Z", now, "en"))
        assertEquals("1 week ago", androidRelativeTimestamp("2026-11-10T12:00:00Z", now, "en"))
        assertEquals("1 month ago", androidRelativeTimestamp("2026-10-17T12:00:00Z", now, "en"))
        assertEquals("1 year ago", androidRelativeTimestamp("2025-11-17T12:00:00Z", now, "en"))
    }
}
