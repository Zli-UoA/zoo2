package zoo

import kotlinx.serialization.json.*

/** Mirror the server-owned integration card envelope; invalid data keeps the original text. */
fun messageCards(value: JsonElement?): List<Row>? =
    try {
        val parsed = if (value is JsonPrimitive) json.parseToJsonElement(value.content) else value
        val envelope = parsed as? JsonObject
        val cards =
            if (envelope?.containsKey("cards") == true)
                envelope["cards"]?.jsonArray?.map { it.jsonObject }
            else envelope?.let { listOf(it) }
        cards?.takeIf { list ->
            list.size in 1..101 &&
                list.all { card ->
                    (card["title"] as? JsonPrimitive)?.isString == true &&
                        card.text("title").isNotBlank() &&
                        card.text("title").length <= 2000 &&
                        listOf("description", "url", "source", "context", "author", "badge").all {
                            key ->
                            card[key] == null || (card[key] as? JsonPrimitive)?.isString == true
                        } &&
                        (card["fields"] == null ||
                            card["fields"] is JsonArray &&
                                card.array("fields").size <= 10 &&
                                card.array("fields").all { field ->
                                    field is JsonObject &&
                                        (field["title"] as? JsonPrimitive)?.isString == true &&
                                        field.text("title").length <= 100 &&
                                        (field["value"] as? JsonPrimitive)?.isString == true &&
                                        field.text("value").length <= 4000 &&
                                        (field["url"] == null ||
                                            (field["url"] as? JsonPrimitive)?.isString == true) &&
                                        listOf("short", "code").all { key ->
                                            field[key] == null ||
                                                (field[key] as? JsonPrimitive)?.let {
                                                    !it.isString && it.booleanOrNull != null
                                                } == true
                                        }
                                })
                }
        }
    } catch (_: Exception) {
        null
    }
