package zoo

import kotlinx.serialization.json.*

/** This inbox excludes missed-call rows handled by the Android call experience. */
internal fun androidNotificationRows(rows: List<Row>): List<Row> =
    rows.filterNot { it.text("notification_type") == "missed_call" }

fun notificationGroups(rows: List<Row>): List<List<Row>> =
    rows
        .sortedWith(
            compareByDescending<Row> { notificationTimestamp(it.text("created_at")) }
                .thenByDescending { it.text("id") }
        )
        .groupBy {
            if (it.text("notification_type") == "reaction")
                "reaction:${it.text("user_id")}:${it.text("channel_id")}:${it.text("message_id")}"
            else "notification:${it.text("id")}"
        }
        .values
        .toList()

private fun notificationGroupKey(rows: List<Row>): String {
    val row = rows.first()
    return if (row.text("notification_type") == "reaction")
        "reaction:${row.text("user_id")}:${row.text("channel_id")}:${row.text("message_id")}"
    else "notification:${row.text("id")}"
}

internal fun notificationTimestamp(value: String): Long {
    val fraction = Regex("""[T ]\d{2}:\d{2}:\d{2}\.(\d+)""").find(value)?.groupValues?.get(1).orEmpty()
    return (timestampMillis(value) ?: 0L) * 1000 + (fraction.padEnd(6, '0').substring(3, 6).toLongOrNull() ?: 0L)
}

internal data class NotificationMirror(val channelId: String, val item: Row, val channelUnread: Boolean)
internal data class UnifiedNotificationEntry(val key: String, val createdAt: String, val channel: List<Row>? = null, val item: Row? = null, val mirrors: List<NotificationMirror> = emptyList())
internal data class UnifiedNotificationFeed(val entries: List<UnifiedNotificationEntry>, val mirrors: List<NotificationMirror>, val duplicateUnreadCount: Int)

internal fun unifiedNotificationFeed(rows: List<Row>, federated: List<Row>, hasOlder: Boolean = false): UnifiedNotificationFeed {
    val channels = notificationGroups(rows).map { UnifiedNotificationEntry("channel:${notificationGroupKey(it)}", it.first().text("created_at"), channel = it) }.toMutableList()
    val matches = mutableMapOf<String, Pair<Int, Row>>()
    channels.forEachIndexed { index, entry -> entry.channel!!.forEach { row ->
        val kind = when(row.text("notification_type")) { "reaction" -> "like"; "reply", "mention" -> "reply"; else -> null }
        if (kind != null) matches["$kind:${row.text("actor_id")}:${row.text("message_id")}"] = index to row
    } }
    val remote = mutableListOf<UnifiedNotificationEntry>()
    federated.forEach { item ->
        val match = if (item.text("actor_id").startsWith("local:")) matches["${item.text("kind")}:${item.text("actor_id").removePrefix("local:")}:${item.text("post_id")}"] else null
        if (match != null) {
            val entry = channels[match.first]
            channels[match.first] = entry.copy(
                mirrors = entry.mirrors + NotificationMirror(match.second.text("id"), item, !match.second.flag("is_read")),
            )
        } else remote += UnifiedNotificationEntry("activitypub:${item.text("id")}", item.text("created_at"), item = item)
    }
    val mirrors = channels.flatMap { it.mirrors }
    val cutoff = if (hasOlder) federated.minOfOrNull { notificationTimestamp(it.text("created_at")) } ?: Long.MIN_VALUE else Long.MIN_VALUE
    val entries = channels.map { entry -> entry.copy(channel = entry.channel!!.map { row ->
        if (entry.mirrors.any { it.channelId == row.text("id") && !it.item.flag("is_read") }) JsonObject(row + ("is_read" to JsonPrimitive(false))) else row
    }) } + remote
    return UnifiedNotificationFeed(entries.filter { notificationTimestamp(it.createdAt) >= cutoff }.sortedWith(compareByDescending<UnifiedNotificationEntry> { notificationTimestamp(it.createdAt) }.thenByDescending { it.key }), mirrors, mirrors.count { !it.item.flag("is_read") && it.channelUnread })
}

/** A preview may reuse the active channel, but must not open a new sync subscription. */
fun notificationCachedMessage(channel: Row, messageId: String, cached: List<Row>): Row? =
    cached.firstOrNull {
        it.text("id") == messageId && !it.present("deleted_at") &&
            it["ciphertext"].let { value -> value == null || value == kotlinx.serialization.json.JsonNull } &&
            !channel.flag("encrypted")
    }

fun notificationPreviewImage(message: Row?): Row? =
    message?.nested("display_data")?.array("attachments")?.rows()?.firstOrNull {
        it.text("id").isNotEmpty() && !it.flag("is_sensitive") &&
            it.text("content_type") in imageTypes
    }
