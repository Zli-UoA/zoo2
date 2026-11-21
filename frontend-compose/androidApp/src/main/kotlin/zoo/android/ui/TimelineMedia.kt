package zoo.android.ui

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.produceState
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextOverflow
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
)

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
            Text(if (quote.author.isBlank()) "引用元を開く" else "引用 · ${quote.author}",
                color = colors.secondary, fontSize = 12.sp, fontWeight = AndroidWeights.label)
            if (quote.body.isNotBlank()) EntryBody(quote.body, colors.foreground, 14.sp, quote.emoji, maxLines = 4)
        }
        quote.media.forEach { media -> TimelineMediaTile(media, colors) { openMedia(media) } }
    }
}

@Composable
internal fun TimelineMediaTile(media: TimelineMedia, colors: AndroidColors, open: () -> Unit) {
    val load = LocalTimelineImage.current
    val image by produceState<ImageBitmap?>(null, media) {
        if (media.kind == TimelineMediaKind.Image && !media.sensitive) value = load(media)
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
