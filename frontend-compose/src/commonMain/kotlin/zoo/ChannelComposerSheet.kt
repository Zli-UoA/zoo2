package zoo

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectVerticalDragGestures
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.*
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.Dp

/** Channel and fullscreen presentations share the caller-owned editing and attachment state. */
@Composable
internal fun ChannelComposerSheet(
    app: Session,
    enabled: Boolean,
    expanded: Boolean,
    changeExpanded: (Boolean) -> Unit,
    content: @Composable ColumnScope.(Boolean, Dp) -> Unit,
) {
    val editor: @Composable (Boolean) -> Unit = { fullScreen ->
        BoxWithConstraints(if (fullScreen) Modifier.fillMaxSize() else Modifier.fillMaxWidth()) {
            val availableHeight = maxHeight
            Column(if (fullScreen) Modifier.fillMaxSize() else Modifier.fillMaxWidth()) {
                content(this, fullScreen, availableHeight)
            }
        }
    }
    if (!LocalTouchUi.current) {
        editor(false)
        return
    }
    var drag by remember(expanded) { mutableFloatStateOf(0f) }
    val density = LocalDensity.current
    if (!expanded) {
        val stretch = with(density) { (-drag).coerceAtLeast(0f).toDp().coerceAtMost(144.dp) }
        Column(Modifier.fillMaxWidth().semantics { testTag = "channel-composer-dock" }
            .background(MaterialTheme.colorScheme.surface)) {
            androidx.compose.material3.HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
            ComposerDragHandle(app, expanded = false, enabled = enabled, onDrag = { drag = it }) {
                changeExpanded(true)
            }
            Spacer(Modifier.height(stretch))
            editor(false)
        }
    }
    OverlayRegistration(
        app.t("composer.editorTitle"), { changeExpanded(false) }, 576.dp,
        visible = expanded, fullScreen = true, customChrome = true,
    ) {
        if (expanded) {
            val displacement = with(density) { drag.coerceAtLeast(0f).toDp().coerceAtMost(144.dp) }
            Column(Modifier.fillMaxSize().offset(y = displacement).background(MaterialTheme.colorScheme.background)) {
                PaneHeader {
                    IconButton("arrow-left", app.t("composer.collapse")) { changeExpanded(false) }
                }
                ComposerDragHandle(app, expanded = true, enabled = true, onDrag = { drag = it }) {
                    changeExpanded(false)
                }
                Box(Modifier.weight(1f).fillMaxWidth()) { editor(true) }
            }
        }
    }
}

@Composable
private fun ComposerDragHandle(
    app: Session, expanded: Boolean, enabled: Boolean, onDrag: (Float) -> Unit, change: () -> Unit,
) {
    val latestChange by rememberUpdatedState(change)
    val latestDrag by rememberUpdatedState(onDrag)
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
            .clickable(enabled = enabled, onClickLabel = action, onClick = { latestChange() })
            .pointerInput(expanded, enabled) {
                if (!enabled) return@pointerInput
                var distance = 0f
                detectVerticalDragGestures(
                    onDragStart = { distance = 0f },
                    onDragCancel = { distance = 0f; latestDrag(0f) },
                    onDragEnd = {
                        val threshold = AndroidDesign.sectionGap.toPx()
                        if (if (expanded) distance >= threshold else distance <= -threshold) latestChange()
                        latestDrag(0f)
                    },
                    onVerticalDrag = { _, delta ->
                        distance += delta
                        latestDrag(distance)
                    },
                )
            },
        contentAlignment = Alignment.Center,
    ) {
        Box(Modifier.size(32.dp, AndroidDesign.spaceSmall)
            .background(MaterialTheme.colorScheme.onSurfaceVariant, RoundedCornerShape(2.dp)))
    }
}
