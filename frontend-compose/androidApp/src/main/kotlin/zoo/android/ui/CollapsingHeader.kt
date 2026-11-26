package zoo.android.ui

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.input.nestedscroll.NestedScrollConnection
import androidx.compose.ui.input.nestedscroll.NestedScrollSource
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

@Composable
internal fun CollapsingHeader(
    header: @Composable () -> Unit,
    forceVisible: Boolean = false,
    content: @Composable ColumnScope.(Dp, Boolean) -> Unit,
) {
    val density = LocalDensity.current
    val threshold = with(density) { 20.dp.toPx() }
    var headerHeight by rememberSaveable { mutableIntStateOf(0) }
    var visible by rememberSaveable { mutableStateOf(true) }
    LaunchedEffect(forceVisible) { if (forceVisible) visible = true }
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
    val shown = visible || forceVisible
    Box(Modifier.fillMaxSize().clipToBounds().nestedScroll(scroll)) {
        Column(Modifier.fillMaxSize()) {
            content(with(density) { headerHeight.toDp() }, shown)
        }
        AnimatedVisibility(
            visible = shown,
            enter = slideInVertically { -it },
            exit = slideOutVertically { -it },
        ) {
            Box(Modifier.onSizeChanged { headerHeight = it.height }
                .pointerInput(Unit) { detectTapGestures {} }) { header() }
        }
    }
}
