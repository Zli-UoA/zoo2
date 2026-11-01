package zoo

import kotlin.test.*
import kotlinx.coroutines.*
import kotlinx.coroutines.test.*

@OptIn(ExperimentalCoroutinesApi::class)
class ActivityPubSettingsTest {
    @Test
    fun saveNormalizesResponseAndRejectedWritesRetainDraft() = runTest {
        val bodies = mutableListOf<Row>()
        var reject = false
        val state =
            ActivityPubSettingsController(backgroundScope) { method, body ->
                if (method == "GET")
                    obj(
                        "display_name" to "Alice",
                        "default_display_name" to "Alice",
                        "summary" to "bio",
                    )
                else {
                    bodies.add(body!!)
                    if (reject) error("save failed")
                    obj(
                        "display_name" to body.text("display_name").trim(),
                        "summary" to body.text("summary"),
                        "default_display_name" to "Alice",
                    )
                }
            }
        state.load()
        runCurrent()
        state.editName(" New Alice ")
        state.editSummary("new bio")
        state.save()
        state.save()
        assertTrue(state.pending)
        runCurrent()
        assertEquals(1, bodies.size)
        assertEquals("New Alice", state.name)
        assertTrue(state.saved)
        state.editName("")
        assertFalse(state.saved)
        state.save()
        runCurrent()
        assertEquals("", state.name)
        reject = true
        state.editName("draft")
        state.save()
        runCurrent()
        assertTrue(state.failed)
        assertEquals("draft", state.name)
        assertFalse(state.pending)
        state.close()
    }

    @Test
    fun unicodeLimitsAndReadDisposal() = runTest {
        assertEquals(1, unicodeLength("😀"))
        assertEquals(2, unicodeLength("a😀"))
        val response = CompletableDeferred<Row>()
        val state = ActivityPubSettingsController(backgroundScope) { _, _ -> response.await() }
        state.editName("😀".repeat(100))
        assertTrue(state.valid)
        state.editName("😀".repeat(101))
        assertFalse(state.valid)
        state.editName(" ".repeat(20))
        state.editSummary("😀".repeat(1000))
        assertTrue(state.valid)
        state.editSummary("😀".repeat(1001))
        assertFalse(state.valid)
        state.load()
        runCurrent()
        state.close()
        response.complete(obj("display_name" to "late"))
        runCurrent()
        assertNull(state.profile)
        assertFalse(state.failed)
    }
}
