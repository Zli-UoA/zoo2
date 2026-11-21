package zoo

data class NotificationDisplay(
    val id: String,
    val author: String,
    val summary: String,
    val snippet: String?,
    val time: String,
    val unread: Boolean,
)

class NotificationInbox(private val session: Session) {
    fun snapshot(): List<NotificationDisplay> {
        val users = session.repo.users.rows.value.associateBy { it.text("id") }
        return feed().entries.map { entry ->
            val item = entry.item ?: entry.channel!!.first()
            val actorId = item.text("actor_id").removePrefix("local:")
            val name = item.text("display_name").ifBlank {
                users[actorId]?.let { it.text("display_name").ifBlank { it.text("user_name") } }
                    .orEmpty().ifBlank { actorId }
            }
            val kind = when (item.text("kind").ifBlank { item.text("notification_type") }) {
                "reaction" -> "like"
                else -> item.text("kind").ifBlank { item.text("notification_type") }
            }
            val summary = session.t("home.notificationKinds.$kind").takeUnless {
                it == "home.notificationKinds.$kind"
            } ?: session.t("notifications.title")
            NotificationDisplay(
                entry.key, name, summary, item.text("content").ifBlank { null },
                session.platform.notificationTime(entry.createdAt),
                entry.channel?.any { !it.flag("is_read") } ?: !item.flag("is_read"),
            )
        }
    }

    fun viewed(id: String) {
        if (!session.env.visible || !session.env.focused || !session.env.online) return
        feed().entries.firstOrNull { it.key == id }?.let(session.notificationReads::viewed)
    }

    private fun feed(): UnifiedNotificationFeed {
        val federated = session.activityPubNotifications
        return unifiedNotificationFeed(
            androidNotificationRows(session.repo.notifications.rows.value), federated.items, federated.nextCursor != null,
        )
    }
}
