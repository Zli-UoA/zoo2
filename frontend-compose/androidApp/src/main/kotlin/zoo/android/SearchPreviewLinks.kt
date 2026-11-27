package zoo.android

import zoo.Row
import zoo.text
import zoo.present
import zoo.flag
import zoo.timestampMillis

/** Keep URLs already classified from full messages, even after their channel subscription closes. */
internal class SearchPreviewLinks(private val limit: Int = 512) {
    private data class Link(val revision: Long, val url: String)

    private val links = linkedMapOf<Pair<String, String>, Link>()

    fun remember(message: Row, url: String?) {
        val key = message.text("channel_id") to message.text("id")
        links.remove(key)
        if (url == null || key.first.isBlank() || key.second.isBlank()) return
        val revision = revision(message) ?: return
        links[key] = Link(revision, url)
        if (links.size > limit) links.remove(links.keys.first())
    }

    fun cached(hit: Row, current: Row? = null): String? {
        if (current != null && (current.present("deleted_at") || current.present("ciphertext") ||
                revision(current) != revision(hit))) return null
        return links[hit.text("channel_id") to hit.text("id")]
            ?.takeIf { hit.containsKey("updated_at") && it.revision == revision(hit) }
            ?.url
    }

    fun remove(channelId: String, messageId: String) {
        links.remove(channelId to messageId)
    }

    fun removeMessage(messageId: String) {
        links.keys.removeAll { it.second == messageId }
    }

    private fun revision(row: Row): Long? =
        timestampMillis(row.text("updated_at").ifBlank { row.text("created_at") })
}


internal fun searchChannelVisible(channel: Row, member: Boolean, guest: Boolean, encrypted: Boolean): Boolean =
    !channel.present("deleted_at") && !encrypted && (member || (!guest && !channel.flag("is_private")))
