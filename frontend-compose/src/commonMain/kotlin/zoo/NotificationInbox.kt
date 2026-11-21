package zoo

data class NotificationActor(val id: String, val name: String, val avatar: String?)

data class NotificationDisplay(
    val id: String,
    val actors: List<NotificationActor>,
    val summary: String,
    val snippet: String?,
    val time: String,
    val unread: Boolean,
    val target: String? = null,
    val followActor: String? = null,
    val followActivity: String? = null,
    val kind: String,
    val unreadIds: List<String> = emptyList(),
    val destination: String? = null,
)

private fun UnifiedNotificationEntry.representative(): Row = item ?: channel!!.first()

internal fun inboxGroups(entries: List<UnifiedNotificationEntry>): Map<String, List<UnifiedNotificationEntry>> =
    entries.groupBy { entry ->
        val row = entry.representative()
        val target = if (entry.item == null) row.text("message_id")
            else row.text("target_post_id").ifBlank { row.text("post_id") }
        if (target.isBlank()) entry.key else "post-group:${noticeKind(row)}:$target"
    }

class NotificationInbox(private val session: Session) {
    val nextCursor: String? get() = session.activityPubNotifications.nextCursor

    fun snapshot(): List<NotificationDisplay> {
        val users = session.repo.users.rows.value.associateBy { it.text("id") }
        return inboxGroups(feed().entries).map { (id, entries) ->
            val latest = entries.first()
            val item = latest.representative()
            val rows = entries.flatMap { it.channel ?: listOf(it.representative()) }
            val actors = rows.distinctBy { it.text("actor_id").removePrefix("local:") }
                .map { notificationActor(it, users) }
            val kind = noticeKind(item)
            val summary = if (rows.size > 1) "${activityLabel(kind)} · ${rows.size}件"
                else {
                    val key = "home.notificationKinds.$kind"
                    session.t(key).takeUnless { it == key } ?: activityLabel(kind)
                }
            val unreadIds = entries.flatMap { entry ->
                (entry.channel ?: listOf(entry.representative())).filterNot { it.flag("is_read") }.map { it.text("id") }
            }
            NotificationDisplay(
                id, actors,
                summary, item.text("content").ifBlank { item.nested("display_data").nested("message").text("content") }
                    .ifBlank { null },
                session.platform.notificationTime(latest.createdAt), unreadIds.isNotEmpty(),
                item.text("channel_id").takeIf { it.isNotBlank() && item.text("message_id").isNotBlank() }
                    ?.let { "/channels/$it?message=${item.text("message_id")}" }
                    ?: item.text("post_id").takeIf { it.isNotBlank() }?.let { "post:$it" },
                item.text("actor_id").takeIf { noticeKind(item) == "follow_request" && item.flag("pending") && it.isNotBlank() },
                item.text("activity_id").takeIf { noticeKind(item) == "follow_request" && item.flag("pending") && it.isNotBlank() },
                kind, unreadIds,
                item.nested("display_data").nested("channel").text("channel_name").takeIf { it.isNotBlank() }
                    ?.let { "#${it.trimStart('#')}" } ?: publicUrl(item.text("post_id"))?.host,
            )
        }
    }

    private fun notificationActor(item: Row, users: Map<String, Row>): NotificationActor {
        val actor = item.nested("display_data").nested("actor")
        val id = item.text("actor_id").removePrefix("local:")
        val user = users[id]
        val name = item.text("display_name").ifBlank { actor.text("display_name") }.ifBlank {
            user?.let(::author).orEmpty().ifBlank { actor.text("user_name").ifBlank { id } }
        }.ifBlank { "Zoo" }
        val avatar = item.nested("avatar").text("image_id").takeIf { it.matches(Regex("[a-f0-9]{64}")) }
            ?.let { "/api/activitypub/emoji-images/$it" }
            ?: actor.text("avatar_url").ifBlank { user?.text("avatar_url").orEmpty() }
                .takeIf { publicUrl(it)?.protocol?.name == "https" }
        return NotificationActor(id, name, avatar)
    }

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


private fun noticeKind(row: Row): String = row.text("kind").ifBlank { row.text("notification_type") }
    .let { if (it == "reaction") "like" else it }
