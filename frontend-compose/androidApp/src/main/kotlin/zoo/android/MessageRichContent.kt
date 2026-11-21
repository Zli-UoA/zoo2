package zoo.android

import kotlinx.serialization.SerializationException
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.intOrNull
import zoo.Row
import zoo.array
import zoo.flag
import zoo.json
import zoo.messageCards
import zoo.publicUrl
import zoo.rows
import zoo.text
import zoo.android.ui.EntryCard
import zoo.android.ui.EntryCardField
import zoo.android.ui.EntryRichContent

private const val COMPONENTS_REPLACE_CONTENT = 1 shl 15

internal fun messageRichContent(row: Row): EntryRichContent {
    val main = componentContent(row["components"]) ?: legacyContent(row)
    val unfurls = (decode(row["app_unfurls"]) as? JsonArray)?.rows().orEmpty().take(20)
    val previews = unfurls.flatMap { unfurl ->
        val name = unfurl.text("name")
        if (name.isBlank() || unfurl.text("app_id").isBlank()) return@flatMap emptyList()
        val components = componentContent(unfurl["components"])
        val cards = components?.cards.orEmpty().ifEmpty { messageCards(unfurl["card"]).orEmpty().map { it.toEntryCard() } }
        cards.map { card ->
            card.copy(url = card.url ?: safeUrl(unfurl.text("url")),
                caption = card.caption.ifBlank { name })
        }
    }
    return if (previews.isEmpty()) main else main.copy(cards = main.cards + previews)
}

private fun componentContent(value: JsonElement?): EntryRichContent? {
    val document = decode(value) as? JsonObject
    val components = (document?.get("components") as? JsonArray)?.takeIf { it.size <= 500 }
    if (!components.isNullOrEmpty()) {
        val cards = mutableListOf<EntryCard>()
        val text = mutableListOf<String>()
        var remaining = 500
        fun visit(raw: JsonElement, depth: Int) {
            if (depth >= 12 || remaining-- <= 0) return
            val component = raw as? JsonObject ?: return
            when ((component["type"] as? JsonPrimitive)?.content) {
                "10", "text_display" -> component.text("content").takeIf(String::isNotBlank)?.let(text::add)
                "1011", "card", "1016", "task_card", "1010", "alert" -> {
                    val card = (component["card"] as? JsonObject) ?: component
                    card.text("title").takeIf(String::isNotBlank)?.let { cards += card.toEntryCard() }
                        ?: card.text("content").ifBlank { card.text("description") }
                            .takeIf(String::isNotBlank)?.let(text::add)
                }
                "1", "action_row", "9", "section", "17", "container" -> {
                    component.array("components").forEach { visit(it, depth + 1) }
                    component["accessory"]?.let { visit(it, depth + 1) }
                }
                "2", "button" -> {
                    val url = safeUrl(component.text("url"))
                    if (url != null && component.text("label").isNotBlank()) {
                        cards += EntryCard(component.text("label"), "", url)
                    }
                }
            }
        }
        components.forEach { visit(it, 0) }
        if (cards.isNotEmpty() || text.isNotEmpty()) {
            val flags = (document["flags"] as? JsonPrimitive)?.intOrNull ?: 0
            return EntryRichContent(cards, text, flags and COMPONENTS_REPLACE_CONTENT != 0)
        }
    }

    return null
}

private fun legacyContent(row: Row): EntryRichContent {
    val blocks = (decode(row["app_blocks"]) as? JsonArray)?.takeIf { it.size <= 50 }?.rows().orEmpty()
    val appCards = blocks.filter { it.text("type") == "card" }.flatMap { messageCards(it["card"]).orEmpty() }
    val cards = appCards.ifEmpty { messageCards(row["card"]).orEmpty() }
    val text = blocks.filter { it.text("type") == "text" }.map { it.text("text") }.filter(String::isNotBlank)
    return EntryRichContent(cards.map { it.toEntryCard() }, text, cards.isNotEmpty())
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

private fun Row.toEntryCard() = EntryCard(
    title = text("title"),
    description = text("description").ifBlank { text("body") }.ifBlank { text("content") },
    url = safeUrl(text("url").ifBlank { text("title_url") }),
    caption = listOf("source", "context", "author", "badge", "subtitle", "subtext").map(::text).filter(String::isNotBlank).joinToString(" · "),
    fields = array("fields").rows().take(10).map {
        EntryCardField(it.text("title"), it.text("value"), it.flag("code"), safeUrl(it.text("url")))
    },
)
