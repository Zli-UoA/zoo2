package zoo.android

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNull
import kotlin.test.assertTrue
import zoo.android.ui.EntryCard
import zoo.android.ui.EntryContentPanel
import zoo.android.ui.EntryContentText
import zoo.obj

class MessageRichContentTest {
    @Test
    fun integrationCardsReplaceFallbackTextAndRejectUnsafeLinks() {
        val content = messageRichContent(obj("card" to """
            {"cards":[{"title":"Build","url":"javascript:alert(1)","fields":[
              {"title":"Branch","value":"main","code":true,"url":"https://example.test/branches"}
            ]}]}
        """.trimIndent()))
        val card = content.blocks.single() as EntryCard
        assertTrue(content.hidesBody)
        assertEquals("Build", card.title)
        assertNull(card.url)
        assertTrue(card.fields.single().code)
        assertEquals("https://example.test/branches", card.fields.single().url)
    }

    @Test
    fun appLinkCardsKeepTheMessageBody() {
        val content = messageRichContent(obj("app_unfurls" to """
            [{"name":"GitHub","app_id":"github","url":"https://example.test/issue",
              "card":{"title":"Issue 42"}}]
        """.trimIndent()))
        assertFalse(content.hidesBody)
        val panel = content.blocks.single() as EntryContentPanel
        assertEquals("GitHub", panel.caption)
        assertEquals("https://example.test/issue", panel.url)
        assertEquals("Issue 42", (panel.blocks.single() as EntryCard).title)
    }

    @Test
    fun knownComponentsReplaceFallbackOnlyWhenTheyCanBeRendered() {
        val content = messageRichContent(obj("components" to """
            {"flags":32768,"components":[{"type":17,"components":[
              {"type":10,"content":"**Status**"},{"type":1011,"title":"Build","body":"Passed"}
            ]}]}
        """.trimIndent()))
        assertTrue(content.hidesBody)
        val panel = content.blocks.single() as EntryContentPanel
        assertEquals(EntryContentText("**Status**"), panel.blocks.first())
        assertEquals("Passed", (panel.blocks.last() as EntryCard).description)

        assertFalse(messageRichContent(obj("components" to """{"flags":32768,"components":[{"type":9999}]}""")).hidesBody)
        assertFalse(messageRichContent(obj("card" to "{")).hidesBody)
    }
}
