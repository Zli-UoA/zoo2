package zoo

import kotlin.test.*
import kotlinx.serialization.json.*

class ThreadJoinTest {
    private fun forbidden(message: String) =
        ApiException(403, buildJsonObject { put("error", message) })

    @Test
    fun theCodeWinsOverTheMessage() {
        val withCode = { message: String, code: String ->
            ApiException(403, buildJsonObject { put("error", message); put("code", code) })
        }
        assertTrue(withCode("参加してから返信してください", "THREAD_JOIN_REQUIRED").requiresThreadJoin)
        assertFalse(withCode("Join the thread before replying", "OTHER_CODE").requiresThreadJoin)
    }

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

    @Test
    fun aReplyThatNeedsAJoinIsAnsweredTheSameForPlainAndEncryptedSends() {
        val needsJoin = ApiException(403, buildJsonObject { put("error", "x"); put("code", "THREAD_JOIN_REQUIRED") })
        val restricted = ApiException(403, buildJsonObject { put("code", "POSTING_RESTRICTED") })
        for (encrypted in listOf(false, true)) {
            assertEquals("threads.joinToReply", sendFailureKey("root", needsJoin, encrypted))
            // 親のない投稿には、スレッドの参加を案内しない。
            assertEquals("common.writeError", sendFailureKey(null, needsJoin, encrypted))
            // 投稿権限の 403 は、参加では直らない。
            assertEquals("common.writeError", sendFailureKey("root", restricted, encrypted))
        }
        // `e2ee.` で始まる理由は、暗号化の送信のときだけ、そのまま使う。
        assertEquals("e2ee.sendNeedsUnlock", sendFailureKey("root", Exception("e2ee.sendNeedsUnlock"), true))
        assertEquals("common.writeError", sendFailureKey("root", Exception("e2ee.sendNeedsUnlock"), false))
    }
}
