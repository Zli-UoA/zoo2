package zoo.android

import zoo.Row
import zoo.array
import zoo.endpoint
import zoo.flag
import zoo.nested
import zoo.publicUrl
import zoo.text
import zoo.android.ui.TimelineMedia
import zoo.android.ui.TimelineMediaKind

private val imageIdPattern = Regex("[a-f0-9]{64}")
private val remoteReactionPattern = Regex("apub:([a-f0-9]{64}):(:[A-Za-z0-9_+-]+:)")

internal fun imageId(value: String): String? = value.takeIf(imageIdPattern::matches)

internal fun timelineImageUrl(base: String, raw: String): String? {
    val url = if (raw.startsWith('/') && !raw.startsWith("//")) endpoint(base, raw) else raw
    return publicUrl(url)?.takeIf { it.protocol.name == "https" }?.toString()
}

internal fun timelineEmojiUrl(base: String, emoji: Row): String? =
    if (emoji.flag("local")) timelineImageUrl(base, emoji.text("url"))
    else imageId(emoji.text("image_id"))?.let { timelineImageUrl(base, "/api/activitypub/emoji-images/$it") }

internal fun timelineAvatarUrl(base: String, row: Row, user: Row? = null): String? =
    timelineImageUrl(base, row.text("avatar_url"))
        ?: imageId(row.nested("avatar").text("image_id"))?.let {
            timelineImageUrl(base, "/api/activitypub/emoji-images/$it")
        }
        ?: user?.let { timelineImageUrl(base, it.text("avatar_url")) }

internal fun timelineReactionEmoji(base: String, value: String, known: Map<String, String?>): Pair<String, String?> {
    val remote = remoteReactionPattern.matchEntire(value)
    if (remote != null) {
        val (id, label) = remote.destructured
        return label to timelineImageUrl(base, "/api/activitypub/emoji-images/$id")
    }
    return value to known[value]
}

internal fun workspaceReactionImages(base: String, values: Set<String>, emojis: List<Row>): Map<String, String> {
    if (values.none { it.startsWith(':') }) return emptyMap()
    return buildMap {
        emojis.forEach { emoji ->
            val names = listOf(emoji.text("name")) + emoji.array("aliases").map { it.toString().trim('"') }
            names.map { ":${it.trim(':')}:" }.filter { it in values }.forEach { name ->
                timelineImageUrl(base, emoji.text("url"))?.let { put(name, it) }
            }
        }
    }
}

internal fun selectTimelineMedia(post: List<TimelineMedia>, attachments: List<TimelineMedia>, preferAttachments: Boolean) =
    distinctTimelineMedia(if (preferAttachments) attachments else post + attachments)

internal fun distinctTimelineMedia(media: List<TimelineMedia>): List<TimelineMedia> {
    val positions = mutableMapOf<String, Int>()
    val result = mutableListOf<TimelineMedia>()
    for (item in media) {
        val keys = if (item.kind == TimelineMediaKind.Image) listOfNotNull(item.key, item.url, item.previewUrl)
        else listOf(item.key)
        val index = keys.firstNotNullOfOrNull(positions::get) ?: result.size
        if (index == result.size) result.add(item)
        else if (item.sensitive && !result[index].sensitive) result[index] = result[index].copy(sensitive = true)
        keys.forEach { positions.putIfAbsent(it, index) }
    }
    return result
}
