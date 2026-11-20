package zoo

import kotlin.test.*
import kotlinx.serialization.json.*

class ThreadJoinTest {
    private fun forbidden(message: String) =
        ApiException(403, buildJsonObject { put("error", message) })

    @Test
    fun onlyTheJoinRequiredForbiddenAsksForAJoin() {
        assertTrue(forbidden("Join the thread before replying").requiresThreadJoin)
        // 投稿権限がないなどの 403、ほかの状態、本文なしは、参加しても直らない。
        assertFalse(forbidden("Only channel managers may post here").requiresThreadJoin)
        assertFalse(
            ApiException(409, buildJsonObject { put("error", "Join the thread before replying") })
                .requiresThreadJoin
        )
        assertFalse(ApiException(403).requiresThreadJoin)
    }
}
