package zoo.android

import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.*
import androidx.compose.ui.platform.LocalHapticFeedback

/** Observe successful touch clicks after children; drags/cancelled presses never vibrate. */
@Composable
internal fun Modifier.operationHaptics(): Modifier {
    val feedback = LocalHapticFeedback.current
    return pointerInput(feedback) {
        awaitEachGesture {
            val down = awaitFirstDown(false, PointerEventPass.Initial)
            if (down.type != PointerType.Touch) return@awaitEachGesture
            val origin = down.position
            var moved = false
            while (true) {
                val event = awaitPointerEvent(PointerEventPass.Final)
                val change = event.changes.find { it.id == down.id } ?: break
                if ((change.position - origin).getDistance() > viewConfiguration.touchSlop ||
                    event.changes.count { it.pressed } > 1) moved = true
                if (!change.pressed) {
                    if (!moved && change.isConsumed &&
                        change.uptimeMillis - down.uptimeMillis < viewConfiguration.longPressTimeoutMillis)
                        feedback.performHapticFeedback(HapticFeedbackType.TextHandleMove)
                    break
                }
            }
        }
    }
}
