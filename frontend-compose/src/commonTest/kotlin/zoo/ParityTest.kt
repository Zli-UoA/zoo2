package zoo

import kotlin.test.*
import kotlinx.serialization.json.*

class ParityTest {
    @Test
    fun publicUrlsRejectMissingSchemesAndEmptyMedia() {
        assertNull(publicUrl(""))
        assertNull(publicUrl("/relative"))
        assertNull(publicUrl("example.com"))
        assertNull(proxyImageUrl("https://proxy.test", ""))
    }

    @Test
    fun visibleUrlsExcludeCodePlainSilentCredentialsAndDuplicates() {
        assertEquals(
            listOf("https://a.test/", "https://b.test/path"),
            previewUrls(
                "`https://code.test` <plain>https://plain.test</plain> ?[quiet](https://silent.test) [a](https://a.test) https://a.test https://b.test/path https://user:secret@private.test"
            ),
        )
        assertEquals(3, previewUrls((1..5).joinToString(" ") { "https://$it.test" }).size)
        assertTrue(previewUrls("```\nhttps://code.test\n```").isEmpty())
    }

    @Test
    fun previewImagesOnlyUseSafeConfiguredProxy() {
        assertNull(proxyImageUrl("//evil.test/proxy", "https://image.test/a"))
        assertNull(proxyImageUrl("https://safe.test/proxy", "javascript:bad"))
        assertNull(proxyImageUrl("https://user:secret@safe.test/", "https://image.test/a"))
        val url = proxyImageUrl("/proxy?preview=1&static=1", "https://image.test/a")!!
        assertTrue(url.startsWith("/proxy?url="))
        assertFalse(url.contains("preview="))
        assertFalse(url.contains("static="))
    }

    @Test
    fun channelAndCommandCompletionRespectsCodeUrlsAndTokenSuffix() {
        val range = completionAt("詳しくは #genx", 9)!!
        assertEquals("channel", range.kind)
        assertEquals("詳しくは #general ", insertCompletion("詳しくは #genx", range, "general").first)
        assertEquals("command", completionAt("/ec", 3)?.kind)
        assertNull(completionAt("https://a.test/#gen", 19))
        assertNull(completionAt("`#gen", 5))
        assertNull(completionAt("```\n#gen", 8))
        val commands = listOf(obj("name" to "echo", "app_name" to "Notes", "app_id" to "notes"))
        assertEquals(
            "notes",
            completionOptions(completionAt("/ec", 3), emptyList(), emptySet(), commands = commands)
                .single()
                .appId,
        )
    }

}
