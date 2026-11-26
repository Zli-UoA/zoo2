package zoo.android.ui

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.produceState
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

internal enum class TimelineMediaKind { Image, Video }

internal data class TimelineMedia(
    val key: String, val name: String, val kind: TimelineMediaKind, val sensitive: Boolean = false,
    val url: String? = null, val attachmentId: String? = null, val previewUrl: String? = null,
)

internal data class TimelineQuote(
    val uri: String, val postId: String?, val author: String, val body: String, val media: List<TimelineMedia>,
    val emoji: Map<String, TimelineMedia> = emptyMap(),
    val avatar: String? = null,
)

internal val LocalKitchenReactionImage = staticCompositionLocalOf<suspend (String) -> String?> { { null } }

internal val LocalTimelineImage = staticCompositionLocalOf<suspend (TimelineMedia) -> ImageBitmap?> { { null } }

internal fun timelineMediaKind(mimeType: String): TimelineMediaKind? = when (mimeType.lowercase().substringBefore('/')) {
    "image" -> TimelineMediaKind.Image
    "video" -> TimelineMediaKind.Video
    else -> null
}

@Composable
internal fun RepostAttribution(actor: String, colors: AndroidColors) {
    Row(Modifier.padding(bottom = 4.dp), verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(6.dp)) {
        UiGlyph(UiIcon.Repost, colors.secondary, AndroidControlSize.metadataIcon)
        Text("${actor}さんが再投稿", color = colors.secondary, fontSize = 12.sp,
            maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}

@Composable
internal fun TimelineQuoteCard(
    quote: TimelineQuote, colors: AndroidColors, open: () -> Unit, openMedia: (TimelineMedia) -> Unit,
) {
    Column(Modifier.fillMaxWidth().padding(top = 6.dp).clip(RoundedCornerShape(8.dp))
        .background(colors.inputSurface).padding(horizontal = 10.dp, vertical = 6.dp)) {
        Column(Modifier.fillMaxWidth().heightIn(min = AndroidControlSize.target)
            .clickable(role = Role.Button, onClick = open), verticalArrangement = Arrangement.Center) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                if (quote.author.isNotBlank()) TimelineAvatar(quote.author, quote.avatar, colors, 20.dp)
                Text(if (quote.author.isBlank()) "引用元を開く" else "引用 · ${quote.author}",
                color = colors.secondary, fontSize = if (quote.author.isBlank()) 12.sp else entryTextSize,
                    fontWeight = if (quote.author.isBlank()) AndroidWeights.label else AndroidWeights.entryAuthor)
            }
            if (quote.body.isNotBlank()) EntryBody(quote.body, colors.foreground, entryTextSize, quote.emoji, maxLines = 4)
        }
        quote.media.forEach { media -> TimelineMediaTile(media, colors) { openMedia(media) } }
    }
}

@Composable
internal fun TimelineMediaTile(media: TimelineMedia, colors: AndroidColors, open: () -> Unit) {
    val load = LocalTimelineImage.current
    val image by key(media) {
        produceState<ImageBitmap?>(null, load) {
            value = if (media.kind == TimelineMediaKind.Image && !media.sensitive) load(media) else null
        }
    }
    val bitmap = image
    Column(Modifier.fillMaxWidth().padding(top = 6.dp).clip(RoundedCornerShape(8.dp))
        .background(colors.inputSurface).clickable(role = Role.Button, onClick = open)) {
        if (bitmap != null) Image(bitmap, media.name, Modifier.fillMaxWidth().heightIn(max = 220.dp)
            .aspectRatio((bitmap.width.toFloat() / bitmap.height).coerceIn(0.85f, 2.4f)), contentScale = ContentScale.Crop)
        else Row(Modifier.fillMaxWidth().heightIn(min = AndroidControlSize.target).padding(horizontal = 10.dp),
            verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            val video = media.kind == TimelineMediaKind.Video
            UiGlyph(if (video) UiIcon.Play else UiIcon.Photo, colors.secondary, AndroidControlSize.inlineIcon)
            val label = when {
                media.sensitive -> if (video) "センシティブな動画を開く" else "センシティブな画像を開く"
                video -> "動画を再生"
                else -> media.name
            }
            Text(label, color = colors.foreground, fontSize = 14.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
    }
}

internal data class TimelineReaction(
    val value: String, val label: String, val count: String, val selected: Boolean,
    val image: String? = null, val imageId: String? = null, val canToggle: Boolean = true,
)

@Composable
private fun timelineIcon(url: String?): ImageBitmap? {
    val load = LocalTimelineImage.current
    return key(url, load) {
        produceState<ImageBitmap?>(null) {
            value = url?.let { load(TimelineMedia(it, "", TimelineMediaKind.Image, url = it)) }
        }.value
    }
}

@Composable
internal fun TimelineAvatar(author: String, url: String?, colors: AndroidColors, size: Dp = 36.dp) {
    val image = timelineIcon(url)
    Box(Modifier.size(size).clip(CircleShape).background(colors.background), contentAlignment = Alignment.Center) {
        if (image == null) Text(author.take(1), Modifier.clearAndSetSemantics {}, color = colors.secondary, fontSize = 13.sp)
        else Image(image, contentDescription = null, Modifier.matchParentSize().testTag("timeline-avatar"), contentScale = ContentScale.Crop)
    }
}

@Composable
internal fun TimelineReactionChip(reaction: TimelineReaction, colors: AndroidColors, toggle: () -> Unit) {
    val kitchen = LocalKitchenReactionImage.current
    val url = if (reaction.image == null && reaction.value.startsWith("kitchen:")) {
        key(reaction.value, kitchen) {
            produceState<String?>(null) { value = kitchen(reaction.value) }.value
        }
    } else reaction.image
    val image = timelineIcon(url)
    Row(
        Modifier.alpha(if (reaction.canToggle) 1f else 0.55f).heightIn(min = AndroidControlSize.target)
            .background(if (reaction.selected) colors.headerLight else colors.background, CircleShape)
            .clickable(enabled = reaction.canToggle, role = Role.Button, onClick = toggle).padding(horizontal = 10.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        if (image == null) Text(reaction.label, color = colors.foreground)
        else Image(image, contentDescription = reaction.label, Modifier.size(18.dp), contentScale = ContentScale.Fit)
        Text(reaction.count, color = colors.foreground)
    }
}
