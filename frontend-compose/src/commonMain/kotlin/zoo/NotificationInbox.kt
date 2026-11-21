package zoo

data class NotificationDisplay(
    val id: String,
    val author: String,
    val summary: String,
    val snippet: String?,
    val time: String,
    val unread: Boolean,
    val target: String? = null,
    val followActor: String? = null,
    val followActivity: String? = null,
    val kinds: List<String> = emptyList(),
    val avatar: String? = null,
    val unreadIds: List<String> = emptyList(),
)

private fun UnifiedNotificationEntry.representative(): Row = item ?: channel!!.first()

internal fun inboxGroups(entries: List<UnifiedNotificationEntry>): Map<String, List<UnifiedNotificationEntry>> =
    entries.groupBy { entry ->
        val row = entry.representative()
        val target = if (entry.item == null) row.text("message_id")
            else row.text("target_post_id").ifBlank { row.text("post_id") }
        if (target.isBlank()) entry.key else "post-group:$target"
    }

class NotificationInbox(private val session: Session) {
    val nextCursor: String? get() = session.activityPubNotifications.nextCursor

    fun snapshot(): List<NotificationDisplay> {
        val users = session.repo.users.rows.value.associateBy { it.text("id") }
        return inboxGroups(feed().entries).map { (id, entries) ->
            val latest = entries.first()
            val item = latest.representative()
            val rows = entries.flatMap { it.channel ?: listOf(it.representative()) }
            val actor = item.nested("display_data").nested("actor")
            val actorId = item.text("actor_id").removePrefix("local:")
            val name = item.text("display_name").ifBlank { actor.text("display_name") }.ifBlank {
                users[actorId]?.let(::author).orEmpty().ifBlank { actor.text("user_name").ifBlank { actorId } }
            }
            val kinds = rows.map(::noticeKind).distinct()
            val summary = if (rows.size > 1) "${rows.size}件 · " + kinds.joinToString("・", transform = ::activityLabel)
                else {
                    val key = "home.notificationKinds.${kinds.first()}"
                    session.t(key).takeUnless { it == key } ?: activityLabel(kinds.first())
                }
            val unreadIds = entries.flatMap { entry ->
                (entry.channel ?: listOf(entry.representative())).filterNot { it.flag("is_read") }.map { it.text("id") }
            }
            val avatar = item.nested("avatar").text("image_id").takeIf { it.matches(Regex("[a-f0-9]{64}")) }
                ?.let { "/api/activitypub/emoji-images/$it" }
                ?: actor.text("avatar_url").ifBlank { users[actorId]?.text("avatar_url").orEmpty() }
                    .takeIf { publicUrl(it)?.protocol?.name == "https" }
            NotificationDisplay(
                id, if (rows.size > 1) "$name ほか".takeIf { rows.map { it.text("actor_id").removePrefix("local:") }.distinct().size > 1 }
                    ?: name else name,
                summary, item.text("content").ifBlank { item.nested("display_data").nested("message").text("content") }
                    .ifBlank { null },
                session.platform.notificationTime(latest.createdAt), unreadIds.isNotEmpty(),
                item.text("channel_id").takeIf { it.isNotBlank() && item.text("message_id").isNotBlank() }
                    ?.let { "/channels/$it?message=${item.text("message_id")}" }
                    ?: item.text("post_id").takeIf { it.isNotBlank() }?.let { "post:$it" },
                item.text("actor_id").takeIf { noticeKind(item) == "follow_request" && item.flag("pending") && it.isNotBlank() },
                item.text("activity_id").takeIf { noticeKind(item) == "follow_request" && item.flag("pending") && it.isNotBlank() },
                kinds, avatar, unreadIds,
            )
        }
    }

    private fun noticeKind(row: Row): String = row.text("kind").ifBlank { row.text("notification_type") }
        .let { if (it == "reaction") "like" else it }

    private fun activityLabel(kind: String): String = when (kind) {
        "like" -> "リアクション"
        "reply" -> "返信"
        "mention" -> "メンション"
        "repost" -> "リポスト"
        "dm" -> "ダイレクトメッセージ"
        "follow_request", "follow_accepted" -> "フォロー"
        else -> session.t("notifications.title")
    }

    fun viewed(id: String) {
        if (!session.env.visible || !session.env.focused || !session.env.online) return
        inboxGroups(feed().entries)[id]?.forEach(session.notificationReads::viewed)
    }

    private fun feed(): UnifiedNotificationFeed {
        val federated = session.activityPubNotifications
        return unifiedNotificationFeed(
            androidNotificationRows(session.repo.notifications.rows.value), federated.items, federated.nextCursor != null,
        )
    }
}
