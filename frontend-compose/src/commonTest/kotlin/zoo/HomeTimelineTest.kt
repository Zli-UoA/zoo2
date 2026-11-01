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

    @Test
    fun actorRoutesRoundTripRemoteIdsWithoutTreatingUrlSlashesAsRouteSegments() {
        val actor = "https://social.example/users/alice?key=a+b"
        assertEquals(
            actor,
            homeRouteId(
                "/home/users/https%3A%2F%2Fsocial.example%2Fusers%2Falice%3Fkey%3Da%2Bb",
                "users",
            ),
        )
        assertNull(homeRouteId("/home/posts/post-id", "users"))
        assertNull(homeRouteId("/home/users/not/a/route", "users"))
        assertEquals(
            "note?key=1",
            homeRouteId("/home/posts/note%3Fkey%3D1?ignored=search", "posts"),
        )
        assertEquals(
            "/api/home?timeline=social&author=https%3A%2F%2Fsocial.example%2Fusers%2Falice%3Fkey%3Da%2Bb&limit=75&before=entry%2Ftail",
            homeTimelinePath("home", actor, 75, "entry/tail"),
        )
    }
}
