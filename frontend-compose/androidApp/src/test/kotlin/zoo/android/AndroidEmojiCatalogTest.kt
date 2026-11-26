package zoo.android

import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonPrimitive
import java.io.File
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotNull
import kotlin.test.assertTrue
import zoo.obj

class AndroidEmojiCatalogTest {
    @Test fun bundledCatalogsProvideUnicodeAndKitchenChoices() = runTest {
        val catalog = AndroidEmojiCatalog { name ->
            File("../src/commonMain/composeResources/files", name).readText()
        }
        val unicode = catalog.unicode()
        assertTrue(unicode.size > 1_000)
        assertEquals(unicode.size, unicode.distinctBy { it.value }.size)
        assertTrue(unicode.any { it.glyph == "🐱" && "cat" in it.keywords })

        val kitchen = catalog.kitchen()
        val cat = kitchen.components.first { it.value == "1f431" }
        val smile = kitchen.components.first { it.value == "1f600" }
        val value = "kitchen:1f431_1f600"
        val combination = kitchen.combinations(cat).first { it.value == value }
        assertTrue(assertNotNull(combination.image).startsWith("https://www.gstatic.com/android/keyboard/emojikitchen/"))
        assertEquals(combination.image, kitchen.combinations(smile).first { it.value == value }.image)
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
        ))).single()
        assertEquals(":party:", custom.value)
        assertTrue("celebration" in custom.keywords)
    }
}
