package zoo.android

import android.app.Notification
import android.app.Person
import android.graphics.Bitmap
import android.graphics.drawable.Icon
import android.os.Build
import zoo.*

internal enum class DeliveryChannel(val id: String, val label: Int) {
    Mentions("mentions", works.zli.zoo.R.string.channel_mentions),
    Replies("replies", works.zli.zoo.R.string.channel_replies),
    Messages("messages", works.zli.zoo.R.string.channel_dm),
    ChannelMessages("channel_messages", works.zli.zoo.R.string.channel_messages),
    Reactions("reactions", works.zli.zoo.R.string.channel_reactions);

    fun register(context: android.content.Context, manager: android.app.NotificationManager) {
        val importance = when {
            this == Reactions -> android.app.NotificationManager.IMPORTANCE_DEFAULT
            this == Replies || this == ChannelMessages ->
                manager.getNotificationChannel("zoo.mentions")?.importance ?: android.app.NotificationManager.IMPORTANCE_HIGH
            else -> android.app.NotificationManager.IMPORTANCE_HIGH
        }
        manager.createNotificationChannel(android.app.NotificationChannel("zoo.$id", context.getString(label), importance)
            .apply { enableVibration(true) })
    }
}

internal fun deliveryChannel(kind: String) = when (kind) {
    "reaction" -> DeliveryChannel.Reactions
    "dm" -> DeliveryChannel.Messages
    "reply" -> DeliveryChannel.Replies
    "channel_message" -> DeliveryChannel.ChannelMessages
    else -> DeliveryChannel.Mentions
}

internal fun notificationChannel(row: Row) = deliveryChannel(row.text("notification_type")).id

// Keep reactions separate from messages, and never combine different channels/accounts.
internal fun notificationGroup(row: Row): String =
    "${notificationChannel(row)}:${row.text("channel_id")}"

internal fun notificationActor(row: Row): String =
    author(row.nested("display_data").nested("actor")).ifEmpty { "Zoo" }

internal fun notificationAvatar(row: Row): String =
    row.nested("display_data").nested("actor").text("avatar_url")

internal fun notificationBody(row: Row, translate: (String, String) -> String): String {
    val data = row.nested("display_data")
    val content = data.nested("message").text("content")
    val kind = row.text("notification_type")
    val actor = notificationActor(row)
    if (kind != "reaction") return content.ifEmpty { notificationKindText(kind, actor, translate) }
    val description = notificationKindText(kind, actor, translate)
    val emoji = data["reactions"]?.rows()?.firstOrNull {
        it.text("user_id") == row.text("actor_id")
    }?.text("emoji").orEmpty()
    val event = listOf(emoji, description).filter { it.isNotEmpty() }.joinToString(" ")
    return listOf(event, content).filter { it.isNotEmpty() }.joinToString("\n")
}

/**
 * `channel_message` のキーは新しい辞書だけに追加される。辞書に無いとき [translate] はキー名を返すので、
 * 全ロケールに既存の `notifications.channelMessage` へ落とし、それも無いときは固定文言を使う。
 * いずれの場合も翻訳キー名を通知に出さない。
 */
private fun notificationKindText(kind: String, actor: String, translate: (String, String) -> String): String {
    val key = "notifications.$kind"
    val text = translate(key, actor)
    if (text != key || kind != "channel_message") return text
    val previousKey = "notifications.channelMessage"
    return translate(previousKey, actor).takeIf { it != previousKey } ?: "新しいメッセージ"
}

/** Native styles let Android handle expansion, accessibility and conversation layout. */
internal fun notificationStyle(row: Row, body: String, self: String, avatar: Bitmap?): Notification.Style {
    if (row.text("notification_type") == "reaction") return Notification.BigTextStyle().bigText(body)
    val data = row.nested("display_data")
    val channel = data.nested("channel")
    val dm = channel.flag("is_dm") || row.text("notification_type") == "dm"
    val style = Notification.MessagingStyle(self)
    // API 26/27 infer a group conversation from the presence of a title.
    if (!dm || Build.VERSION.SDK_INT >= 28)
        style.setConversationTitle(channel.text("channel_name").ifEmpty { notificationActor(row) })
    val time = timestampMillis(row.text("created_at")) ?: 0L
    if (Build.VERSION.SDK_INT >= 28) {
        style.setGroupConversation(!dm)
        val person = Person.Builder().setName(notificationActor(row))
            .setKey(row.text("actor_id"))
            .apply { avatar?.let { setIcon(Icon.createWithBitmap(it)) } }.build()
        style.addMessage(Notification.MessagingStyle.Message(body, time, person))
    } else {
        style.addMessage(body, time, notificationActor(row))
    }
    return style
}
