package zoo

import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive

/** Preserve joined server data, overlaying only the repository's pending writes. */
internal fun optimisticDisplay(
    display: Row,
    messageId: String,
    baseMessages: List<Row>,
    messageEdits: Map<String, Row?>,
    baseReactions: List<Row>,
    reactionEdits: Map<String, Row?>,
    workspaceEmojis: List<Row> = emptyList(),
): Row {
    var replyDelta = 0
    for ((id, after) in messageEdits) {
        val before = baseMessages.find { rowKey(it) == id }
        if (before?.text("parent_id") == messageId && !before.present("deleted_at")) replyDelta--
        if (after?.text("parent_id") == messageId && !after.present("deleted_at")) replyDelta++
    }
    val reactions = display.array("reactions").rows().associateBy { it.text("id") }.toMutableMap()
    val emojis = display.nested("emojis").toMutableMap()
    var changed = replyDelta != 0
    for ((id, after) in reactionEdits) {
        val before = baseReactions.find { rowKey(it) == id }
        if (before?.text("message_id") != messageId && after?.text("message_id") != messageId) continue
        changed = true
        reactions.remove(id)
        if (after?.text("message_id") == messageId) {
            reactions[id] = after
            val key = after.text("emoji")
            if (key.startsWith(":") && key.endsWith(":")) {
                val name = key.removeSurrounding(":")
                val emoji = workspaceEmojis.find { it.text("name") == name || it.array("aliases").any { alias -> alias.toString().trim('"') == name } }
                if (emoji != null && name !in emojis) emojis[name] = emoji
            }
        }
    }
    return if (!changed) display else JsonObject(display + mapOf(
        "reply_count" to JsonPrimitive(((display.text("reply_count").toIntOrNull() ?: 0) + replyDelta).coerceAtLeast(0)),
        "reactions" to JsonArray(reactions.values.toList()),
        "emojis" to JsonObject(emojis),
    ))
}
