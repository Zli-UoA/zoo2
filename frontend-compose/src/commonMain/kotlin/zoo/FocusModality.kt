package zoo

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

/** Match browser :focus-visible across controls, including Tab arriving from a text field. */
internal object FocusInputModality {
    var keyboard by mutableStateOf(true)
}

fun Modifier.focusInputModality(): Modifier =
    onPreviewKeyEvent {
            if (it.type == KeyEventType.KeyDown) FocusInputModality.keyboard = true
            false
        }
        .pointerInput(Unit) {
            awaitPointerEventScope {
                while (true) {
                    val event = awaitPointerEvent(PointerEventPass.Initial)
                    if (event.changes.any { it.pressed && !it.previousPressed })
                        FocusInputModality.keyboard = false
                }
            }
        }

internal fun Modifier.focusVisibleRing(
    radius: Dp,
    color: Color,
    visible: Boolean,
    width: Dp = 3.dp,
    preferredWidth: Float = Float.NaN,
): Modifier =
    if (!visible) this
    else
        drawBehind {
            val ring = width.toPx()
            val half = ring / 2
            drawRoundRect(
                color,
                Offset(-half, -half),
                Size(cssPaintWidth(size.width, preferredWidth) + ring, size.height + ring),
                CornerRadius(radius.toPx() + half),
                style = Stroke(ring),
            )
        }

/** Preserve CSS intrinsic fractional widths while respecting stretched or constrained controls. */
internal fun cssPaintWidth(measured: Float, preferred: Float): Float =
    if (preferred.isFinite() && kotlin.math.ceil(preferred) == measured) preferred else measured
