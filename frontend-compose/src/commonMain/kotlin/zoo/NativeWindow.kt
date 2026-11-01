package zoo

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.BlurredEdgeTreatment
import androidx.compose.ui.draw.blur
import androidx.compose.ui.unit.dp

/** One overlay/toast owner per scene, shared by the application and Storybook. */
@Composable
fun NativeWindow(
    app: Session,
    modifier: Modifier = Modifier,
    content: @Composable BoxScope.() -> Unit,
) {
    val modalOpen = app.overlays.any { it.anchor == null && !it.nonModal && !it.tooltip }
    Box(modifier.fillMaxSize()) {
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
