package zoo

import androidx.compose.foundation.*
import androidx.compose.foundation.interaction.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.PathFillType
import androidx.compose.ui.graphics.drawscope.scale
import androidx.compose.ui.graphics.vector.PathParser
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.*
import androidx.compose.ui.zIndex
import kotlin.time.TimeSource
import kotlinx.coroutines.*

/** Sonner's normal toast does not have an icon or rich success/error colors. */
enum class ToastKind {
    Default,
    Success,
    Error,
}

data class NativeToastState(
    val serial: Long,
    val message: String,
    val kind: ToastKind,
    /** null is Sonner's Infinity duration. */
    val durationMillis: Long?,
)

private val toastEpoch = TimeSource.Monotonic.markNow()

/** The product and catalog use the same serial-safe lifetime and pause state. */
class NativeToastController(
    private val scope: CoroutineScope,
    private val clockMillis: () -> Long = { toastEpoch.elapsedNow().inWholeMilliseconds },
) {
    var current by mutableStateOf<NativeToastState?>(null)
        private set

    private var serial = 0L
    private var remainingMillis: Long? = null
    private var startedAt: Long? = null
    private var timer: Job? = null
    private var hovered = false
    private var visible = true
    private var closed = false

    fun show(message: String, kind: ToastKind = ToastKind.Default, durationMillis: Long? = 4_000L) {
        if (closed) return
        stopTimer()
        current = NativeToastState(++serial, message, kind, durationMillis)
        remainingMillis = durationMillis?.coerceAtLeast(0L)
        hovered = false
        startTimer()
    }

    fun dismiss(expectedSerial: Long? = null) {
        if (expectedSerial != null && current?.serial != expectedSerial) return
        stopTimer()
        current = null
        remainingMillis = null
        hovered = false
    }

    fun setHovered(expectedSerial: Long, value: Boolean) {
        if (current?.serial != expectedSerial || hovered == value) return
        stopTimer()
        hovered = value
        startTimer()
    }

    fun setVisible(value: Boolean) {
        if (visible == value) return
        stopTimer()
        visible = value
        startTimer()
    }

    private fun stopTimer() {
        timer?.cancel()
        timer = null
        startedAt?.let { start ->
            remainingMillis =
                remainingMillis?.let {
                    (it - (clockMillis() - start).coerceAtLeast(0L)).coerceAtLeast(0L)
                }
        }
        startedAt = null
    }

    private fun startTimer() {
        val entry = current ?: return
        val remaining = remainingMillis ?: return
        if (hovered || !visible) return
        startedAt = clockMillis()
        timer =
            scope.launch {
                delay(remaining)
                if (current?.serial == entry.serial) dismiss(entry.serial)
            }
    }

    fun close() {
        dismiss()
        closed = true
    }
}

/** Sonner's compact SVG arc flags are expanded for Compose's PathParser. */
internal fun nativeToastIconPath(kind: ToastKind): String? =
    when (kind) {
        ToastKind.Success ->
            "M10 18 a8 8 0 1 0 0 -16 a8 8 0 0 0 0 16 z m3.857 -9.809 a.75 .75 0 0 0 -1.214 -.882 l-3.483 4.79 -1.88 -1.88 a.75 .75 0 1 0 -1.06 1.061 l2.5 2.5 a.75 .75 0 0 0 1.137 -.089 l4 -5.5 z"
        ToastKind.Error ->
            "M18 10 a8 8 0 1 1 -16 0 a8 8 0 0 1 16 0 z m-8 -5 a.75 .75 0 0 1 .75 .75 v4.5 a.75 .75 0 0 1 -1.5 0 v-4.5 A.75 .75 0 0 1 10 5 z m0 10 a1 1 0 1 0 0 -2 a1 1 0 0 0 0 2 z"
        ToastKind.Default -> null
    }

/** Shared toast surface. The paths and spacing are from the installed Sonner assets. */
@Composable
fun NativeToast(
    message: String,
    kind: ToastKind = ToastKind.Default,
    modifier: Modifier = Modifier,
) {
    Row(
        modifier
            .widthIn(max = 356.dp)
            .fillMaxWidth()
            .toastShadow(RoundedCornerShape(8.dp))
            .border(1.dp, MaterialTheme.colorScheme.outline, RoundedCornerShape(8.dp))
            .background(MaterialTheme.colorScheme.surface, RoundedCornerShape(8.dp))
            .padding(17.dp)
            .semantics { liveRegion = LiveRegionMode.Polite },
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        if (kind != ToastKind.Default) {
            val path =
                remember(kind) {
                    PathParser().parsePathString(nativeToastIconPath(kind)!!).toPath().apply {
                        fillType = PathFillType.EvenOdd
                    }
                }
            val color = MaterialTheme.colorScheme.onSurface
            Box(Modifier.size(17.dp, 16.dp), contentAlignment = Alignment.CenterStart) {
                Canvas(Modifier.requiredSize(20.dp).offset(x = (-4).dp)) {
                    scale(size.width / 20f, size.height / 20f, pivot = Offset.Zero) {
                        drawPath(path, color)
                    }
                }
            }
        }
        val fonts = LocalFonts.current
        CompositionLocalProvider(LocalFonts provides fonts.copy(latin = FontFamily.SansSerif)) {
            Text(message, weight = FontWeight.Medium, size = 13.sp, lineHeight = 19.5.sp)
        }
    }
}

/**
 * Put this host above the application's content and overlays; it does not consume empty-space
 * input.
 */
@Composable
fun NativeToastHost(app: Session, modifier: Modifier = Modifier) {
    val entry = app.toastState ?: return
    val interaction = remember { MutableInteractionSource() }
    val hovered by interaction.collectIsHoveredAsState()
    var focused by remember { mutableStateOf(false) }
    LaunchedEffect(entry.serial, hovered, focused) {
        app.setToastHovered(entry.serial, hovered || focused)
    }
    DisposableEffect(entry.serial) { onDispose { app.setToastHovered(entry.serial, false) } }
    BoxWithConstraints(modifier.fillMaxSize().zIndex(999999f)) {
        val mobile = maxWidth <= 600.dp
        val offset = if (mobile) 16.dp else 24.dp
        NativeToast(
            entry.message,
            entry.kind,
            Modifier.align(Alignment.BottomEnd)
                .padding(start = if (mobile) offset else 0.dp, end = offset, bottom = offset)
                .width(if (mobile) (maxWidth - offset * 2).coerceAtLeast(0.dp) else 356.dp)
                .hoverable(interaction)
                .onFocusChanged { focused = it.isFocused }
                .focusable(interactionSource = interaction),
        )
    }
}
