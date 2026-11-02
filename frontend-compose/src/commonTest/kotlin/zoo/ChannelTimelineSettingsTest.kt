package zoo

import kotlin.test.*
import kotlinx.coroutines.*
import kotlinx.coroutines.test.*

@OptIn(ExperimentalCoroutinesApi::class)
class ChannelTimelineSettingsTest {
    @Test
    fun readRetryAndRejectedWritesRetainTheEditablePattern() = runTest {
        var reads = 0
        val writes = mutableListOf<Row>()
        val state =
            ChannelTimelineSettingsController(backgroundScope) { method, body ->
                if (method == "GET") {
                    reads++
                    if (reads == 1) error("read failed")
                    obj("can_manage" to true, "ltl_channel_pattern" to "ltl-%")
                } else {
                    assertEquals("PUT", method)
                    writes.add(body!!)
                    error("write failed")
                }
            }
        state.load()
        runCurrent()
        assertTrue(state.failed)
        assertFalse(state.canManage)
        state.load()
        runCurrent()
        assertFalse(state.failed)
        assertTrue(state.canManage)
        assertEquals("ltl-%", state.pattern)
        state.edit("public-%")
        state.save()
        state.save()
        assertTrue(state.pending)
        runCurrent()
        assertEquals(listOf(obj("ltl_channel_pattern" to "public-%")), writes)
        assertTrue(state.failed)
        assertFalse(state.pending)
        assertEquals("public-%", state.pattern)
        state.load()
        runCurrent()
        assertEquals("ltl-%", state.pattern)
        assertFalse(state.failed)
        state.close()
    }

    @Test
    fun readOnlySettingsCannotWriteAndDisposalCancelsPendingSaves() = runTest {
        var manage = false
        var writes = 0
        val released = CompletableDeferred<Unit>()
        val state =
            ChannelTimelineSettingsController(backgroundScope) { method, _ ->
                if (method == "PUT") {
                    writes++
                    released.await()
                }
                obj("can_manage" to manage, "ltl_channel_pattern" to "")
            }
        state.load()
        runCurrent()
        state.edit("not-permitted")
        state.save()
        runCurrent()
        assertEquals("", state.pattern)
        assertEquals(0, writes)
        manage = true
        state.load()
        runCurrent()
        state.edit("public-%")
        state.save()
        runCurrent()
        assertEquals(1, writes)
        assertTrue(state.pending)
        state.edit("changed while saving")
        assertEquals("public-%", state.pattern)
        state.close()
        runCurrent()
        released.complete(Unit)
        runCurrent()
        state.save()
        assertEquals(1, writes)
        assertFalse(state.pending)
        assertFalse(state.failed)
    }
}
