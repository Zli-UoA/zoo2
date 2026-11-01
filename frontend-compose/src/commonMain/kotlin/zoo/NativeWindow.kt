package zoo

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.BlurredEdgeTreatment
import androidx.compose.ui.draw.blur
import androidx.compose.ui.focus.*
import androidx.compose.ui.input.key.*
import androidx.compose.ui.platform.LocalWindowInfo
import androidx.compose.ui.unit.dp

/** One overlay/toast owner per scene, shared by the application and Storybook. */
@Composable
fun NativeWindow(
    app: Session,
    modifier: Modifier = Modifier,
    content: @Composable BoxScope.() -> Unit,
) {
    val modalOpen = app.overlays.any { it.anchor == null && !it.nonModal && !it.tooltip }
    val rootFocus = remember { FocusRequester() }
    var hasFocus by remember { mutableStateOf(false) }
    val windowFocused = LocalWindowInfo.current.isWindowFocused
    // Removing a focused control can leave the scene without a focused child. Recover
    // only while a modal owns the window, after its normal autofocus has had a frame.
    LaunchedEffect(modalOpen, hasFocus, windowFocused) {
        if (modalOpen && windowFocused && !hasFocus) {
            withFrameNanos {}
            if (!hasFocus) rootFocus.requestFocus()
        }
    }
    Box(
        modifier
            .fillMaxSize()
            .focusRequester(rootFocus)
            .onFocusChanged { hasFocus = it.hasFocus }
            .onPreviewKeyEvent {
                if (
                    it.type == KeyEventType.KeyDown &&
                        it.key == Key.Escape &&
                        app.overlays.any { slot -> !slot.tooltip }
                ) {
                    app.escape()
                    true
                } else false
            }
            .focusProperties { canFocus = modalOpen }
            .focusTarget()
    ) {
        Box(
            Modifier.fillMaxSize()
                .then(
                    if (modalOpen)
                        Modifier.blur(4.dp, edgeTreatment = BlurredEdgeTreatment.Unbounded)
                    else Modifier
                ),
            content = content,
        )
        OverlayHost(app)
        NativeToastHost(app)
    }
}
