package zoo

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

class MessageHeadingsTest {
    @Test
    fun allHeadingLevelsRetainTheirInlineContent() {
        val source = (1..6).joinToString("\n") { "${"#".repeat(it)} **Heading $it**" }
        val blocks = messageHeadingBlocks(source)
        assertEquals((1..6).toList(), blocks.map { it.level })
        assertEquals((1..6).map { "**Heading $it**" }, blocks.map { it.content })
    }

    @Test
    fun codePlainQuotesAndFunctionSubtreesDoNotCreateHeadings() {
        val protectedContent =
            "```md\n# code\n```\n<plain>\n## plain\n</plain>\n> ### quote\n\$[x2\n#### nested\n]"
        val blocks = messageHeadingBlocks("# Title\n$protectedContent\n## End")
        assertEquals(listOf(1, 2), blocks.mapNotNull { it.level })
        assertTrue(
            blocks
                .filter { it.level == null }
                .joinToString("\n") { it.content }
                .contains(protectedContent)
        )
    }

    @Test
    fun onlySupportedRootMarkersAreRemoved() {
        val source = "    # indented\n####### too deep\n#missing-space\n   ## title ###  "
        val blocks = messageHeadingBlocks(source)
        assertEquals(listOf(2), blocks.mapNotNull { it.level })
        assertEquals("title", blocks.last().content)
        assertTrue(blocks.first().content.contains("    # indented"))
        assertTrue(blocks.first().content.contains("####### too deep"))
        assertTrue(blocks.first().content.contains("#missing-space"))
    }
}
