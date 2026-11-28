package zoo.android

import android.content.Context
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.*
import zoo.Row
import zoo.array
import zoo.text
import zoo.android.ui.EmojiChoice
import zoo.android.ui.KitchenChoices

internal class AndroidEmojiCatalog(
    locale: String = "ja",
    private val uncategorized: String = "未分類",
    private val readAsset: (String) -> String,
) {
    constructor(context: Context, locale: String, uncategorized: String) : this(locale, uncategorized, { name ->
        context.assets.open("emoji/$name").bufferedReader().use { it.readText() }
    })

    // Keep the app-to-Emojibase locale mapping aligned with frontend/src/lib/emoji/locale.ts.
    private val dataLocale = when (locale) {
        "ja" -> "ja"
        "ko-KR", "ko-KP" -> "ko"
        "zh-Hans", "zh-Hant", "yue" -> "zh-hant"
        else -> "en"
    }

    private val standard by lazy {
        val groups = read("$dataLocale/messages.json").array("groups").associate {
            val group = it.jsonObject
            group.text("order") to group.text("message")
        }
        val metadata = Json.parseToJsonElement(readAsset("$dataLocale/data.json")).jsonArray
            .flatMap { entry ->
                val row = entry.jsonObject
                (listOf(row) + (row["skins"] as? JsonArray).orEmpty().map { it.jsonObject }).map {
                    it.text("emoji").withoutVariationSelector() to row
                }
            }.toMap()
        read("shortcodes.json").entries.groupBy({ it.value.jsonPrimitive.content }, { it.key })
            .map { (glyph, names) ->
                val row = metadata[glyph.withoutVariationSelector()]
                val choice = EmojiChoice(
                    glyph, glyph, names.first().replace('_', ' '),
                    localizedName = row?.text("label").orEmpty(),
                    keywords = (names +
                        row?.array("tags").orEmpty().map { it.jsonPrimitive.content }).joinToString(" "),
                    category = groups[row?.text("group")] ?: uncategorized,
                )
                val group = row?.text("group")?.toIntOrNull() ?: Int.MAX_VALUE
                val order = row?.text("order")?.toIntOrNull() ?: Int.MAX_VALUE
                Triple(group, order, choice)
            }.sortedWith(compareBy({ it.first }, { it.second })).map { it.third }
    }
    private val kitchen by lazy { parseKitchen(read("emoji-kitchen.json")) }

    suspend fun unicode(): List<EmojiChoice> = withContext(Dispatchers.IO) { standard }

    suspend fun kitchen(): KitchenChoices = withContext(Dispatchers.IO) { kitchen }

    suspend fun kitchenReaction(value: String): String? {
        if (!value.startsWith("kitchen:")) return null
        val parts = value.removePrefix("kitchen:").split('_')
        if (parts.size != 2) return null
        val normalized = parts.map(::kitchenComponentCode).sorted()
        val first = normalized.first()
        val key = "kitchen:" + normalized.joinToString("_")
        val catalog = kitchen()
        val component = catalog.components.firstOrNull { kitchenComponentCode(it.value) == first } ?: return null
        return catalog.combinations(component).firstOrNull { it.value == key }?.image
    }

    private fun read(name: String): JsonObject = Json.parseToJsonElement(readAsset(name)).jsonObject
}

private fun String.withoutVariationSelector() = replace("\uFE0F", "")

internal fun workspaceEmojiChoices(rows: List<Row>, uncategorized: String): List<EmojiChoice> = rows.mapNotNull { row ->
    val name = row.text("name").trim(':').takeIf { it.isNotBlank() } ?: return@mapNotNull null
    EmojiChoice(":$name:", "", name, row.text("url").ifBlank { null },
        row.text("category") + " " + row.array("aliases").joinToString(" ") { it.jsonPrimitive.contentOrNull.orEmpty() },
        category = row.text("category").ifBlank { uncategorized })
}

internal fun accountEmojiChoices(rows: List<Row>, category: String): List<EmojiChoice> = rows.mapNotNull { row ->
    val username = row.text("username").takeIf { it.isNotBlank() } ?: return@mapNotNull null
    val label = "@$username"
    EmojiChoice(":$label:", "", label, row.text("avatar_url").ifBlank { null }, row.text("display_name"), category = category)
}

private fun kitchenComponentCode(value: String) =
    value.lowercase().split('-').map { it.trimStart('0').ifEmpty { "0" } }.filterNot { it == "fe0f" }.joinToString("-")

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
            val code = listOf(first.value, second.value).map(::kitchenComponentCode).sorted().joinToString("_")
            second.copy(value = "kitchen:$code", glyph = first.glyph + second.glyph,
                image = "https://www.gstatic.com/android/keyboard/emojikitchen/$date/$left/${left}_$right.png")
        }
    }
}

internal suspend fun prefetchEmojiImages(urls: List<String>, load: suspend (String) -> Unit) = coroutineScope {
    val next = AtomicInteger()
    repeat(minOf(urls.size, FOREGROUND_IMAGE_PARALLELISM)) {
        launch {
            while (true) {
                val url = urls.getOrNull(next.getAndIncrement()) ?: break
                try {
                    load(url)
                } catch (cancelled: CancellationException) {
                    throw cancelled
                } catch (_: Exception) {
                    // Keep warming the rest of the catalog if this image is unavailable.
                }
            }
        }
    }
}
