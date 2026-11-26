package zoo

import kotlinx.serialization.json.*

fun JsonElement.rows(): List<Row> =
    when (this) {
        is JsonArray -> mapNotNull { it as? JsonObject }
        is JsonObject ->
            if ("txid" in this)
                get("data")
                    ?.let { (it as? JsonArray)?.mapNotNull { row -> row as? JsonObject } }
                    .orEmpty()
            else emptyList()
        else -> emptyList()
    }

fun obj(vararg values: Pair<String, Any?>) = buildJsonObject {
    values.forEach { (key, value) ->
        put(
            key,
            when (value) {
                null -> JsonNull
                is JsonElement -> value
                is String -> JsonPrimitive(value)
                is Boolean -> JsonPrimitive(value)
                is Number -> JsonPrimitive(value)
                else -> error("Unsupported JSON value for $key")
            },
        )
    }
}

fun JsonElement.objectOrEmpty() = this as? JsonObject ?: JsonObject(emptyMap())

fun Row.nested(key: String): JsonObject =
    (get(key) as? JsonObject)
        ?: try {
            json.parseToJsonElement(text(key)).objectOrEmpty()
        } catch (_: Exception) {
            JsonObject(emptyMap())
        }

fun Row.array(key: String): JsonArray =
    (get(key) as? JsonArray)
        ?: try {
            json.parseToJsonElement(text(key)) as? JsonArray ?: JsonArray(emptyList())
        } catch (_: Exception) {
            JsonArray(emptyList())
        }
