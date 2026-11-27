package zoo

import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.getValue
import io.ktor.http.encodeURLPathPart
import kotlinx.coroutines.CancellationException
import kotlinx.serialization.json.JsonObject

private val proxiedReaction = Regex("apub:([a-f0-9]{64}):(:[A-Za-z0-9_+-]+:)")

data class NotificationReaction(val emoji: String, val image: String? = null)
data class NotificationReactionContext(val id: String, val actorIds: Set<String>)

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
    val reactions: List<NotificationReaction> = emptyList(),
    val reactionContext: NotificationReactionContext? = null,
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
    private data class ContextAvailability(val available: Boolean, val display: Row)
    private val contextAvailability = mutableStateMapOf<String, ContextAvailability>()

    val loadFailed: Boolean get() = session.activityPubNotifications.error
    val nextCursor: String? get() = session.activityPubNotifications.nextCursor

    fun snapshot(): List<NotificationDisplay> {
        val users = session.repo.users.rows.value.associateBy { it.text("id") }
        return grouped.map { (id, entries) ->
            val latest = entries.first()
            val item = latest.representative()
            val rows = entries.flatMap { it.channel ?: listOf(it.representative()) }
            val actors = rows.distinctBy { it.text("actor_id").removePrefix("local:") }
                .map { notificationActor(it, users) }
            val kind = noticeKind(item)
            val summary = if (rows.size > 1) "${groupActivityLabel(kind)} · ×${rows.size}"
                else {
                    val key = "home.notificationKinds.$kind"
                    session.t(key).takeUnless { it == key } ?: activityLabel(kind)
                }
            val unreadIds = entries.flatMap { entry ->
                (entry.channel ?: listOf(entry.representative())).filterNot { it.flag("is_read") }.map { it.text("id") }
            }
            val display = item.nested("display_data")
            val checked = contextAvailability[item.text("id")]?.takeIf { it.display == display }
            val targetUnavailable = checked?.let { !it.available }
                ?: ("message" in display && display.nested("message").isEmpty())
            val channelReactions = rows.filter { it.text("notification_type") == "reaction" }
            NotificationDisplay(
                id, actors,
                summary, item.text("content").ifBlank { item.nested("display_data").nested("message").text("content") }
                    .ifBlank { null },
                session.platform.notificationTime(latest.createdAt), unreadIds.isNotEmpty(),
                item.text("channel_id").takeIf { !targetUnavailable && it.isNotBlank() && item.text("message_id").isNotBlank() }
                    ?.let { "/channels/$it?message=${item.text("message_id")}" }
                    ?: item.text("post_id").takeIf { it.isNotBlank() }?.let { "post:$it" },
                item.text("actor_id").takeIf { noticeKind(item) == "follow_request" && item.flag("pending") && it.isNotBlank() },
                item.text("activity_id").takeIf { noticeKind(item) == "follow_request" && item.flag("pending") && it.isNotBlank() },
                kind, unreadIds,
                item.nested("display_data").nested("channel").text("channel_name").takeIf { it.isNotBlank() }
                    ?.let { "#${it.trimStart('#')}" } ?: publicUrl(item.text("post_id"))?.host,
                reactions = entries.mapNotNull { entry ->
                    entry.item?.takeIf { noticeKind(it) == "like" }?.let(::federatedReaction)
                }.distinct(),
                reactionContext = channelReactions.firstOrNull()?.text("id")?.takeIf { it.isNotBlank() }?.let { id ->
                    NotificationReactionContext(id, channelReactions.map { it.text("actor_id") }.toSet())
                },
            )
        }
    }

    suspend fun loadReactions(context: NotificationReactionContext): List<NotificationReaction> {
        val display = session.repo.notifications.rows.value.firstOrNull { it.text("id") == context.id }
            ?.nested("display_data")
        val response = try {
            session.api.request("/api/notifications/${context.id.encodeURLPathPart()}/context")
        } catch (cancelled: CancellationException) {
            throw cancelled
        } catch (failure: ApiException) {
            if (failure.status in setOf(403, 404) && display != null) recordAvailability(context.id, display, false)
            return emptyList()
        } catch (_: Exception) {
            return emptyList()
        }
        val data = response as? JsonObject
        if (display != null && data != null && "message" in data) {
            recordAvailability(context.id, display, data.nested("message").isNotEmpty())
        }
        val emojis = data?.array("reactions")?.rows().orEmpty()
            .filter { it.text("user_id") in context.actorIds }
            .map { it.text("emoji") }
            .filter { it.isNotBlank() }
            .distinct()
        if (emojis.any { it.startsWith(":") && it.endsWith(":") }) {
            try {
                session.loadWorkspaceEmojis()
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (_: Exception) {
                // Keep Unicode reactions and show the shortcode if its image is unavailable.
            }
        }
        return emojis.map { emoji ->
            proxiedReaction.matchEntire(emoji)?.let { match ->
                val (id, label) = match.destructured
                return@map NotificationReaction(label, "/api/activitypub/emoji-images/$id")
            }
            val name = emoji.removeSurrounding(":")
            val image = session.workspaceEmojis.firstOrNull {
                name != emoji && (it.text("name") == name || it.array("aliases").any { alias -> alias.toString().trim('"') == name })
            }?.text("url")?.takeIf { it.isNotBlank() }
            NotificationReaction(emoji, image)
        }
    }

    private fun recordAvailability(id: String, display: Row, available: Boolean) {
        if (id !in contextAvailability && contextAvailability.size >= 256) {
            contextAvailability.remove(contextAvailability.keys.first())
        }
        contextAvailability[id] = ContextAvailability(available, display)
    }

    private fun federatedReaction(row: Row): NotificationReaction {
        val emoji = row.text("reaction").ifBlank { "❤️" }
        val imageId = row.nested("emoji").text("image_id").takeIf { it.matches(Regex("[a-f0-9]{64}")) }
        return NotificationReaction(emoji, imageId?.let { "/api/activitypub/emoji-images/$it" })
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

    private fun groupActivityLabel(kind: String): String = session.t(when (kind) {
        "like" -> "notifications.reactionContext"
        "reply" -> "home.reply"
        "mention" -> "notifications.mentionContext"
        "repost" -> "home.repost"
        "dm" -> "notifications.dmContext"
        "follow_request", "follow_accepted" -> "home.follow"
        else -> "notifications.title"
    })

    fun viewed(id: String) {
        if (!session.env.visible || !session.env.focused || !session.env.online) return
        grouped[id]?.forEach(session.notificationReads::viewed)
    }

    private val grouped by derivedStateOf {
        val federated = session.activityPubNotifications
        inboxGroups(unifiedNotificationFeed(
            androidNotificationRows(session.repo.notifications.rows.value), federated.items, federated.nextCursor != null,
        ).entries)
    }

}


private fun noticeKind(row: Row): String = row.text("kind").ifBlank { row.text("notification_type") }
    .let { if (it == "reaction") "like" else it }
