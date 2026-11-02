package zoo

import kotlin.test.*

class PublicProfileTest {
    @Test
    fun addressesMatchWebFingerOrigins() {
        assertEquals("@alice@zoo.example", publicAccountAddress("alice", "https://zoo.example/api/activitypub/users/a"))
        assertEquals("@alice@zoo.example", publicAccountAddress("alice", "https://zoo.example:443/api/activitypub/users/a"))
        assertEquals("@alice@localhost:8000", publicAccountAddress("alice", "http://localhost:8000/api/activitypub/users/a"))
        assertEquals("@alice@zoo.example:8443", publicAccountAddress("alice", "https://zoo.example:8443/api/activitypub/users/a"))
    }

    @Test
    fun anonymousProfilePathsExcludeAuthenticatedRoutes() {
        assertTrue(isPublicProfilePath("/public/users/alice?before=p1"))
        assertTrue(isPublicProfilePath("/public/users/alice/"))
        assertFalse(isPublicProfilePath("/home/users/alice"))
        assertFalse(isPublicProfilePath("/public/users/alice/private"))
    }
}
