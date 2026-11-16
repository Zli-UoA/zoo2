package zoo

import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectVerticalDragGestures
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.semantics.*
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.Dp

/** One editor moves between the channel and its overlay, preserving selection and IME state. */
@Composable
internal fun ChannelComposerSheet(
    app: Session,
    owner: String,
    enabled: Boolean,
    expanded: Boolean,
    changeExpanded: (Boolean) -> Unit,
    content: @Composable ColumnScope.(Boolean, Dp) -> Unit,
) {
    val latestContent = rememberUpdatedState(content)
    val editor = remember(owner) {
        movableContentOf<Boolean> { fullScreen ->
            BoxWithConstraints(if (fullScreen) Modifier.fillMaxSize() else Modifier.fillMaxWidth()) {
                Column(if (fullScreen) Modifier.fillMaxSize() else Modifier.fillMaxWidth()) {
                    latestContent.value(this, fullScreen, maxHeight)
                }
            }
        }
    }
    if (!LocalTouchUi.current) {
        editor(false)
        return
    }
    if (!expanded) {
        Column(Modifier.fillMaxWidth()) {
            ComposerDragHandle(app, expanded = false, enabled = enabled) { changeExpanded(true) }
            editor(false)
        }
    }
    OverlayRegistration(
        app.t("composer.editorTitle"), { changeExpanded(false) }, 576.dp,
        visible = expanded, fullScreen = true, customChrome = true,
    ) {
        if (expanded) {
            PaneHeader {
                IconButton("arrow-left", app.t("composer.collapse")) { changeExpanded(false) }
                Text(app.t("messages.compose"), Modifier.weight(1f),
                    size = AndroidDesign.titleSize, lineHeight = AndroidDesign.titleLineHeight)
            }
            ComposerDragHandle(app, expanded = true, enabled = true) { changeExpanded(false) }
            Box(Modifier.weight(1f).fillMaxWidth()) { editor(true) }
        }
    }
}

@Composable
private fun ComposerDragHandle(app: Session, expanded: Boolean, enabled: Boolean, change: () -> Unit) {
    val latestChange by rememberUpdatedState(change)
    val action = app.t(if (expanded) "composer.collapse" else "composer.expand")
    Box(
        Modifier.fillMaxWidth().height(AndroidDesign.sectionGap)
            .background(MaterialTheme.colorScheme.background)
            .semantics {
                testTag = "composer-drag-handle"
                contentDescription = app.t("composer.dragHandle")
                stateDescription = action
                if (enabled) customActions = listOf(CustomAccessibilityAction(action) { latestChange(); true })
            }
            .pointerInput(expanded, enabled) {
                var distance = 0f
                detectVerticalDragGestures(
                    onDragStart = { distance = 0f },
                    onDragCancel = { distance = 0f },
                    onDragEnd = {
                        val threshold = 24.dp.toPx()
                        if (enabled && if (expanded) distance >= threshold else distance <= -threshold) latestChange()
                    },
                    onVerticalDrag = { _, delta -> distance += delta },
                )
            },
        contentAlignment = Alignment.Center,
    ) {
        Box(Modifier.size(32.dp, AndroidDesign.spaceSmall)
            .background(MaterialTheme.colorScheme.onSurfaceVariant, RoundedCornerShape(2.dp)))
    }
}
