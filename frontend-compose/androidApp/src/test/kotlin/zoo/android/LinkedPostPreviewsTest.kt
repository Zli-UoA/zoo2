package zoo.android

import kotlinx.coroutines.test.runTest
import org.junit.Assert.*
import org.junit.Test
import zoo.*
import zoo.android.ui.PostPreview

class LinkedPostPreviewsTest {
    @Test fun oversizedFederatedIdDoesNotBlockOtherPreviews() = runTest {
        val oversized = "https://remote.test/" + "猫".repeat(700)
        val source = LinkedPostPreviews({ ids ->
            assertEquals(listOf("local"), ids)
            listOf(obj("id" to "local", "content" to "本文"))
        }, "https://zoo.test")
        source.watch(oversized)
        source.watch("local")
        source.refresh(setOf(oversized, "local"), false)
        assertEquals(PostPreview.Unavailable, source.read(oversized))
        assertEquals("本文", (source.read("local") as PostPreview.Loaded).body)
    }

    @Test fun onlyRecentPublicResultsSurviveScrollAndForegroundChanges() = runTest {
        var now = 100L
        var reads = 0
        val source = LinkedPostPreviews({ ids ->
            reads++
            ids.map { id -> obj("id" to id, "content" to id, "visibility" to if (id == "private") "followers" else "public") }
        }, "https://zoo.test", 1, { now })
        suspend fun show(id: String) { source.watch(id); source.refresh(setOf(id), false); source.release(id) }
        show("one"); show("one")
        assertEquals(1, reads)
        show("private"); show("private")
        assertEquals(3, reads)
        show("two"); show("one")
        assertEquals(5, reads)
        now += 30_001
        show("one")
        assertEquals(6, reads)
        source.suspendDisplay()
        show("one")
        assertEquals(7, reads)
    }

    @Test fun sharesResultsAndDropsTheBodyWhenTheReaderLosesAccess() = runTest {
        var allowed = true
        val requests = mutableListOf<List<String>>()
        val source = LinkedPostPreviews({ ids ->
            requests += ids
            if (allowed) listOf(obj("post" to obj("id" to "one", "display_name" to "なぎ", "content" to "開場は10時です。"))) else emptyList()
        }, "https://zoo.example")
        source.watch("one")
        source.watch("one")
        source.watch("missing")
        source.refresh(setOf("one", "missing"), false)
        assertEquals("開場は10時です。", (source.read("one") as PostPreview.Loaded).body)
        assertEquals(PostPreview.Unavailable, source.read("missing"))
        source.release("one")
        source.refresh(setOf("one"), false)
        assertEquals(1, requests.size)
        allowed = false
        source.invalidate()
        source.refresh(setOf("one"), true)
        assertEquals(PostPreview.Unavailable, source.read("one"))
        assertEquals(listOf(listOf("one", "missing"), listOf("one")), requests)
    }
}
