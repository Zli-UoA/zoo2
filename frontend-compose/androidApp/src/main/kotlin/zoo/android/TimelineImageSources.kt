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
import zoo.android.ui.timelineMediaKind

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

internal fun timelineReactionCanToggle(value: String, selected: Boolean, canReact: Boolean, post: Boolean): Boolean =
    if (post) selected || canReact else !value.startsWith("apub:")

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

private val localMediaPath = Regex("/api/home/images/[a-fA-F0-9-]+")

internal fun timelinePostMedia(base: String, row: Row): TimelineMedia? {
    val raw = row.text("url")
    val local = raw.matches(localMediaPath)
    val id = imageId(row.text("image_id"))
    val kind = timelineMediaKind(row.text("media_type"))
        ?: if (local || id != null) TimelineMediaKind.Image else return null
    val preview = if (kind == TimelineMediaKind.Image && id != null) {
        timelineImageUrl(base, "/api/activitypub/media-images/$id")
    } else null
    if (kind == TimelineMediaKind.Image && !local && preview == null) return null
    val original = if (local || !raw.startsWith('/')) timelineImageUrl(base, raw) else null
    val url = original ?: preview ?: return null
    val name = row.text("name").ifBlank { if (kind == TimelineMediaKind.Video) "動画" else "画像" }
    return TimelineMedia(url, name, kind, row.flag("is_sensitive"), url = url, previewUrl = preview)
}

internal fun selectTimelineMedia(post: List<TimelineMedia>, attachments: List<TimelineMedia>, preferAttachments: Boolean) =
    distinctTimelineMedia(if (preferAttachments) attachments else post + attachments)

internal fun timelineImageIdentities(media: List<TimelineMedia>): Set<String> =
    media.flatMap { listOfNotNull(it.url, it.previewUrl) }.toSet()

internal fun distinctTimelineMedia(media: List<TimelineMedia>): List<TimelineMedia> {
    val positions = mutableMapOf<String, Int>()
    val result = mutableListOf<TimelineMedia?>()
    val groupKeys = mutableListOf<MutableSet<String>>()
    for (item in media) {
        val keys = if (item.kind == TimelineMediaKind.Image) listOfNotNull(item.key, item.url, item.previewUrl)
        else listOf(item.key)
        val matches = keys.mapNotNull(positions::get).distinct().sorted()
        val keep = matches.firstOrNull() ?: result.size
        if (keep == result.size) {
            result.add(item)
            groupKeys.add(mutableSetOf())
        }
        val winner = checkNotNull(result[keep])
        if (item.sensitive || matches.any { result[it]?.sensitive == true }) {
            result[keep] = winner.copy(sensitive = true)
        }
        for (merged in matches.drop(1)) {
            groupKeys[keep].addAll(groupKeys[merged])
            groupKeys[merged].clear()
            result[merged] = null
        }
        groupKeys[keep].addAll(keys)
        groupKeys[keep].forEach { positions[it] = keep }
    }
    return result.filterNotNull()
}
