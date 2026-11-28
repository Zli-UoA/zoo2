package zoo

import kotlin.test.*
import kotlinx.datetime.TimeZone

class SearchPeriodTest {
    @Test fun daysMustBeRealCalendarDays() {
        assertTrue(isSearchDay("2026-02-28"))
        assertFalse(isSearchDay("2026-02-30"))
        assertFalse(isSearchDay("2026-13-01"))
        assertFalse(isSearchDay("2026-1-01"))
    }

    @Test fun lastNDaysCountToday() {
        assertEquals("2026-11-22", searchPeriodAfter(SearchPeriod.Last7, "2026-11-28"))
        assertEquals("2025-11-29", searchPeriodAfter(SearchPeriod.Last365, "2026-11-28"))
        assertNull(searchPeriodAfter(SearchPeriod.Anytime, "2026-11-28"))
    }

    @Test fun theChoiceIsReadBackFromTheFilters() {
        val today = "2026-11-28"
        assertEquals(SearchPeriod.Anytime, searchPeriodOf(GlobalSearchFilters(), today))
        assertEquals(SearchPeriod.Last30, searchPeriodOf(GlobalSearchFilters(after = "2026-10-30"), today))
        assertEquals(SearchPeriod.Custom, searchPeriodOf(GlobalSearchFilters(after = "2026-10-31"), today))
        assertEquals(SearchPeriod.Custom, searchPeriodOf(GlobalSearchFilters(after = "2026-10-30", before = "2026-11-01"), today))
    }

    @Test fun theApiCutsAtLocalMidnight() {
        assertEquals("2026-11-22T00:00:00+09:00", searchLocalMidnight("2026-11-22", TimeZone.of("Asia/Tokyo")))
        assertEquals("2026-11-22T00:00:00+00:00", searchLocalMidnight("2026-11-22", TimeZone.UTC))
        // São Paulo had no midnight on 2018-11-04 (daylight saving started at 0:00).
        assertEquals("2018-11-04T01:00:00-02:00", searchLocalMidnight("2018-11-04", TimeZone.of("America/Sao_Paulo")))
    }

    @Test fun thePeriodIsKeptInTheRouteAndSentToTheApi() {
        val filters = GlobalSearchFilters(after = "2026-11-01", before = "2026-11-21")
        val path = globalSearchPath("zoo", GlobalSearchType.All, filters)
        assertTrue(path.endsWith("&after=2026-11-01&before=2026-11-21"), path)
        assertEquals(filters, globalSearchRoute(path)?.filters)
        assertEquals(GlobalSearchFilters(), globalSearchRoute("/search?q=a&after=2026-02-30")?.filters)
        val request = globalSearchRequestPath("zoo", GlobalSearchType.All, filters, null)
        assertTrue(request.contains("&after=2026-11-01T00%3A00%3A00"), request)
        assertTrue(filters.active)
    }
}
