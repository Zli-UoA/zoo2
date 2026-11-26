package zoo.android.ui

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.expandVertically
import androidx.compose.animation.shrinkVertically
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.input.nestedscroll.NestedScrollConnection
import androidx.compose.ui.input.nestedscroll.NestedScrollSource
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.dp

@Composable
internal fun CollapsingHeader(
    header: @Composable () -> Unit,
    content: @Composable ColumnScope.() -> Unit,
) {
    val threshold = with(LocalDensity.current) { 20.dp.toPx() }
    var visible by remember { mutableStateOf(true) }
    val scroll = remember(threshold) {
        object : NestedScrollConnection {
            private var distance = 0f
            override fun onPostScroll(consumed: Offset, available: Offset, source: NestedScrollSource): Offset {
                if (source == NestedScrollSource.UserInput) {
                    val delta = if (consumed.y == 0f) available.y.coerceAtLeast(0f) else consumed.y
                    if (delta != 0f) {
                        if (distance * delta < 0) distance = 0f
                        distance += delta
                        if (distance <= -threshold || distance >= threshold) {
                            visible = distance > 0
                            distance = 0f
                        }
                    }
                }
                return Offset.Zero
            }
        }
    }
    Column(Modifier.fillMaxSize().nestedScroll(scroll)) {
        AnimatedVisibility(
            visible = visible,
            enter = expandVertically(expandFrom = Alignment.Top),
            exit = shrinkVertically(shrinkTowards = Alignment.Top),
        ) { header() }
        content()
    }
}
