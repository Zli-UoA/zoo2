package zoo.android.ui

import androidx.compose.animation.core.animate
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.Orientation
import androidx.compose.foundation.gestures.draggable
import androidx.compose.foundation.gestures.rememberDraggableState
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.runtime.*
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.compose.ui.window.DialogWindowProvider
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import kotlinx.coroutines.flow.collectLatest
import kotlin.math.abs
import kotlin.math.roundToInt
import kotlin.math.sign

internal val LocalTimelineImageRetry = staticCompositionLocalOf<suspend (TimelineMedia) -> ImageBitmap?> { { null } }

@Composable
internal fun TimelineImageViewer(selection: TimelineMediaSelection, dismiss: () -> Unit, openOriginal: (TimelineMedia) -> Unit) {
    Dialog(onDismissRequest = dismiss, properties = DialogProperties(
        usePlatformDefaultWidth = false, decorFitsSystemWindows = false,
    )) {
        val window = (LocalView.current.parent as? DialogWindowProvider)?.window
        SideEffect {
            window?.let {
                it.setDimAmount(0f)
                WindowCompat.getInsetsController(it, it.decorView).apply {
                    show(WindowInsetsCompat.Type.systemBars())
                }
            }
        }
        Box(Modifier.fillMaxSize().safeDrawingPadding()) { TimelineImagePages(selection, openOriginal, dismiss) }
    }
}

@Composable
internal fun TimelineImagePages(
    selection: TimelineMediaSelection, openOriginal: (TimelineMedia) -> Unit = {}, dismiss: () -> Unit,
) {
    val pager = rememberPagerState(initialPage = selection.initialPage) { selection.items.size }
    val revealed = remember(selection) { mutableStateMapOf(selection.initialPage to true) }
    var offset by remember(selection) { mutableFloatStateOf(0f) }
    val closeLabel = androidLabel("common.close", "閉じる")
    BoxWithConstraints(Modifier.fillMaxSize().testTag("timeline-image-viewer")) {
        val height = with(LocalDensity.current) { maxHeight.toPx().coerceAtLeast(1f) }
        val drag = rememberDraggableState { delta -> offset = (offset + delta).coerceIn(-height, height) }
        Box(Modifier.matchParentSize().background(Color.Black.copy(alpha = 1f - (abs(offset) / height).coerceIn(0f, 1f))))
        HorizontalPager(
            state = pager, beyondViewportPageCount = 1, pageSpacing = 12.dp,
            modifier = Modifier.matchParentSize()
                .offset { IntOffset(0, offset.roundToInt()) }
                .draggable(drag, Orientation.Vertical, onDragStopped = {
                    val exits = abs(offset) >= height * 0.22f
                    if (exits) {
                        animate(offset, sign(offset) * height) { value, _ -> offset = value }
                        dismiss()
                    } else {
                        animate(offset, 0f) { value, _ -> offset = value }
                    }
                })
                .semantics { customActions = listOf(CustomAccessibilityAction(closeLabel) { dismiss(); true }) },
        ) { index ->
            val item = selection.items[index]
            val image = remember(item) { item.copy(useThumbnail = false) }
            val hidden = image.sensitive && revealed[index] != true
            val isDenied = LocalAttachmentImageDenied.current
            val denied = image.attachmentId?.let(isDenied) == true
            val cached = LocalCachedTimelineImage.current
            val load = LocalTimelineImage.current
            val loadWithPreview = LocalTimelineImageWithPreview.current
            val retry = LocalTimelineImageRetry.current
            var attempt by remember(image) { mutableIntStateOf(0) }
            val result = key(image, hidden, attempt, cached, load, loadWithPreview, retry, isDenied) {
                val initial = remember { if (hidden || denied) null else cached(image) }
                var state by remember { mutableStateOf(ViewerImage(initial)) }
                LaunchedEffect(Unit) {
                    if (hidden) return@LaunchedEffect
                    var first = true
                    snapshotFlow { image.attachmentId?.let(isDenied) == true }.collectLatest { blocked ->
                        val opening = first
                        first = false
                        if (blocked && !opening) {
                            state = ViewerImage(null, failed = true)
                            return@collectLatest
                        }
                        if (opening && initial != null && image.attachmentId == null) return@collectLatest

                        val loaded = when {
                            opening && attempt > 0 -> retry(image)
                            opening && loadWithPreview != null -> loadWithPreview(image) { state = ViewerImage(it) }
                            else -> load(image)
                        }
                        state = ViewerImage(loaded, failed = loaded == null)
                    }
                }
                state
            }
            val bitmap = result.bitmap.takeUnless { denied }
            val blur = rememberMediaBlurHash(image.blurhash.takeIf { bitmap == null && !denied })
            Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                if (bitmap != null) {
                    Image(bitmap, image.name, Modifier.fillMaxSize().testTag("viewer-image-$index"), contentScale = ContentScale.Fit)
                } else {
                    blur?.let {
                        BoxWithConstraints(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                            val ratio = image.imageAspectRatio?.takeIf { it > 0f && it.isFinite() } ?: 4f / 3f
                            val width = minOf(maxWidth, maxHeight * ratio)
                            Image(it, null, Modifier.size(width, width / ratio), contentScale = ContentScale.FillBounds)
                        }
                    }
                }
                if (!hidden && bitmap == null) {
                    if (denied || result.failed) {
                        Column(Modifier.background(Color.Black.copy(alpha = 0.75f)).padding(12.dp),
                            horizontalAlignment = Alignment.CenterHorizontally) {
                            Text(androidLabel("attachments.imagePreviewFailed", "画像を読み込めませんでした"), color = Color.White)
                            UiIconButton(UiIcon.Refresh, androidLabel("attachments.retry", "再試行"), Color.White,
                                { attempt++ })
                        }
                    } else if (blur == null) {
                        CircularProgressIndicator(color = Color.White)
                    }
                }
                if (hidden) {
                    val label = androidLabel("attachments.showSensitive", "画像を表示")
                    Box(Modifier.matchParentSize()
                        .clickable(role = Role.Button) { revealed[index] = true }
                        .semantics { contentDescription = label },
                        contentAlignment = Alignment.Center) {
                        if (blur == null) Text(label, color = Color.White)
                    }
                } else if (index == pager.currentPage && (image.attachmentId != null || image.url != null)) {
                    UiIconButton(
                        if (image.attachmentId != null) UiIcon.Download else UiIcon.External,
                        androidLabel("attachments.openOriginal", "元のファイル"), Color.White,
                        { openOriginal(item) }, fill = Color.Black.copy(alpha = 0.65f),
                        modifier = Modifier.align(Alignment.TopEnd).safeDrawingPadding().padding(8.dp),
                    )
                }
            }
        }
    }
}

private data class ViewerImage(val bitmap: ImageBitmap?, val failed: Boolean = false)
