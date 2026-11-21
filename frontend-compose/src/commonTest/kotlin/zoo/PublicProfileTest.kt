package zoo

import kotlin.test.*

class PublicProfileTest {
    @Test
    fun anonymousProfilePathsExcludeAuthenticatedRoutes() {
        assertTrue(isPublicProfilePath("/public/users/alice?before=p1"))
        assertTrue(isPublicProfilePath("/public/users/alice/"))
        assertFalse(isPublicProfilePath("/home/users/alice"))
        assertFalse(isPublicProfilePath("/public/users/alice/private"))
    }
}
