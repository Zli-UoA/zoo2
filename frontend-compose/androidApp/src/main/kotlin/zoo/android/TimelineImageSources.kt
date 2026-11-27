package zoo.android

import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull
import zoo.Row
import zoo.accountEmojiName
import zoo.array
import zoo.endpoint
import zoo.findCustomEmojiShortcodes
import zoo.flag
import zoo.messageExcerpt
import zoo.nested
import zoo.publicUrl
import zoo.rows
import zoo.text
import zoo.android.ui.TimelineMedia
import zoo.android.ui.TimelineMediaKind
import zoo.android.ui.timelineMediaKind
import zoo.android.ui.containsRenderedEmoji

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

internal fun workspaceEmojiImages(base: String, emojis: List<Row>): Map<String, String> = buildMap {
    for (emoji in emojis) {
        val url = timelineImageUrl(base, emoji.text("url")) ?: continue
        val names = listOf(emoji.text("name")) + emoji.array("aliases").mapNotNull { (it as? JsonPrimitive)?.contentOrNull }
        names.map { it.trim(':') }.filter { it.isNotBlank() }.forEach { put(":$it:", url) }
    }
}

internal fun timelineBodyEmojis(
    base: String,
    data: Row,
    metadata: Row,
    body: String,
    isPost: Boolean,
    workspace: Map<String, String>,
    displayedBlocks: List<String> = emptyList(),
): Map<String, TimelineMedia> {
    val attached = if (isPost) data.array("emojis").rows().map { it.text("name") to it }
    else (data.nested("display_data").nested("emojis") + metadata.nested("emojis"))
        .mapNotNull { (name, value) -> (value as? JsonObject)?.let { name to it } }
    return buildMap {
        for ((rawName, emoji) in attached.take(256)) {
            val name = rawName.trim(':')
            if (name.isBlank()) continue
            val url = if (isPost) timelineEmojiUrl(base, emoji)
            else timelineImageUrl(base, emoji.text("url"))
            if (url == null) continue
            val media = inlineEmoji(name, url)
            put(":$name:", media)
            emoji.array("aliases").take(32).mapNotNull { (it as? JsonPrimitive)?.contentOrNull }
                .map { it.trim(':') }.filter { it.isNotBlank() }
                .forEach { put(":$it:", media) }
        }
        // A missing remote emoji must retain its shortcode rather than borrow a local image.
        if (workspace.isNotEmpty() && (!isPost || !data.flag("remote"))) {
            for (code in (sequenceOf(body) + displayedBlocks.asSequence()).flatMap(::findCustomEmojiShortcodes)) {
                val url = workspace[code] ?: continue
                if (code !in this) put(code, inlineEmoji(code.trim(':'), url))
            }
        }
    }
}

internal fun isWorkspaceEmojiCandidate(code: String): Boolean =
    code.startsWith(':') && code.endsWith(':') && accountEmojiName(code) == null &&
        code.removeSurrounding(":").let { it.isNotEmpty() && it == it.trim() && ':' !in it }

internal fun hasMissingWorkspaceEmoji(body: String, emoji: Map<String, TimelineMedia>): Boolean {
    val candidates = findCustomEmojiShortcodes(body).filter { it !in emoji && isWorkspaceEmojiCandidate(it) }.toSet()
    return candidates.isNotEmpty() && containsRenderedEmoji(body, emoji.keys, candidates)
}

internal fun linkedExcerptNeedsWorkspaceEmoji(body: String, known: Set<String>): Boolean {
    val candidates = findCustomEmojiShortcodes(body).filter { it !in known && isWorkspaceEmojiCandidate(it) }.toSet()
    if (candidates.isEmpty()) return false
    val excerpt = messageExcerpt(body, customEmojis = known)
    return candidates.any { candidate ->
        val withCandidate = messageExcerpt(body, customEmojis = known + candidate)
        // A speculative candidate can hide a known emoji at a shared colon.
        withCandidate.split(candidate).size > excerpt.split(candidate).size
    }
}

private fun inlineEmoji(name: String, url: String) =
    TimelineMedia(url, name, TimelineMediaKind.Image, url = url, previewUrl = url)

private val localMediaPath = Regex("/api/home/images/[a-fA-F0-9-]+")

private fun Row.imageAspectRatio(kind: TimelineMediaKind): Float? {
    if (kind != TimelineMediaKind.Image) return null
    val width = text("width").toIntOrNull()?.takeIf { it > 0 } ?: return null
    val height = text("height").toIntOrNull()?.takeIf { it > 0 } ?: return null
    return width.toFloat() / height
}

internal fun timelineAttachmentMedia(row: Row): TimelineMedia? {
    val id = row.text("id").takeIf(String::isNotBlank) ?: return null
    val kind = timelineMediaKind(row.text("content_type")) ?: return null
    val name = row.text("file_name").ifBlank { if (kind == TimelineMediaKind.Video) "動画" else "画像" }
    return TimelineMedia(id, name, kind, row.flag("is_sensitive"), attachmentId = id,
        imageAspectRatio = row.imageAspectRatio(kind), blurhash = row.text("blurhash").ifBlank { null })
}

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
    val thumbnail = if (local && kind == TimelineMediaKind.Image) "$url?thumbnail=true" else null
    val name = row.text("name").ifBlank { if (kind == TimelineMediaKind.Video) "動画" else "画像" }
    return TimelineMedia(url, name, kind, row.flag("is_sensitive"), url = url, previewUrl = preview, thumbnailUrl = thumbnail, imageAspectRatio = row.imageAspectRatio(kind), blurhash = row.text("blurhash").ifBlank { null })
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
        val sensitive = winner.sensitive || item.sensitive || matches.any { result[it]?.sensitive == true }
        val aspectRatio = winner.imageAspectRatio ?: item.imageAspectRatio
            ?: matches.firstNotNullOfOrNull { result[it]?.imageAspectRatio }
        val blurhash = winner.blurhash ?: item.blurhash ?: matches.firstNotNullOfOrNull { result[it]?.blurhash }
        result[keep] = winner.copy(sensitive = sensitive, imageAspectRatio = aspectRatio, blurhash = blurhash)
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
