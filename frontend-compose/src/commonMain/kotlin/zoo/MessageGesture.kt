package zoo

import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.awaitHorizontalTouchSlopOrCancellation
import androidx.compose.foundation.gestures.horizontalDrag
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.pointer.*
import androidx.compose.ui.platform.ViewConfiguration
import androidx.compose.ui.unit.dp

/** Links keep their platform long-press behavior instead of opening the enclosing message menu. */
private val holdExcludedPointers = mutableSetOf<PointerId>()

internal fun Modifier.preventMessageHold(
    excludeAt: (androidx.compose.ui.geometry.Offset) -> Boolean = { true }
): Modifier =
    pointerInput(Unit) {
        awaitEachGesture {
            val down = awaitFirstDown(requireUnconsumed = false, pass = PointerEventPass.Initial)
            if (down.type == PointerType.Touch && excludeAt(down.position)) {
                holdExcludedPointers += down.id
                try {
                    do {
                        val event = awaitPointerEvent(PointerEventPass.Final)
                    } while (event.changes.any { it.id == down.id && it.pressed })
                } finally {
                    holdExcludedPointers -= down.id
                }
            }
        }
    }

/** A message context gesture owns touch long-press; text selection keeps every other gesture. */
internal class MessageGestureArbitration {
    var pointer: PointerId? = null

    fun selectionConfiguration(base: ViewConfiguration): ViewConfiguration =
        object : ViewConfiguration by base {
            override val longPressTimeoutMillis: Long
                get() =
                    if (pointer?.let { it !in holdExcludedPointers } == true) Int.MAX_VALUE.toLong()
                    else base.longPressTimeoutMillis
        }
}

internal fun Modifier.messageHold(
    enabled: Boolean,
    arbitration: MessageGestureArbitration,
    open: () -> Unit,
): Modifier =
    pointerInput(enabled, arbitration) {
        awaitEachGesture {
            val down = awaitFirstDown(requireUnconsumed = false, pass = PointerEventPass.Initial)
            if (!enabled || down.type != PointerType.Touch) return@awaitEachGesture
            // Initial runs before the child's selection detector starts its Main-pass
            // timer. Link exclusions are registered by the child's Initial pass.
            arbitration.pointer = down.id
            try {
                awaitPointerEvent(PointerEventPass.Final)
                if (down.isConsumed || down.id in holdExcludedPointers) return@awaitEachGesture
                val cancelled =
                    withTimeoutOrNull(500L) {
                        while (true) {
                            val event = awaitPointerEvent(PointerEventPass.Final)
                            val pointer = event.changes.find { it.id == down.id }
                            if (
                                pointer == null ||
                                    !pointer.pressed ||
                                    pointer.isConsumed ||
                                    (pointer.position - down.position).getDistance() >
                                        10.dp.toPx() ||
                                    event.changes.count { it.pressed } > 1 ||
                                    down.id in holdExcludedPointers
                            )
                                return@withTimeoutOrNull true
                        }
                    }
                if (cancelled == null) {
                    open()
                    do {
                        val event = awaitPointerEvent(PointerEventPass.Final)
                        event.changes.filter { it.id == down.id }.forEach { it.consume() }
                    } while (event.changes.any { it.id == down.id && it.pressed })
                }
            } finally {
                arbitration.pointer = null
            }
        }
    }

/** Left drag follows the finger. Exclude both system back edges and child gestures. */
internal fun Modifier.messageSwipe(
    onOffset: (Float) -> Unit,
    onThreshold: () -> Unit,
    onCommit: () -> Unit,
): Modifier = pointerInput(Unit) {
    val edge = 24.dp.toPx()
    val threshold = 64.dp.toPx()
    awaitEachGesture {
        val down = awaitFirstDown(requireUnconsumed = false)
        if (down.type != PointerType.Touch || down.position.x !in edge..(size.width - edge))
            return@awaitEachGesture
        var distance = 0f
        var signalled = false
        fun shiftBy(delta: Float) {
            distance = (distance - delta).coerceIn(0f, threshold * 1.25f)
            onOffset(-distance)
            if (!signalled && distance >= threshold) {
                signalled = true
                onThreshold()
            }
        }
        try {
            val start = awaitHorizontalTouchSlopOrCancellation(down.id) { change, overSlop ->
                if (overSlop < 0f && down.id !in holdExcludedPointers) {
                    change.consume()
                    shiftBy(overSlop)
                }
            } ?: return@awaitEachGesture
            if (horizontalDrag(start.id) { change ->
                shiftBy(change.positionChange().x)
                change.consume()
            } && distance >= threshold) onCommit()
        } finally {
            onOffset(0f)
        }
    }
}
