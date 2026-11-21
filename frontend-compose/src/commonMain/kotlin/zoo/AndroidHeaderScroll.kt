package zoo

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.expandVertically
import androidx.compose.animation.shrinkVertically
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.input.nestedscroll.NestedScrollConnection
import androidx.compose.ui.input.nestedscroll.NestedScrollSource
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.dp
import kotlin.math.abs

internal class AndroidHeaderScroll(private val threshold: Float) : NestedScrollConnection {
    var visible by mutableStateOf(true)
        private set
    private var distance = 0f

    override fun onPostScroll(consumed: Offset, available: Offset, source: NestedScrollSource): Offset {
        if (source != NestedScrollSource.UserInput) return Offset.Zero
        // A drag at the start of the list should also bring the header back.
        val vertical = if (consumed.y == 0f) available.y.coerceAtLeast(0f) else consumed.y
        if (vertical == 0f || abs(vertical) <= abs(consumed.x)) return Offset.Zero
        if ((distance > 0f) != (vertical > 0f)) distance = 0f
        distance += vertical
        if (abs(distance) >= threshold) {
            visible = distance > 0f
            distance = 0f
        }
        return Offset.Zero
    }
}

@Composable
internal fun rememberAndroidHeaderScroll(key: Any?, enabled: Boolean): AndroidHeaderScroll? {
    val threshold = with(LocalDensity.current) { 16.dp.toPx() }
    return remember(key, enabled, threshold) { if (enabled) AndroidHeaderScroll(threshold) else null }
}

internal fun Modifier.withAndroidHeaderScroll(state: AndroidHeaderScroll?): Modifier =
    if (state == null) this else nestedScroll(state)

@Composable
internal fun AndroidScrollingHeader(state: AndroidHeaderScroll?, content: @Composable () -> Unit) {
    if (state == null) content()
    else AnimatedVisibility(state.visible, enter = expandVertically(), exit = shrinkVertically()) { content() }
}
