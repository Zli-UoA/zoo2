package zoo

import kotlin.test.*

class HomeTimelineTest {
    @Test
    fun retainedTimelineComparesMicrosecondKeysWithoutFloatingPointRounding() {
        assertTrue(HomeTimeline.numericCompare("17935272000000000001", "17935272000000000000") > 0)
        assertTrue(HomeTimeline.numericCompare("9999999999999999999", "10000000000000000000") < 0)
        assertEquals(0, HomeTimeline.numericCompare("000123", "123"))
    }

    @Test
    fun repostEntriesRemainDistinctFromTheirOriginalPost() {
        assertNotEquals(
            HomeTimeline.key(obj("id" to "post", "entry_id" to "repost")),
            HomeTimeline.key(obj("id" to "post")),
        )
    }
}
