package zoo.android

import kotlinx.serialization.SerializationException
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.intOrNull
import zoo.Row
import zoo.android.ui.EntryCard
import zoo.android.ui.EntryCardField
import zoo.android.ui.EntryContentBlock
import zoo.android.ui.EntryContentPanel
import zoo.android.ui.EntryContentText
import zoo.android.ui.EntryRichContent
import zoo.array
import zoo.flag
import zoo.json
import zoo.messageCards
import zoo.publicUrl
import zoo.rows
import zoo.text

private const val COMPONENTS_REPLACE_CONTENT = 1 shl 15

internal fun messageRichContent(row: Row): EntryRichContent {
    val main = componentContent(row["components"]) ?: legacyContent(row)
    val unfurls = (decode(row["app_unfurls"]) as? JsonArray)?.rows().orEmpty().take(20)
    val previews = unfurls.mapNotNull { unfurl ->
        val name = unfurl.text("name")
        if (name.isBlank() || unfurl.text("app_id").isBlank()) return@mapNotNull null
        val details = componentContent(unfurl["components"])?.blocks
            ?: messageCards(unfurl["card"]).orEmpty().map { it.toEntryCard() }
        details.takeIf { it.isNotEmpty() }?.let {
            EntryContentPanel(it, caption = name, url = safeUrl(unfurl.text("url")))
        }
    }
    return if (previews.isEmpty()) main else main.copy(blocks = main.blocks + previews)
}

private fun componentContent(value: JsonElement?): EntryRichContent? {
    val document = decode(value) as? JsonObject ?: return null
    val components = document["components"] as? JsonArray ?: return null
    if (components.size > 500) return null
    var remaining = 500
    fun visit(raw: JsonElement, depth: Int): List<EntryContentBlock> {
        if (depth >= 12 || remaining-- <= 0) return emptyList()
        val component = raw as? JsonObject ?: return emptyList()
        val type = (component["type"] as? JsonPrimitive)?.content
        return when (type) {
            "10", "text_display" -> listOfNotNull(component.contentText("content"))
            "1011", "card", "1016", "task_card", "1010", "alert" -> {
                val card = (component["card"] as? JsonObject) ?: component
                if (card.text("title").isNotBlank()) listOf(card.toEntryCard())
                else listOfNotNull(card.contentText("body") ?: card.contentText("content") ?: card.contentText("description"))
            }
            "1", "action_row", "9", "section", "17", "container", "1012", "carousel" -> {
                val children = component.array(if (type == "1012" || type == "carousel") "cards" else "components")
                    .flatMap { visit(it, depth + 1) } +
                    component["accessory"]?.let { visit(it, depth + 1) }.orEmpty()
                if ((type == "17" || type == "container") && children.isNotEmpty()) {
                    listOf(EntryContentPanel(children))
                } else children
            }
            "2", "button" -> {
                val url = safeUrl(component.text("url"))
                val label = component.text("label")
                if (url != null && label.isNotBlank()) listOf(EntryCard(label, "", url)) else emptyList()
            }
            else -> emptyList()
        }
    }

    val blocks = components.flatMap { visit(it, 0) }.takeIf { it.isNotEmpty() } ?: return null
    val flags = (document["flags"] as? JsonPrimitive)?.intOrNull ?: 0
    return EntryRichContent(blocks, flags and COMPONENTS_REPLACE_CONTENT != 0)
}

private fun legacyContent(row: Row): EntryRichContent {
    val blocks = (decode(row["app_blocks"]) as? JsonArray)?.takeIf { it.size <= 50 }?.rows().orEmpty()
        .flatMap { block ->
            when (block.text("type")) {
                "card" -> messageCards(block["card"]).orEmpty().map { it.toEntryCard() }
                "text" -> listOfNotNull(block.contentText("text"))
                else -> emptyList()
            }
        }
    if (blocks.any { it is EntryCard }) return EntryRichContent(blocks, hidesBody = true)
    val cards = messageCards(row["card"]).orEmpty().map { it.toEntryCard() }
    return EntryRichContent(cards + blocks, hidesBody = cards.isNotEmpty())
}

private fun decode(value: JsonElement?): JsonElement? {
    if (value !is JsonPrimitive || !value.isString) return value
    return try {
        json.parseToJsonElement(value.content)
    } catch (_: SerializationException) {
        null
    }
}

private fun safeUrl(value: String) = publicUrl(value)?.takeIf { it.protocol.name == "https" }?.toString()

private fun Row.contentText(key: String): EntryContentText? =
    (this[key] as? JsonPrimitive)?.takeIf { it.isString && it.content.isNotBlank() }
        ?.let { EntryContentText(it.content) }

private fun Row.toEntryCard() = EntryCard(
    title = text("title"),
    description = text("description").ifBlank { text("body") }.ifBlank { text("content") },
    url = safeUrl(text("url").ifBlank { text("title_url") }),
    caption = listOf("source", "context", "author", "badge", "subtitle", "subtext").map(::text).filter(String::isNotBlank).joinToString(" · "),
    fields = array("fields").rows().take(10).map {
        EntryCardField(it.text("title"), it.text("value"), it.flag("code"), safeUrl(it.text("url")))
    },
)
