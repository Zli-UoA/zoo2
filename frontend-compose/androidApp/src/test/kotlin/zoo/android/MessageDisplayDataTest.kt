package zoo.android

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue
import kotlinx.serialization.json.JsonArray
import zoo.obj

class MessageDisplayDataTest {
    private val message = obj(
        "id" to "m1",
        "display_data" to obj(
            "attachments" to JsonArray(listOf(obj("id" to "a1", "file_name" to "x.png"))),
            "reactions" to JsonArray(listOf(
                obj("emoji" to "👍", "user_id" to "alice"),
                obj("emoji" to "apub:image:party", "user_id" to "https://remote.example/users/bob"),
            )),
        ),
    )

    @Test
    fun attachmentsAndReactionsComeFromTheRowsDisplayData() {
        assertEquals(listOf("a1"), displayAttachments(message).map { it["id"].toString().trim('"') })
        assertEquals(listOf("👍"), displayReactions(message).map { it["emoji"].toString().trim('"') })
    }

    @Test
    fun rowsWithoutDisplayDataHaveNoAttachmentsOrReactions() {
        assertTrue(displayAttachments(obj("id" to "m2")).isEmpty())
        assertTrue(displayReactions(obj("id" to "m2")).isEmpty())
    }

    @Test
    fun pageCoversATargetOnlyWhenTheOldestLoadedRowIsNotNewerThanIt() {
        assertFalse(pageCovers(null, "2026-11-01", "b"))
        assertTrue(pageCovers("2026-10-01" to "z", "2026-11-01", "b"))
        assertFalse(pageCovers("2026-12-01" to "a", "2026-11-01", "b"))
        // 同じ時刻は id で決める。
        assertTrue(pageCovers("2026-11-01" to "a", "2026-11-01", "b"))
        assertFalse(pageCovers("2026-11-01" to "c", "2026-11-01", "b"))
    }
}
