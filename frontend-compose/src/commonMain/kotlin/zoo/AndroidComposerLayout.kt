package zoo

import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

/** Layout only. The caller owns the editor, draft, formatting and send behavior. */
@Composable
internal fun ColumnScope.AndroidComposerLayout(
    fullScreen: Boolean,
    scrollEditor: Boolean,
    showTools: Boolean,
    showFormatting: Boolean,
    editor: @Composable (Modifier, Dp?) -> Unit,
    attach: @Composable () -> Unit,
    send: @Composable () -> Unit,
    tools: @Composable RowScope.() -> Unit,
    formatting: @Composable RowScope.() -> Unit,
) {
    val fillEditor = fullScreen && !scrollEditor
    Column(Modifier.fillMaxWidth().background(MaterialTheme.colorScheme.background)
        .then(if (fillEditor) Modifier.weight(1f) else Modifier)) {
        Row(
            Modifier.fillMaxWidth().then(if (fillEditor) Modifier.weight(1f) else Modifier),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            if (!showTools) attach()
            BoxWithConstraints(
                Modifier.weight(1f).then(if (fillEditor) Modifier.fillMaxHeight() else Modifier),
            ) {
                val editorHeight = when {
                    fillEditor -> maxHeight.coerceAtLeast(AndroidDesign.touchTarget)
                    scrollEditor -> 96.dp
                    else -> null
                }
                editor(
                    Modifier.fillMaxWidth().then(if (fillEditor) Modifier.fillMaxHeight() else Modifier),
                    editorHeight,
                )
            }
            if (!showTools) send()
        }
        if (showFormatting) Row(
            Modifier.fillMaxWidth().padding(horizontal = AndroidDesign.spaceMedium)
                .horizontalScroll(rememberScrollState()),
            horizontalArrangement = Arrangement.spacedBy(AndroidDesign.spaceSmall),
            content = formatting,
        )
        if (showTools) Row(
            Modifier.fillMaxWidth().padding(horizontal = AndroidDesign.spaceMedium),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            attach()
            tools()
            Spacer(Modifier.weight(1f))
            send()
        }
    }
}
