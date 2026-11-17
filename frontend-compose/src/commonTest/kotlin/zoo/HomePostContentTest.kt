package zoo

import androidx.compose.ui.unit.dp
import kotlin.test.*
import kotlinx.serialization.json.JsonObject

class HomePostContentTest {
    private val quote = obj("uri" to "https://remote.test/notes/one")

    @Test fun onlyTheReceiverAppendedQuoteLinkIsRemoved() {
        val link = "<https://remote.test/notes/one>"
        assertEquals("", homePostBody(link, quote))
        assertEquals("comment", homePostBody("comment\n\n$link", quote))
        assertEquals("comment $link", homePostBody("comment $link", quote))
        assertEquals(link, homePostBody(link, null))
    }

    @Test fun quoteLinksRespectLocalRoutesAndUnavailablePrivatePosts() {
        val local = JsonObject(quote + ("post" to obj("id" to "local", "remote" to false)))
        assertEquals("/home/posts/local", homeQuoteDestination(local))
        assertEquals("https://remote.test/notes/one", homeQuoteDestination(quote))
        assertNull(homeQuoteDestination(JsonObject(quote + obj("local" to true))))
        assertNull(homeQuoteDestination(obj("uri" to "javascript:alert(1)")))
    }

    @Test fun quoteUrlsAreExcludedBeforeTheThreePreviewLimit() {
        assertEquals(listOf("https://a.test/", "https://b.test/", "https://c.test/"),
            previewUrls("https://quoted.test https://a.test https://b.test https://c.test", "https://quoted.test/"))
    }

    @Test fun tallWideAndMissingImageSizesStayInsideTheFrame() {
        assertEquals(320.dp, constrainedImageHeight(160.dp, obj("width" to 100, "height" to 10000), 320.dp))
        assertEquals(16.dp, constrainedImageHeight(160.dp, obj("width" to 10000, "height" to 100), 320.dp))
        assertEquals(120.dp, constrainedImageHeight(160.dp, obj(), 320.dp))
        assertEquals(80.dp, constrainedImageHeight(160.dp, obj("width" to 10000, "height" to 100), 384.dp, 80.dp))
    }

    @Test fun homeImagesUseOnlyServerProxyPaths() {
        assertNull(homeMediaPath(obj("url" to "https://untrusted.test/image.jpg")))
        assertEquals("/api/activitypub/media-images/${"a".repeat(64)}",
            homeMediaPath(obj("image_id" to "a".repeat(64))))
    }
}
