package zoo.android

import android.content.Context
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.*
import zoo.Row
import zoo.array
import zoo.text
import zoo.android.ui.EmojiChoice
import zoo.android.ui.KitchenChoices

internal class AndroidEmojiCatalog(private val readAsset: (String) -> String) {
    constructor(context: Context) : this({ name ->
        context.assets.open("emoji/$name").bufferedReader().use { it.readText() }
    })

    private val standard by lazy {
        val popular = listOf("😀", "😊", "😂", "🥰", "👍", "❤️", "🎉", "🙌", "👏", "✨", "🔥", "🙏")
            .withIndex().associate { it.value.replace("\uFE0F", "") to it.index }
        read("shortcodes.json").entries.groupBy({ it.value.jsonPrimitive.content }, { it.key }).map { (glyph, names) ->
            EmojiChoice(glyph, glyph, names.first().replace('_', ' '), keywords = names.joinToString(" "))
        }.sortedBy { popular[it.value.replace("\uFE0F", "")] ?: Int.MAX_VALUE }
    }
    private val kitchen by lazy { parseKitchen(read("emoji-kitchen.json")) }

    suspend fun unicode(): List<EmojiChoice> = withContext(Dispatchers.IO) { standard }

    suspend fun kitchen(): KitchenChoices = withContext(Dispatchers.IO) { kitchen }

    private fun read(name: String): JsonObject = Json.parseToJsonElement(readAsset(name)).jsonObject
}

internal fun workspaceEmojiChoices(rows: List<Row>): List<EmojiChoice> = rows.mapNotNull { row ->
    val name = row.text("name").trim(':').takeIf { it.isNotBlank() } ?: return@mapNotNull null
    EmojiChoice(":$name:", "", name, row.text("url").ifBlank { null },
        row.text("category") + " " + row.array("aliases").joinToString(" ") { it.jsonPrimitive.contentOrNull.orEmpty() })
}

internal fun accountEmojiChoices(rows: List<Row>): List<EmojiChoice> = rows.mapNotNull { row ->
    val username = row.text("username").takeIf { it.isNotBlank() } ?: return@mapNotNull null
    val label = "@$username"
    EmojiChoice(":$label:", "", label, row.text("avatar_url").ifBlank { null }, row.text("display_name"))
}

private fun parseKitchen(data: JsonObject): KitchenChoices {
    val components = data.array("emojis").map { row ->
        val (code, glyph, label) = row.jsonArray.map { it.jsonPrimitive.content }
        EmojiChoice(code, glyph, label.replace('_', ' '))
    }
    val dates = data.array("dates").map { it.jsonPrimitive.content }
    val pairs = data["pairs"]?.jsonObject.orEmpty()
    val index = components.mapIndexed { index, choice -> choice.value to index }.toMap()
    return KitchenChoices(components) { first ->
        val firstIndex = index.getValue(first.value)
        components.mapIndexedNotNull { secondIndex, second ->
            val low = minOf(firstIndex, secondIndex)
            val high = maxOf(firstIndex, secondIndex)
            val packed = pairs["${low}_${high}"]?.jsonPrimitive?.int ?: return@mapIndexedNotNull null
            val date = dates[packed / 2]
            val (left, right) = (if (packed % 2 == 0) listOf(low, high) else listOf(high, low)).map { i ->
                components[i].value.split('-').joinToString("-") { "u$it" }
            }
            val code = listOf(first.value, second.value).map { value ->
                value.split('-').filterNot { it == "fe0f" }.joinToString("-")
            }.sorted().joinToString("_")
            second.copy(value = "kitchen:$code", glyph = first.glyph + second.glyph,
                image = "https://www.gstatic.com/android/keyboard/emojikitchen/$date/$left/${left}_$right.png")
        }
    }
}
