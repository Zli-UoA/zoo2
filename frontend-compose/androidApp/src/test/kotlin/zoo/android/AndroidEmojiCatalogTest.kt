package zoo.android

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonPrimitive
import java.io.File
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotNull
import kotlin.test.assertTrue
import zoo.android.ui.rankEmojiChoices
import zoo.obj

class AndroidEmojiCatalogTest {
    @OptIn(ExperimentalCoroutinesApi::class)
    @Test fun prefetchesEveryImageWithBoundedConcurrencyEvenIfAnImageFails() = runTest {
        val urls = List(20) { "emoji-$it" }
        val started = mutableListOf<String>()
        val release = CompletableDeferred<Unit>()
        val prefetch = launch {
            prefetchEmojiImages(urls) { url ->
                started.add(url)
                release.await()
                if (url == urls.first()) error("Image unavailable")
            }
        }
        runCurrent()
        assertEquals(urls.take(FOREGROUND_IMAGE_PARALLELISM), started)
        release.complete(Unit)
        prefetch.join()
        assertEquals(urls.size, started.size)
        assertEquals(urls.toSet(), started.toSet())
    }

    @Test fun bundledCatalogsProvideUnicodeAndKitchenChoices() = runTest {
        val catalog = AndroidEmojiCatalog(readAsset = ::readBundledAsset)
        val unicode = catalog.unicode()
        assertTrue(unicode.size > 1_000)
        assertEquals(unicode.size, unicode.distinctBy { it.value }.size)
        val catChoice = unicode.single { it.glyph == "🐱" }
        assertTrue("cat" in catChoice.keywords)
        assertEquals("動物と自然", catChoice.category)
        assertEquals("スマイリーと感情", unicode.single { it.glyph == "😀" }.category)

        val kitchen = catalog.kitchen()
        val cat = kitchen.components.first { it.value == "1f431" }
        val smile = kitchen.components.first { it.value == "1f600" }
        val value = "kitchen:1f431_1f600"
        val combination = kitchen.combinations(cat).first { it.value == value }
        assertTrue(assertNotNull(combination.image).startsWith("https://www.gstatic.com/android/keyboard/emojikitchen/"))
        assertEquals(combination.image, kitchen.combinations(smile).first { it.value == value }.image)
    }

    @Test fun standardCategoriesFollowTheAppLocale() = runTest {
        val catalog = AndroidEmojiCatalog("en", "Uncategorized", ::readBundledAsset)
        assertEquals("animals & nature", catalog.unicode().single { it.glyph == "🐱" }.category)
    }

    @Test fun japaneseNameMatchRanksBeforeTagMatch() = runTest {
        val catalog = AndroidEmojiCatalog("ja", "未分類", ::readBundledAsset)
        val candidates = catalog.unicode().filter { it.glyph in setOf("😵", "🥴") }
        assertEquals(listOf("🥴", "😵"), candidates.map { it.glyph })
        assertEquals(listOf("😵", "🥴"), rankEmojiChoices(candidates, "めまい").map { it.glyph })
    }

    private fun readBundledAsset(name: String): String {
        val directory = if ('/' in name) "../../frontend/public/emojibase-data"
            else "../src/commonMain/composeResources/files"
        return File(directory, name).readText()
    }

    @Test fun kitchenLookupNormalizesVariationSelectorsAndOrder() = runTest {
        val catalog = AndroidEmojiCatalog {
            """{"emojis":[["1f600","😀","smile"],["2764-fe0f","❤️","heart"]],"dates":["20200101"],"pairs":{"0_1":0}}"""
        }
        val image = assertNotNull(catalog.kitchenReaction("kitchen:1f600_2764"))
        assertEquals(image, catalog.kitchenReaction("kitchen:2764-FE0F_01f600"))
        assertEquals(null, catalog.kitchenReaction("kitchen:1f600_1f431"))
    }

    @Test fun workspaceChoicesInsertTheShortcodeAndSearchAliases() {
        val custom = workspaceEmojiChoices(listOf(obj(
            "name" to "party", "aliases" to JsonArray(listOf(JsonPrimitive("celebration"))), "category" to "events",
        )), uncategorized = "Uncategorized").single()
        assertEquals(":party:", custom.value)
        assertEquals("events", custom.category)
        assertTrue("celebration" in custom.keywords)
    }
}
