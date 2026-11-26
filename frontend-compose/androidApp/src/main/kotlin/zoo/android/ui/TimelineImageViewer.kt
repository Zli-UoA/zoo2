package zoo.android.ui

import androidx.compose.animation.core.animate
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.Orientation
import androidx.compose.foundation.gestures.draggable
import androidx.compose.foundation.gestures.rememberDraggableState
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.compose.ui.window.DialogWindowProvider
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import kotlin.math.abs
import kotlin.math.roundToInt
import kotlin.math.sign

@Composable
internal fun TimelineImageViewer(selection: TimelineMediaSelection, dismiss: () -> Unit) {
    Dialog(onDismissRequest = dismiss, properties = DialogProperties(
        usePlatformDefaultWidth = false, decorFitsSystemWindows = false,
    )) {
        val window = (LocalView.current.parent as? DialogWindowProvider)?.window
        SideEffect {
            window?.let {
                it.setDimAmount(0f)
                WindowCompat.getInsetsController(it, it.decorView).apply {
                    systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
                    hide(WindowInsetsCompat.Type.systemBars())
                }
            }
        }
        TimelineImagePages(selection, dismiss)
    }
}

@Composable
internal fun TimelineImagePages(selection: TimelineMediaSelection, dismiss: () -> Unit) {
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
            val image = selection.items[index]
            val hidden = image.sensitive && revealed[index] != true
            val bitmap = rememberTimelineImage(image.takeUnless { hidden })
            val blur = rememberMediaBlurHash(image.blurhash.takeIf { bitmap == null })
            Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                if (bitmap != null) {
                    Image(bitmap, null, Modifier.fillMaxSize().testTag("viewer-image-$index"), contentScale = ContentScale.Fit)
                } else {
                    blur?.let {
                        BoxWithConstraints(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                            val ratio = image.imageAspectRatio?.takeIf { it > 0f && it.isFinite() } ?: 4f / 3f
                            val width = minOf(maxWidth, maxHeight * ratio)
                            Image(it, null, Modifier.size(width, width / ratio), contentScale = ContentScale.FillBounds)
                        }
                    }
                }
                if (image.sensitive) {
                    UiIconButton(
                        if (hidden) UiIcon.Eye else UiIcon.EyeOff,
                        if (hidden) androidLabel("attachments.showSensitive", "画像を表示")
                        else androidLabel("attachments.hideSensitive", "画像を隠す"),
                        Color.White, { revealed[index] = hidden }, fill = Color.Black.copy(alpha = 0.65f),
                        modifier = Modifier.align(Alignment.TopEnd).safeDrawingPadding().padding(8.dp),
                    )
                }
            }
        }
    }
}
