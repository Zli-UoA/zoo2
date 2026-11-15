package zoo.android

import zoo.*

/** Missing/deleted targets require opening the channel before replying, never guessing a root. */
internal fun notificationReplyParent(notification: Row, messages: List<Row>): String? {
    val target = messages.find {
        it.text("id") == notification.text("message_id") &&
            it.text("channel_id") == notification.text("channel_id") && !it.present("deleted_at")
    } ?: return null
    return target.text("parent_id").ifEmpty { target.text("id") }
}
