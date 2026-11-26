package zoo

import androidx.compose.ui.unit.dp
import kotlin.test.*
import kotlinx.serialization.json.JsonObject

class HomePostContentTest {
    private val quote = obj("uri" to "https://remote.test/notes/one")

    @Test fun quoteUrlsAreExcludedBeforeTheThreePreviewLimit() {
        assertEquals(listOf("https://a.test/", "https://b.test/", "https://c.test/"),
            previewUrls("https://quoted.test https://a.test https://b.test https://c.test", "https://quoted.test/"))
    }

}
