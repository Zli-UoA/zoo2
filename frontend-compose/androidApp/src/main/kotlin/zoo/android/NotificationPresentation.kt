package zoo.android

import android.app.Notification
import android.app.Person
import android.graphics.Bitmap
import android.graphics.drawable.Icon
import android.os.Build
import zoo.*

internal fun notificationChannel(row: Row): String = when (row.text("notification_type")) {
    "reaction" -> "reactions"
    "dm" -> "messages"
    else -> "mentions"
}

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
    if (kind != "reaction") return content.ifEmpty { translate("notifications.$kind", notificationActor(row)) }
    val description = translate("notifications.$kind", notificationActor(row))
    val emoji = data["reactions"]?.rows()?.firstOrNull {
        it.text("user_id") == row.text("actor_id")
    }?.text("emoji").orEmpty()
    return listOf(description, emoji, content).filter { it.isNotEmpty() }.joinToString("\n")
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
