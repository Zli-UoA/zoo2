package zoo.android.ui

import androidx.compose.ui.semantics.selected
import androidx.compose.foundation.border
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.setValue
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.snapshotFlow
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.DpSize
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.first
import zoo.ChannelLinkTarget
import kotlinx.coroutines.withContext

internal enum class TimelineMediaKind { Image, Video }

internal data class TimelineMediaSelection(
    val items: List<TimelineMedia>, val initialPage: Int, val searchTarget: ChannelLinkTarget? = null,
) {
    val selected get() = items[initialPage]
}

internal fun selectMediaPreview(items: List<TimelineMedia>, key: String): TimelineMediaSelection? {
    val selected = items.firstOrNull { it.key == key } ?: return null
    val pages = if (selected.kind == TimelineMediaKind.Image) items.filter { it.kind == TimelineMediaKind.Image }
        else listOf(selected)
    return TimelineMediaSelection(pages, pages.indexOf(selected))
}

internal data class TimelineMedia(
    val key: String, val name: String, val kind: TimelineMediaKind, val sensitive: Boolean = false,
    val url: String? = null, val attachmentId: String? = null, val previewUrl: String? = null,
    val imageAspectRatio: Float? = null, val blurhash: String? = null,
    val thumbnailUrl: String? = null, val useThumbnail: Boolean = true, val description: String? = null,
)

internal data class TimelineQuote(
    val uri: String, val postId: String?, val author: String, val body: String, val media: List<TimelineMedia>,
    val emoji: Map<String, TimelineMedia> = emptyMap(),
    val avatar: String? = null,
    val postAuthor: zoo.Row? = null,
    val local: Boolean = false,
    val remotePostAvailable: Boolean = false,
) {
    val canOpen: Boolean get() = postId != null || !local
}

internal val LocalKitchenReactionImage = staticCompositionLocalOf<suspend (String) -> String?> { { null } }

internal val LocalTimelineImage = staticCompositionLocalOf<suspend (TimelineMedia) -> ImageBitmap?> { { null } }

internal val LocalTimelineImageWithPreview =
    staticCompositionLocalOf<(suspend (TimelineMedia, (ImageBitmap) -> Unit) -> ImageBitmap?)?> { null }

internal val LocalAttachmentImageDenied = staticCompositionLocalOf<(String) -> Boolean> { { false } }

internal val LocalCachedTimelineImage = staticCompositionLocalOf<(TimelineMedia) -> ImageBitmap?> { { null } }

@Composable
internal fun rememberTimelineImage(media: TimelineMedia?, revealed: Boolean = true): ImageBitmap? {
    val isDenied = LocalAttachmentImageDenied.current
    val denied = media?.attachmentId?.let(isDenied) == true
    val cached = LocalCachedTimelineImage.current
    val load = LocalTimelineImage.current
    val loadWithPreview = LocalTimelineImageWithPreview.current
    val retry = LocalTimelineImageRetry.current
    val initial = remember(media, cached) { if (denied) null else media?.let(cached) }
    val isRevealed = rememberUpdatedState(revealed)
    return key(media, cached, load, loadWithPreview, retry, isDenied) {
        var bitmap by remember { mutableStateOf(initial) }
        LaunchedEffect(Unit) {
            if (media == null) return@LaunchedEffect
            var first = true
            snapshotFlow { media.attachmentId?.let(isDenied) == true }.collectLatest { blocked ->
                val opening = first
                first = false
                if (blocked && !opening) {
                    bitmap = null
                    return@collectLatest
                }
                if (opening && initial != null) return@collectLatest

                val prefetching = !isRevealed.value
                bitmap = if (opening && loadWithPreview != null) {
                    loadWithPreview(media) { bitmap = it }
                } else load(media)
                if (prefetching && bitmap == null && !blocked) {
                    snapshotFlow { isRevealed.value }.first { it }
                    bitmap = retry(media)
                }
            }
        }
        bitmap.takeUnless { denied }
    }
}

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
    openProfile: (() -> Unit)? = null,
) {
    var showOriginal by remember(quote.uri, quote.remotePostAvailable) { mutableStateOf(false) }
    val shape = RoundedCornerShape(12.dp)
    Column(Modifier.fillMaxWidth().padding(top = 6.dp).clip(shape).background(colors.background)
        .border(1.dp, colors.inputBoundary, shape).padding(horizontal = 12.dp, vertical = 8.dp)) {
        if (!quote.canOpen) {
            Text(androidLabel("home.embeddedPostUnavailable", "この投稿は表示できません"), color = colors.secondary)
            return@Column
        }
        val quoteLink = Modifier.heightIn(min = AndroidControlSize.target)
            .clickable(role = Role.Button) { if (quote.remotePostAvailable) showOriginal = true else open() }
        val title: @Composable () -> Unit = {
            Text(quote.author.ifBlank { android.net.Uri.parse(quote.uri).host ?: "投稿を開く" },
                color = colors.foreground, fontSize = if (quote.author.isBlank()) 12.sp else entryTextSize,
                fontWeight = if (quote.author.isBlank()) AndroidWeights.label else AndroidWeights.entryAuthor)
            if (quote.author.isNotBlank()) android.net.Uri.parse(quote.uri).host?.let { host ->
                Text(host, color = colors.secondary, fontSize = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
        }
        val body: @Composable () -> Unit = {
            if (quote.body.isNotBlank()) EntryBody(quote.body, colors.foreground, entryTextSize, quote.emoji, maxLines = 4)
        }
        if (openProfile != null) {
            Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                TimelineAvatar(quote.author, quote.avatar, colors, 20.dp, openProfile, Alignment.CenterStart)
                Column(Modifier.weight(1f).then(quoteLink), verticalArrangement = Arrangement.Center) {
                    title()
                    body()
                }
            }
        } else {
            Column(Modifier.fillMaxWidth().then(quoteLink), verticalArrangement = Arrangement.Center) {
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    if (quote.author.isNotBlank()) TimelineAvatar(quote.author, quote.avatar, colors, 20.dp)
                    title()
                }
                body()
            }
        }
        TimelineMediaGroup(quote.media, colors, apub = true, open = openMedia)
    }
    if (showOriginal) {
        AlertDialog(
            onDismissRequest = { showOriginal = false },
            title = { Text(quote.author.ifBlank { android.net.Uri.parse(quote.uri).host.orEmpty() }) },
            text = {
                Column(Modifier.verticalScroll(rememberScrollState())) {
                    android.net.Uri.parse(quote.uri).host?.let { Text(it) }
                    EntryBody(quote.body, colors.foreground, entryTextSize, quote.emoji)
                    TimelineMediaGroup(quote.media, colors, apub = true) { media ->
                        showOriginal = false
                        openMedia(media)
                    }
                }
            },
            confirmButton = {
                TextButton(onClick = { showOriginal = false }) { Text(androidLabel("common.close", "閉じる")) }
            },
            dismissButton = {
                TextButton(onClick = { showOriginal = false; open() }) {
                    Text("投稿を開く")
                }
            },
        )
    }
}

private val TimelineMedia.aspect: Float
    get() = imageAspectRatio?.takeIf { it.isFinite() && it > 0f } ?: 4f / 3f

@Composable
internal fun TimelineMediaGroup(
    media: List<TimelineMedia>, colors: AndroidColors, apub: Boolean,
    showSensitiveToggle: Boolean = true, open: (TimelineMedia) -> Unit,
) {
    if (media.isEmpty()) return
    Column {
        var start = 0
        while (start < media.size) {
            val first = media[start]
            if (first.kind == TimelineMediaKind.Video) {
                key(first.key) { TimelineMediaTile(first, colors) { open(first) } }
                start++
            } else {
                var end = start + 1
                while (end < media.size && media[end].kind == TimelineMediaKind.Image) end++
                val images = media.subList(start, end)
                key(first.key) {
                    if (apub) ApubImageGrid(images, colors, open)
                    else ChannelImageStrip(images, colors, showSensitiveToggle, open)
                }
                start = end
            }
        }
    }
}

@Composable
private fun ApubImageGrid(images: List<TimelineMedia>, colors: AndroidColors, open: (TimelineMedia) -> Unit) {
    val single = images.singleOrNull()
    var decodedRatio by remember(single) { mutableStateOf<Float?>(null) }
    val ratio = if (single == null) 16f / 9f
        else (single.imageAspectRatio ?: decodedRatio ?: single.aspect).coerceIn(3f / 4f, 16f / 9f)
    val gap = 2.dp
    Box(Modifier.padding(top = 6.dp).fillMaxWidth().aspectRatio(ratio)
        .clip(RoundedCornerShape(8.dp)).testTag("apub-image-grid")) {
        if (single != null) {
            MediaFrame(single, colors, Modifier.matchParentSize(),
                contentScale = if (single.sensitive && single.imageAspectRatio == null) ContentScale.Fit else ContentScale.Crop,
                onImageRatio = if (single.imageAspectRatio == null && !single.sensitive) ({ decodedRatio = it }) else null) { open(single) }
        } else {
            Row(Modifier.matchParentSize(), horizontalArrangement = Arrangement.spacedBy(gap)) {
                val columns = when (images.size) {
                    2 -> listOf(listOf(0), listOf(1))
                    3 -> listOf(listOf(0), listOf(1, 2))
                    else -> listOf(listOf(0, 2), listOf(1, 3))
                }
                columns.forEach { column ->
                    Column(Modifier.weight(1f).fillMaxHeight(), verticalArrangement = Arrangement.spacedBy(gap)) {
                        column.forEach { index ->
                            val image = images[index]
                            key(image.key) {
                                MediaFrame(image, colors, Modifier.weight(1f).fillMaxWidth(),
                                    more = if (index == 3) images.size - 4 else 0) { open(image) }
                            }
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun ChannelImageStrip(
    images: List<TimelineMedia>, colors: AndroidColors, showSensitiveToggle: Boolean, open: (TimelineMedia) -> Unit,
) {
    val decodedRatios = remember(images) { mutableStateMapOf<String, Float>() }
    BoxWithConstraints(Modifier.fillMaxWidth().padding(top = 6.dp)) {
        val single = images.size == 1
        val limitWidth = if (single) maxWidth else (maxWidth - 24.dp).coerceAtLeast(maxWidth / 2)
        val limitHeight = if (single) 220.dp else 160.dp
        val sizes = images.associate { image ->
            val ratio = image.imageAspectRatio ?: decodedRatios[image.key] ?: image.aspect
            val height = (limitWidth / ratio).coerceAtMost(limitHeight)
            val minimum = if (image.sensitive) AndroidControlSize.target + 8.dp else 0.dp
            image.key to DpSize(
                (height * ratio).coerceAtLeast(minimum).coerceAtMost(limitWidth),
                height.coerceAtLeast(minimum),
            )
        }
        val tile: @Composable (TimelineMedia) -> Unit = { image ->
            val size = sizes.getValue(image.key)
            MediaFrame(
                image, colors, Modifier.size(size.width, size.height).clip(RoundedCornerShape(8.dp)),
                contentScale = ContentScale.Fit, imageAlignment = Alignment.TopStart,
                onImageRatio = if (image.imageAspectRatio == null && !image.sensitive) {
                    { decodedRatios[image.key] = it }
                } else null,
                showSensitiveToggle = showSensitiveToggle,
            ) { open(image) }
        }
        if (single) tile(images.first())
        else LazyRow(Modifier.fillMaxWidth().height(sizes.values.maxOf { it.height }).testTag("channel-image-strip"),
            horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            items(images, key = { it.key }) { tile(it) }
        }
    }
}

@Composable
internal fun TimelineMediaTile(media: TimelineMedia, colors: AndroidColors, open: () -> Unit) {
    val video = media.kind == TimelineMediaKind.Video
    if (video && !media.sensitive) {
        Row(Modifier.fillMaxWidth().padding(top = 6.dp).clip(RoundedCornerShape(8.dp))
            .background(colors.inputSurface).clickable(role = Role.Button, onClick = open)
            .heightIn(min = AndroidControlSize.target).padding(horizontal = 10.dp),
            verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            UiGlyph(UiIcon.Play, colors.secondary, AndroidControlSize.inlineIcon)
            Text("動画を再生", color = colors.foreground, fontSize = 14.sp)
        }
        return
    }
    BoxWithConstraints(Modifier.fillMaxWidth().padding(top = 6.dp).clip(RoundedCornerShape(8.dp))) {
        val ratio = media.aspect.coerceIn(0.85f, 2.4f)
        MediaFrame(media, colors,
            Modifier.fillMaxWidth().height((maxWidth / ratio).coerceIn(AndroidControlSize.target, 220.dp)), open = open)
    }
}

@Composable
internal fun rememberMediaBlurHash(hash: String?): ImageBitmap? = if (hash != null) key(hash) {
    produceState<ImageBitmap?>(null) {
        value = withContext(Dispatchers.Default) { decodeBlurHash(hash) }
    }.value
} else null

@Composable
private fun MediaFrame(
    media: TimelineMedia, colors: AndroidColors, modifier: Modifier,
    contentScale: ContentScale = ContentScale.Crop, more: Int = 0, imageAlignment: Alignment = Alignment.Center,
    onImageRatio: ((Float) -> Unit)? = null, showSensitiveToggle: Boolean = true, open: () -> Unit,
) {
    val video = media.kind == TimelineMediaKind.Video
    var revealed by remember(media.key, media.url, media.previewUrl, media.sensitive) { mutableStateOf(false) }
    val hidden = media.sensitive && !revealed
    val loaded = rememberTimelineImage(media.takeUnless { video }, revealed = !hidden)
    val bitmap = loaded.takeUnless { hidden }
    if (bitmap != null && onImageRatio != null) {
        SideEffect { onImageRatio(bitmap.width.toFloat() / bitmap.height) }
    }
    val denied = media.attachmentId?.let(LocalAttachmentImageDenied.current) == true
    val placeholder = rememberMediaBlurHash(media.blurhash.takeIf { bitmap == null && !denied })
    val showLabel = if (video) androidLabel("attachments.showSensitiveVideo", "動画を表示") else androidLabel("attachments.showSensitive", "画像を表示")
    val hideLabel = if (video) androidLabel("attachments.hideSensitiveVideo", "動画を隠す") else androidLabel("attachments.hideSensitive", "画像を隠す")
    val playLabel = androidLabel("attachments.play", "再生")
    Box(modifier.background(colors.inputSurface)) {
        Box(Modifier.matchParentSize().clickable(
            role = Role.Button, onClickLabel = showLabel.takeIf { hidden },
            onClick = { if (hidden) revealed = true else open() },
        ), contentAlignment = Alignment.Center) {
            when {
                bitmap != null -> Image(bitmap, media.description ?: media.name, Modifier.matchParentSize(), alignment = imageAlignment, contentScale = contentScale)
                placeholder != null -> Image(placeholder, (media.description ?: media.name).takeUnless { hidden }, Modifier.matchParentSize().testTag("media-blurhash"),
                    contentScale = ContentScale.FillBounds)
                hidden && !showSensitiveToggle -> Text(showLabel, color = colors.foreground, fontSize = 14.sp)
                !hidden && !video -> Text(media.name, Modifier.padding(horizontal = 10.dp), color = colors.foreground,
                    fontSize = 14.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
            if (video && !hidden) {
                Box(Modifier.semantics { contentDescription = playLabel }) {
                    UiGlyph(UiIcon.Play, colors.foreground)
                }
            }
        }
        if (more > 0) {
            Box(Modifier.matchParentSize().background(Color.Black.copy(alpha = 0.55f)),
                contentAlignment = Alignment.Center) {
                Text("+$more", color = Color.White, fontSize = 24.sp, fontWeight = AndroidWeights.emphasis)
            }
        }
        if (media.sensitive && (showSensitiveToggle || !hidden)) {
            UiIconButton(
                when {
                    !showSensitiveToggle -> UiIcon.Close
                    hidden -> UiIcon.Eye
                    else -> UiIcon.EyeOff
                },
                if (hidden) showLabel else hideLabel,
                Color.White, { revealed = hidden }, fill = Color.Black.copy(alpha = 0.65f),
                modifier = Modifier.align(Alignment.TopEnd).padding(4.dp),
            )
        }
    }
}

internal data class TimelineReaction(
    val value: String, val label: String, val count: String, val selected: Boolean,
    val image: String? = null, val imageId: String? = null, val canToggle: Boolean = true,
)

@Composable
internal fun cachedTimelineIcon(url: String?): ImageBitmap? =
    url?.takeIf { it.startsWith("https://", ignoreCase = true) }
        ?.let { LocalCachedTimelineImage.current(TimelineMedia(it, "", TimelineMediaKind.Image, url = it)) }

@Composable
private fun timelineIcon(url: String?): ImageBitmap? =
    rememberTimelineImage(url?.let { TimelineMedia(it, "", TimelineMediaKind.Image, url = it) })

@Composable
internal fun TimelineAvatar(
    author: String, url: String?, colors: AndroidColors, size: Dp = 36.dp, openProfile: (() -> Unit)? = null,
    avatarAlignment: Alignment = Alignment.TopStart,
) {
    val image = timelineIcon(url)
    val imageContent: @Composable () -> Unit = {
        Box(Modifier.size(size).clip(CircleShape).background(colors.background), contentAlignment = Alignment.Center) {
            if (image == null) Text(author.take(1), Modifier.clearAndSetSemantics {}, color = colors.secondary, fontSize = 13.sp)
            else Image(image, contentDescription = null, Modifier.matchParentSize().testTag("timeline-avatar"), contentScale = ContentScale.Crop)
        }
    }
    if (openProfile == null) imageContent()
    else {
        val label = androidLabel("home.openAuthorProfile", "「${author}」のプロフィールを開く", "author" to author)
        Box(Modifier.size(AndroidControlSize.target)
            .clickable(role = Role.Button, onClick = openProfile)
            .semantics { contentDescription = label },
            contentAlignment = avatarAlignment,
        ) { imageContent() }
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
internal fun TimelineReactions(entry: TimelineItem, colors: AndroidColors, showAdd: Boolean, dispatch: (AndroidUiAction) -> Unit) {
    val sending = entry.operationStatus[EntryOperation.React] == EntryOperationStatus.Sending
    FlowRow(
        Modifier.fillMaxWidth().padding(top = 6.dp),
        horizontalArrangement = Arrangement.spacedBy(6.dp),
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        entry.reactions.forEach { reaction ->
            TimelineReactionChip(reaction.copy(canToggle = entry.canReact && !sending && reaction.canToggle), colors) {
                dispatch(AndroidUiAction.ActOnEntry(entry, EntryOperation.React, reaction.value, reaction.imageId))
            }
        }
        if (showAdd && entry.canReact) {
            val label = androidLabel("home.addReaction", "リアクションを追加")
            Box(
                Modifier.size(width = 38.dp, height = 32.dp).clip(ReactionShape)
                    .border(1.dp, colors.inputBoundary, ReactionShape)
                    .clickable(enabled = !sending, role = Role.Button) { dispatch(AndroidUiAction.ActOnEntry(entry, EntryOperation.React)) }
                    .semantics { contentDescription = label },
                contentAlignment = Alignment.Center,
            ) {
                UiGlyph(UiIcon.AddReaction, colors.foreground, 20.dp)
            }
        }
    }
}

private val ReactionShape = RoundedCornerShape(9.dp)

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
        Modifier.alpha(if (reaction.canToggle) 1f else 0.55f).heightIn(min = 32.dp)
            .clip(ReactionShape)
            .background(if (reaction.selected) colors.headerLight else colors.inputSurface)
            .border(1.dp, if (reaction.selected) colors.reactionSelectedBoundary else colors.inputBoundary, ReactionShape)
            .semantics { selected = reaction.selected }
            .clickable(enabled = reaction.canToggle, role = Role.Button, onClick = toggle).padding(horizontal = 10.dp, vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        if (image == null) Text(reaction.label, color = colors.foreground, fontSize = 18.sp, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.widthIn(max = 112.dp))
        else Image(image, contentDescription = reaction.label, Modifier.size(18.dp), contentScale = ContentScale.Fit)
        Text(reaction.count, color = colors.foreground, fontSize = 13.sp, fontWeight = AndroidWeights.emphasis)
    }
}
