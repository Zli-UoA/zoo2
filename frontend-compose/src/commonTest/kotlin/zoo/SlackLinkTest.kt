package zoo

import kotlin.test.*
import kotlinx.coroutines.*
import kotlinx.coroutines.test.*
import kotlinx.serialization.json.*

@OptIn(ExperimentalCoroutinesApi::class)
class SlackLinkTest {
    private val notLinked = obj("configured" to true, "link" to null)
    private val linked =
        obj(
            "configured" to true,
            "link" to obj("team_id" to "T1", "slack_user_id" to "U1", "slack_user_name" to "alice"),
        )

    @Test
    fun pluralKeysFollowTheCount() {
        assertEquals("example.count_one", pluralKey("example.count", 1))
        assertEquals("example.count_other", pluralKey("example.count", 0))
        assertEquals("example.count_other", pluralKey("example.count", 3))
    }

}
