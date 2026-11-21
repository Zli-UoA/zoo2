package zoo

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp

@Composable
internal fun AndroidThreadLayout(
    title: String,
    backLabel: String,
    replyLabel: String,
    modifier: Modifier,
    back: () -> Unit,
    parent: @Composable () -> Unit,
    actions: @Composable () -> Unit,
    replies: @Composable ColumnScope.() -> Unit,
    compose: (@Composable () -> Unit)?,
) {
    val colors = MaterialTheme.colorScheme
    Column(modifier.fillMaxHeight().background(colors.background)) {
        PaneHeader {
            IconButton("arrow-left", backLabel, click = back)
            Text(title, Modifier.weight(1f).semantics { heading() },
                weight = FontWeight.SemiBold, size = AndroidDesign.titleSize)
        }
        Box(
            Modifier.fillMaxWidth().heightIn(max = 180.dp).verticalScroll(rememberScrollState())
                .padding(horizontal = AndroidDesign.screenInset),
        ) { parent() }
        HorizontalDivider(Modifier.padding(horizontal = AndroidDesign.screenInset), color = colors.outlineVariant)
        Text(
            replyLabel,
            Modifier.padding(horizontal = AndroidDesign.screenInset, vertical = AndroidDesign.spaceMedium),
            small = true, muted = true,
        )
        actions()
        Column(Modifier.weight(1f), content = replies)
        compose?.invoke()
    }
}
