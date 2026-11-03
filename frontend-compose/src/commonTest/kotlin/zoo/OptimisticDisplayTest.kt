package zoo

import kotlin.test.*
import kotlinx.serialization.json.*

class OptimisticDisplayTest {
    private val reaction = obj("id" to "own", "message_id" to "root", "user_id" to "me", "emoji" to "👍")
    private val remote = obj("id" to "peer", "message_id" to "root", "user_id" to "peer", "emoji" to "👍")
    private val reply = obj("id" to "reply", "parent_id" to "root")
    private val display = JsonObject(mapOf("reply_count" to JsonPrimitive(10), "reactions" to JsonArray(listOf(remote))))

    @Test fun initialShapesDoNotReplaceJoinedData() {
        assertSame(display, optimisticDisplay(display, "root", emptyList(), emptyMap(), emptyList(), emptyMap()))
    }
    @Test fun localInsertsPreserveRemoteReactionAndCount() {
        val result = optimisticDisplay(display, "root", emptyList(), mapOf("reply" to reply), emptyList(), mapOf("own" to reaction))
        assertEquals("11", result.text("reply_count"))
        assertEquals(listOf(remote, reaction), result.array("reactions").rows())
    }
    @Test fun deletionAndRollbackUseJoinedBaseline() {
        val cached = JsonObject(display + ("reactions" to JsonArray(listOf(remote, reaction))))
        val result = optimisticDisplay(cached, "root", listOf(reply), mapOf("reply" to null), listOf(reaction), mapOf("own" to null))
        assertEquals("9", result.text("reply_count"))
        assertEquals(listOf(remote), result.array("reactions").rows())
        assertSame(cached, optimisticDisplay(cached, "root", listOf(reply), emptyMap(), listOf(reaction), emptyMap()))
    }
}
