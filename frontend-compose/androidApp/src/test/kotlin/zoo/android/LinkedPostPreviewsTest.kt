package zoo.android

import kotlinx.coroutines.test.runTest
import org.junit.Assert.*
import org.junit.Test
import zoo.*
import zoo.android.ui.PostPreview

class LinkedPostPreviewsTest {
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
